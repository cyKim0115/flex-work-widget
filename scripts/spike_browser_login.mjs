#!/usr/bin/env node
// Phase 0 스파이크: 위젯 전용 Chrome 프로필로 flex.team 로그인을 받고, CDP로 쿠키 메타데이터만 확인한다.
// 쿠키 값·이메일은 절대 출력하지 않는다. 로그인 입력은 사용자가 창에서 직접 한다.
//
//   node scripts/spike_browser_login.mjs            # 창 + 디버깅 포트(기본 흐름)
//   node scripts/spike_browser_login.mjs fallback   # 포트 없이 창 → 사용자가 창을 닫음 → headless로 쿠키 읽기
//   node scripts/spike_browser_login.mjs headless   # 4단계(무인 갱신)만 다시 확인
//   node scripts/spike_browser_login.mjs refresh-sim  # 인증 쿠키 판별 + V2_WS_AID 재발급 확인(실패 시 원복)
//
// 결과 요약은 --out <file> 로 JSON 저장(값 없음).

import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const LOCAL = process.env.LOCALAPPDATA ?? ".";
const PROFILE = path.join(LOCAL, "FlexWorkWidget", "login-profile");
const LOGIN_URL = "https://flex.team/";
const HOME_URL = "https://flex.team/home";
const ME_URL = "https://flex.team/api/v2/workspace/users/me/workspace-users";
const TRACKED = ["AID", "V2_WS_AID", "V2_WS_RID"];
const LOGIN_TIMEOUT_MS = Number(process.env.SPIKE_LOGIN_TIMEOUT_MS ?? 10 * 60_000);

const args = process.argv.slice(2);
const mode = args[0] && !args[0].startsWith("--") ? args[0] : "port";
const outIdx = args.indexOf("--out");
const outFile = outIdx >= 0 ? args[outIdx + 1] : null;

const report = { mode, startedAt: new Date().toISOString(), steps: {} };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const log = (msg) => console.log(`[${new Date().toLocaleTimeString()}] ${msg}`);

function regDefault(key) {
  try {
    const out = execFileSync("reg", ["query", key, "/ve"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
    const m = out.match(/REG_(?:EXPAND_)?SZ\s+(.+)/);
    return m ? m[1].trim().replace(/^"|"$/g, "") : null;
  } catch {
    return null;
  }
}

function findBrowser() {
  const env = process.env.FLEX_BROWSER_PATH;
  if (env && fs.existsSync(env)) return { path: env, source: "env" };
  for (const exe of ["chrome.exe", "msedge.exe"]) {
    for (const hive of ["HKCU", "HKLM"]) {
      const p = regDefault(`${hive}\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\${exe}`);
      if (p && fs.existsSync(p)) return { path: p, source: `${hive} App Paths` };
    }
    const roots = [process.env.ProgramFiles, process.env["ProgramFiles(x86)"], LOCAL].filter(Boolean);
    const rel = exe === "chrome.exe" ? "Google\\Chrome\\Application\\chrome.exe" : "Microsoft\\Edge\\Application\\msedge.exe";
    for (const root of roots) {
      const p = path.join(root, rel);
      if (fs.existsSync(p)) return { path: p, source: "well-known" };
    }
  }
  throw new Error("Chrome/Edge 실행 파일을 찾지 못했습니다");
}

function profileInUse() {
  const lock = path.join(PROFILE, "lockfile");
  if (!fs.existsSync(lock)) return false;
  try {
    fs.rmSync(lock);
    return false;
  } catch {
    return true;
  }
}

async function waitPortFile(timeoutMs) {
  const file = path.join(PROFILE, "DevToolsActivePort");
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try {
      const [port, wsPath] = fs.readFileSync(file, "utf8").split(/\r?\n/);
      if (port && wsPath) return { port: Number(port), wsPath, ms: Date.now() - t0 };
    } catch {}
    await sleep(100);
  }
  throw new Error(`DevToolsActivePort ${timeoutMs}ms 안에 생기지 않음`);
}

class Cdp {
  constructor(url) {
    this.ws = new WebSocket(url);
    this.id = 0;
    this.pending = new Map();
    this.closed = false;
    this.ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(ev.data);
      const p = this.pending.get(msg.id);
      if (!p) return;
      this.pending.delete(msg.id);
      msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result);
    });
    this.ws.addEventListener("close", () => {
      this.closed = true;
      for (const p of this.pending.values()) p.reject(new Error("websocket closed"));
      this.pending.clear();
    });
  }
  open() {
    return new Promise((resolve, reject) => {
      this.ws.addEventListener("open", resolve, { once: true });
      this.ws.addEventListener("error", () => reject(new Error("websocket error")), { once: true });
    });
  }
  send(method, params = {}) {
    if (this.closed) return Promise.reject(new Error("websocket closed"));
    const id = ++this.id;
    this.ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }
}

const isFlex = (c) => c.domain === "flex.team" || c.domain.endsWith(".flex.team");
const meta = (c) => ({
  name: c.name,
  domain: c.domain,
  session: c.session,
  expires: c.session ? null : new Date(c.expires * 1000).toISOString(),
  httpOnly: c.httpOnly,
  secure: c.secure,
  len: c.value.length,
});

async function flexCookies(cdp) {
  const { cookies } = await cdp.send("Storage.getCookies");
  return cookies.filter(isFlex);
}

function hasRequired(cookies) {
  return cookies.some((c) => (c.name === "AID" || c.name === "V2_WS_AID") && c.value);
}

// work.rs 와 같은 헤더로 평문 HTTP 호출. 상태 코드·키 이름·userIdHash 존재 여부만 남긴다.
async function validate(cookies) {
  const v = (n) => cookies.find((c) => c.name === n)?.value ?? "";
  const aid = v("AID") || v("V2_WS_AID");
  const res = await fetch(ME_URL, {
    redirect: "manual",
    headers: {
      Origin: "https://flex.team",
      Referer: "https://flex.team/home",
      Accept: "application/json",
      "User-Agent": "flex-work-widget/0.1",
      Cookie: `AID=${v("AID")}; V2_WS_AID=${v("V2_WS_AID")}; V2_WS_RID=${v("V2_WS_RID")}`,
      "x-flex-aid": aid,
      "FlexTeam-Version": "V2",
    },
  });
  const out = { status: res.status, contentType: res.headers.get("content-type") };
  if (res.status === 200) {
    try {
      const body = await res.json();
      out.topKeys = Array.isArray(body) ? ["<array>"] : Object.keys(body);
      out.hasUserIdHash = JSON.stringify(body).includes('"userIdHash"');
    } catch {
      out.parse = "not json";
    }
  }
  return out;
}

function launch(browser, extra) {
  try {
    fs.rmSync(path.join(PROFILE, "DevToolsActivePort"), { force: true });
  } catch {}
  const child = spawn(
    browser.path,
    [`--user-data-dir=${PROFILE}`, "--no-first-run", "--no-default-browser-check", ...extra],
    { stdio: "ignore", windowsHide: false },
  );
  const exited = new Promise((r) => child.on("exit", (code) => r(code)));
  return { child, exited };
}

async function connect() {
  const pf = await waitPortFile(15_000);
  const cdp = new Cdp(`ws://127.0.0.1:${pf.port}${pf.wsPath}`);
  await cdp.open();
  return { cdp, portFileMs: pf.ms };
}

function chromeVersion() {
  try {
    return fs.readFileSync(path.join(PROFILE, "Last Version"), "utf8").trim();
  } catch {
    return null;
  }
}

async function headedWithPort(browser) {
  const { exited } = launch(browser, ["--remote-debugging-port=0", "--new-window", LOGIN_URL]);
  let processExited = false;
  exited.then(() => (processExited = true));
  const { cdp, portFileMs } = await connect();
  const ver = await cdp.send("Browser.getVersion");
  report.steps.launch = { ok: true, portFileMs, product: ver.product, headlessUa: /HeadlessChrome/.test(ver.userAgent) };
  log(`창을 띄웠습니다 (${ver.product}). 창에서 flex.team에 직접 로그인하세요. 제한 ${LOGIN_TIMEOUT_MS / 60000}분.`);

  const t0 = Date.now();
  let lastNote = 0;
  let lastValidate = null;
  while (Date.now() - t0 < LOGIN_TIMEOUT_MS) {
    if (cdp.closed || processExited) {
      report.steps.login = { ok: false, reason: "창이 닫혔습니다", lastValidate };
      log("창이 닫혔습니다.");
      return false;
    }
    let cookies;
    try {
      cookies = await flexCookies(cdp);
    } catch {
      await sleep(1500);
      continue;
    }
    if (hasRequired(cookies)) {
      const result = await validate(cookies).catch((e) => ({ error: String(e) }));
      if (JSON.stringify(result) !== JSON.stringify(lastValidate)) log(`validate: ${JSON.stringify(result)}`);
      lastValidate = result;
      if (result.status === 200 && result.hasUserIdHash) {
        report.steps.cookies = cookies.filter((c) => TRACKED.includes(c.name)).map(meta);
        report.steps.allFlexCookieNames = cookies.map((c) => c.name);
        report.steps.login = { ok: true, afterMs: Date.now() - t0 };
        report.steps.validate = result;
        log(`로그인 확인 (${Math.round((Date.now() - t0) / 1000)}s). 쿠키 메타: ${JSON.stringify(report.steps.cookies)}`);
        await cdp.send("Browser.close").catch(() => {});
        await Promise.race([exited, sleep(10_000)]);
        report.steps.closedProcess = processExited;
        return true;
      }
    }
    if (Date.now() - lastNote > 15_000) {
      lastNote = Date.now();
      log(`로그인 대기 중… flex 쿠키 ${cookies.length}개, 필수 쿠키 ${hasRequired(cookies) ? "있음" : "없음"}`);
    }
    await sleep(1500);
  }
  report.steps.login = { ok: false, reason: "timeout", lastValidate };
  await cdp.send("Browser.close").catch(() => {});
  return false;
}

async function headedFallback(browser) {
  const { exited } = launch(browser, ["--new-window", LOGIN_URL]);
  log("포트 없이 창을 띄웠습니다. 로그인한 뒤 창을 직접 닫으세요.");
  const code = await Promise.race([exited, sleep(LOGIN_TIMEOUT_MS).then(() => "timeout")]);
  report.steps.fallbackWindow = { exitCode: code };
  return code !== "timeout";
}

async function headlessCheck(browser) {
  const version = chromeVersion();
  const major = version?.split(".")[0];
  const ua = `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major ?? "120"}.0.0.0 Safari/537.36`;
  report.steps.headlessUa = { lastVersionFile: version, major };
  const { exited } = launch(browser, ["--headless=new", "--remote-debugging-port=0", `--user-agent=${ua}`, HOME_URL]);
  const { cdp, portFileMs } = await connect();
  const ver = await cdp.send("Browser.getVersion");
  const first = await flexCookies(cdp);
  const firstMeta = first.filter((c) => TRACKED.includes(c.name)).map(meta);
  log(`headless 첫 읽기 (${portFileMs}ms, UA에 HeadlessChrome ${/HeadlessChrome/.test(ver.userAgent) ? "있음" : "없음"}): ${JSON.stringify(firstMeta)}`);

  // 홈 페이지가 로드되며 쿠키를 갱신하는지 최대 20초 본다.
  let last = first;
  const t0 = Date.now();
  while (Date.now() - t0 < 20_000) {
    await sleep(2000);
    last = await flexCookies(cdp).catch(() => last);
  }
  const lastMeta = last.filter((c) => TRACKED.includes(c.name)).map(meta);
  const result = hasRequired(last) ? await validate(last).catch((e) => ({ error: String(e) })) : { skipped: "no required cookie" };
  const changed = TRACKED.map((n) => {
    const a = first.find((c) => c.name === n);
    const b = last.find((c) => c.name === n);
    return { name: n, valueChanged: !!a && !!b && a.value !== b.value, expiresBefore: a ? meta(a).expires : null, expiresAfter: b ? meta(b).expires : null };
  });
  report.steps.headless = { portFileMs, firstMeta, lastMeta, changed, validate: result };
  log(`headless 20초 후: ${JSON.stringify(changed)} validate=${JSON.stringify(result)}`);
  await cdp.send("Browser.close").catch(() => {});
  await Promise.race([exited, sleep(10_000)]);
}

// 어떤 쿠키가 인증에 실제로 쓰이는지, V2_WS_AID(12시간)가 없을 때 /home 이 AID(7일)로 다시 발급하는지 본다.
// 재발급이 안 되면 원래 쿠키를 되돌려 놓는다.
async function refreshSim(browser) {
  const { exited } = launch(browser, ["--headless=new", "--remote-debugging-port=0", `--user-agent=${regularUa()}`]);
  const { cdp } = await connect();
  const before = await flexCookies(cdp);
  const v = (n) => before.find((c) => c.name === n)?.value ?? "";
  const probe = async (label, cookie, xAid) => {
    const headers = { Origin: "https://flex.team", Referer: "https://flex.team/home", Accept: "application/json", "FlexTeam-Version": "V2" };
    if (cookie) headers.Cookie = cookie;
    if (xAid) headers["x-flex-aid"] = xAid;
    const res = await fetch(ME_URL, { redirect: "manual", headers });
    return { label, status: res.status };
  };
  const variants = [
    await probe("AID cookie + x-flex-aid=AID", `AID=${v("AID")}`, v("AID")),
    await probe("AID cookie only", `AID=${v("AID")}`, null),
    await probe("V2_WS_AID cookie + x-flex-aid=V2_WS_AID", `V2_WS_AID=${v("V2_WS_AID")}`, v("V2_WS_AID")),
    await probe("x-flex-aid=AID only", null, v("AID")),
  ];
  log(`header variants: ${JSON.stringify(variants)}`);

  const ws = before.find((c) => c.name === "V2_WS_AID");
  let reissued = null;
  if (ws) {
    await cdp.send("Storage.setCookies", { cookies: [{ name: ws.name, value: "", domain: ws.domain, path: ws.path, expires: 1 }] });
    const gone = !(await flexCookies(cdp)).some((c) => c.name === "V2_WS_AID");
    await cdp.send("Target.createTarget", { url: HOME_URL });
    const t0 = Date.now();
    while (Date.now() - t0 < 25_000 && !reissued) {
      await sleep(1000);
      const now = await flexCookies(cdp).catch(() => []);
      const c = now.find((x) => x.name === "V2_WS_AID" && x.value);
      if (c) reissued = { afterMs: Date.now() - t0, valueChanged: c.value !== ws.value, ...meta(c) };
    }
    log(`V2_WS_AID 제거(${gone ? "확인" : "실패"}) 후 /home 재발급: ${JSON.stringify(reissued)}`);
    if (!reissued) {
      const { name, value, domain, path: p, expires, httpOnly, secure, sameSite } = ws;
      await cdp.send("Storage.setCookies", { cookies: [{ name, value, domain, path: p, expires, httpOnly, secure, sameSite }] });
      log("재발급이 없어 원래 V2_WS_AID를 되돌렸습니다.");
    }
  }
  const after = await flexCookies(cdp);
  const result = hasRequired(after) ? await validate(after).catch((e) => ({ error: String(e) })) : { skipped: "no required cookie" };
  report.steps.refreshSim = { variants, reissued, after: after.filter((c) => TRACKED.includes(c.name)).map(meta), validate: result };
  log(`최종 validate=${JSON.stringify(result)}`);
  await cdp.send("Browser.close").catch(() => {});
  await Promise.race([exited, sleep(10_000)]);
}

function regularUa() {
  const major = chromeVersion()?.split(".")[0] ?? "120";
  return `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${major}.0.0.0 Safari/537.36`;
}

async function main() {
  fs.mkdirSync(PROFILE, { recursive: true });
  const browser = findBrowser();
  report.browser = { source: browser.source, file: path.basename(browser.path) };
  log(`브라우저: ${browser.path} (${browser.source}), 프로필: ${PROFILE}`);
  if (profileInUse()) throw new Error("같은 프로필 Chrome이 이미 떠 있습니다. 그 창을 닫고 다시 실행하세요.");

  let ok = true;
  if (mode === "refresh-sim") {
    await refreshSim(browser);
    if (outFile) fs.writeFileSync(outFile, JSON.stringify(report, null, 2));
    return;
  }
  if (mode === "port") ok = await headedWithPort(browser);
  else if (mode === "fallback") ok = await headedFallback(browser);
  if (ok) {
    await sleep(1500);
    await headlessCheck(browser);
  }
  report.finishedAt = new Date().toISOString();
  if (outFile) fs.writeFileSync(outFile, JSON.stringify(report, null, 2));
  log(`끝. ${ok ? "로그인 단계 성공" : "로그인 단계 실패"}`);
}

main().catch((e) => {
  report.error = String(e?.message ?? e);
  if (outFile) fs.writeFileSync(outFile, JSON.stringify(report, null, 2));
  console.error(`실패: ${report.error}`);
  process.exit(1);
});
