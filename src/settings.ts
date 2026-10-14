import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  applyTheme,
  loadPreferences,
  savePreferences,
  type DisplayMode,
  type Preferences,
  type ThemeMode,
} from "./preferences";
import { phaseLabel, tidyMessage, type WorkPhase, type WorkSnapshot } from "./work-types";

/// 제목 표시줄과 화면 위아래 여유. 이만큼 빼야 창 아래 끝이 작업 표시줄에 걸리지 않는다.
const SCREEN_MARGIN = 80;
/// tauri.conf.json 의 minHeight 와 같다.
const MIN_HEIGHT = 520;

let prefs: Preferences = loadPreferences();
let autostartEnabled = false;
let isDevBuild = false;

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

function showBootError(message: string) {
  document.body.innerHTML = `
    <div class="settings-window" style="padding:24px">
      <h1>설정 로드 실패</h1>
      <p class="settings-hint settings-hint--error">${message}</p>
      <button id="btn-error-close" class="btn secondary wide" type="button">닫기</button>
    </div>`;
  document.getElementById("btn-error-close")?.addEventListener("click", () => {
    void closeWindow();
  });
}

function contentHeight(): number {
  const el = document.querySelector(".settings-window");
  return el ? Math.ceil(el.getBoundingClientRect().height) : 0;
}

/// 화면에 들어가는 만큼만 내용 높이에 창을 맞추고 가운데로 옮긴다. 내용이 더 길면
/// 스크롤로 넘기고 닫기 버튼은 아래에 붙어 있다. 고정 높이로 두면 FHD처럼 세로가
/// 짧은 화면에서 창 아래가 화면 밖으로 나가, 늘리지도 닫지도 못하게 된다.
async function fitToScreen() {
  try {
    const win = getCurrentWindow();
    const inner = (await win.innerSize()).toLogical(await win.scaleFactor());
    const maxHeight = Math.max(MIN_HEIGHT, window.screen.availHeight - SCREEN_MARGIN);
    const target = Math.max(MIN_HEIGHT, Math.min(contentHeight(), maxHeight));
    if (Math.abs(target - inner.height) > 1) {
      await win.setSize(new LogicalSize(inner.width, target));
    }
    await win.center();
  } catch {
    /* 크기 조절 실패는 스크롤로 버틴다 */
  }
}

/// 위에 알림이 떠서 내용이 길어지면, 지금 자리에서 화면 아래 끝까지 남은 만큼만 늘린다.
/// 사용자가 옮겨 둔 창을 움직이거나 줄이지는 않는다. 모자라는 만큼은 스크롤이다.
async function growToContent() {
  try {
    const win = getCurrentWindow();
    const scale = await win.scaleFactor();
    const inner = (await win.innerSize()).toLogical(scale);
    const needed = contentHeight();
    if (needed <= inner.height + 1) return;

    const top = (await win.innerPosition()).toLogical(scale).y;
    const availTop = (window.screen as Screen & { availTop?: number }).availTop ?? 0;
    const room = Math.floor(availTop + window.screen.availHeight - top - 8);
    const target = Math.min(needed, room);
    if (target > inner.height + 1) {
      await win.setSize(new LogicalSize(inner.width, target));
    }
  } catch {
    /* 크기 조절 실패는 스크롤로 버틴다 */
  }
}

function setActionStatus(message: string, tone: "info" | "ok" | "error" = "info") {
  const el = $("action-status");
  el.textContent = message;
  el.classList.remove("hidden", "settings-status--ok", "settings-status--error");
  if (tone === "ok") el.classList.add("settings-status--ok");
  else if (tone === "error") el.classList.add("settings-status--error");
  void growToContent();
}

function clearActionStatus() {
  $("action-status").classList.add("hidden");
  $("action-status").textContent = "";
}

function badgeClass(state: WorkPhase): string {
  switch (state) {
    case "Working":
    case "Resting":
      return "settings-badge--ok";
    case "NeedLogin":
    case "FetchError":
      return "settings-badge--error";
    case "Done":
      return "settings-badge--done";
    default:
      return "settings-badge--muted";
  }
}

function renderConnection(snap: WorkSnapshot) {
  const badge = $("connection-badge");
  badge.textContent = phaseLabel(snap.state);
  badge.className = `settings-badge ${badgeClass(snap.state)}`;

  const detail = $("session-detail");
  if (snap.state === "NeedLogin") {
    detail.textContent = snap.error || snap.label || "브라우저 로그인 후 세션을 가져오세요.";
    return;
  }
  if (snap.state === "FetchError") {
    detail.textContent = snap.error || snap.label || "갱신에 실패했습니다.";
    return;
  }
  // work.rs 는 Working/Resting 라벨을 "오늘 누적 · 출근 09:43" 처럼 출근 시각까지
  // 붙여서 준다. 여기서 또 붙이면 "· 출근 09:43 · 출근 09:43" 이 된다.
  // 반대로 Done 라벨은 "오늘 누적 근무시간" 이라 출근 시각이 없다 — 그건 붙여줘야 한다.
  const base = snap.label || "연결됨";
  const startedAt = snap.startedAt;
  detail.textContent =
    startedAt && !base.includes(`출근 ${startedAt}`) ? `${base} · 출근 ${startedAt}` : base;
}

async function fetchWork(): Promise<WorkSnapshot> {
  return invoke<WorkSnapshot>("get_work");
}

async function pushWorkToMain(snap: WorkSnapshot) {
  await emit("work-updated", snap);
}

/// 백엔드 LoginInfo. 쿠키 값은 오지 않는다.
type LoginInfo = {
  connected: boolean;
  account: string | null;
  savedAtMs: number | null;
  expiresAtMs: number | null;
  hasProfile: boolean;
  browserFound: boolean;
};

function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString([], {
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/// 연결됨 · 계정 · 저장 시각 · 만료 예상
async function renderLoginInfo() {
  const info = await invoke<LoginInfo>("get_login_info");
  const el = $("login-detail");
  if (!info.browserFound) {
    el.textContent = "Chrome 또는 Edge를 찾지 못했습니다. 설치 후 다시 시도하세요.";
    return;
  }
  if (!info.connected) {
    el.textContent = info.hasProfile
      ? "연결 안 됨 · 전용 프로필에 로그인이 남아 있으면 버튼 한 번으로 다시 연결됩니다."
      : "연결 안 됨";
    return;
  }
  const parts = ["연결됨"];
  if (info.account) parts.push(info.account);
  if (info.savedAtMs) parts.push(`저장 ${formatDateTime(info.savedAtMs)}`);
  if (info.expiresAtMs) parts.push(`만료 예상 ${formatDateTime(info.expiresAtMs)}`);
  el.textContent = parts.join(" · ");
}

async function afterSessionChange(snap: WorkSnapshot) {
  renderConnection(snap);
  await pushWorkToMain(snap);
  await renderLoginInfo().catch(() => {});
}

function setConnectionButtonsDisabled(disabled: boolean) {
  for (const id of ["btn-login", "btn-disconnect", "btn-reset-profile"]) {
    ($(id) as HTMLButtonElement).disabled = disabled;
  }
}

async function refreshConnection() {
  try {
    const snap = await fetchWork();
    renderConnection(snap);
    await pushWorkToMain(snap);
    return snap;
  } catch (e) {
    const snap: WorkSnapshot = {
      state: "FetchError",
      workedSeconds: null,
      label: "갱신 실패",
      startedAt: null,
      error: String(e),
      fetchedAtMs: Date.now(),
    };
    renderConnection(snap);
    return snap;
  }
}

function renderRadioGroup<T extends string>(
  container: HTMLElement,
  name: string,
  options: { value: T; label: string; hint?: string }[],
  current: T,
  onChange: (value: T) => void,
) {
  container.replaceChildren();
  for (const opt of options) {
    const label = document.createElement("label");
    label.className = "option-item";

    const input = document.createElement("input");
    input.type = "radio";
    input.name = name;
    input.value = opt.value;
    input.checked = opt.value === current;

    const textWrap = document.createElement("span");
    textWrap.className = "option-item-text";
    const title = document.createElement("span");
    title.className = "option-item-title";
    title.textContent = opt.label;
    textWrap.append(title);
    if (opt.hint) {
      const hint = document.createElement("span");
      hint.className = "option-item-hint";
      hint.textContent = opt.hint;
      textWrap.append(hint);
    }

    input.addEventListener("change", () => {
      if (!input.checked) return;
      onChange(opt.value);
    });

    label.append(input, textWrap);
    container.append(label);
  }
}

async function notifyMain() {
  await emit("settings-changed", prefs);
}

async function setDisplayMode(mode: DisplayMode) {
  prefs = { ...prefs, displayMode: mode };
  savePreferences(prefs);
  renderDisplayMode();
  await notifyMain();
}

async function setAlwaysOnTop(enabled: boolean) {
  // Backend persists + applies to the main window; it is the source of truth.
  await invoke("set_always_on_top", { enabled });
  prefs = { ...prefs, alwaysOnTop: enabled };
}

async function renderAlwaysOnTop() {
  const toggle = $("always-on-top-toggle") as HTMLInputElement;
  try {
    const enabled = await invoke<boolean>("get_always_on_top");
    prefs = { ...prefs, alwaysOnTop: enabled };
    toggle.checked = enabled;
  } catch {
    toggle.checked = prefs.alwaysOnTop;
  }
}

async function setTheme(theme: ThemeMode) {
  prefs = { ...prefs, theme };
  savePreferences(prefs);
  applyTheme(theme);
  renderTheme();
  await notifyMain();
}

function renderDisplayMode() {
  renderRadioGroup(
    $("display-mode-picker"),
    "display-mode",
    [
      { value: "worked", label: "누적 근무시간", hint: "오늘 일한 시간" },
      { value: "remaining", label: "남은 근무시간", hint: "8시간 기준" },
    ],
    prefs.displayMode,
    (value) => {
      void setDisplayMode(value);
    },
  );
}

function renderTheme() {
  renderRadioGroup(
    $("theme-picker"),
    "theme",
    [
      { value: "system", label: "시스템" },
      { value: "light", label: "라이트" },
      { value: "dark", label: "다크" },
    ],
    prefs.theme,
    (value) => {
      void setTheme(value);
    },
  );
}

function syncAutostartUi() {
  const toggle = $("autostart-toggle") as HTMLInputElement;
  const hint = $("autostart-hint");

  if (isDevBuild) {
    toggle.checked = false;
    toggle.disabled = true;
    hint.textContent =
      "개발 모드에서는 시작프로그램을 바꿀 수 없습니다. 「시작.bat」으로 설치·실행한 뒤 다시 시도하세요.";
    hint.classList.remove("hidden");
    return;
  }

  toggle.disabled = false;
  toggle.checked = autostartEnabled;
  hint.classList.add("hidden");
}

async function refreshAutostart() {
  isDevBuild = await invoke<boolean>("is_dev_build");
  autostartEnabled = await invoke<boolean>("is_autostart_enabled");
  syncAutostartUi();
}

/// 백엔드를 건드리는 갱신 단계. 서로 의존하지 않으므로 한 단계가 실패해도
/// 나머지는 그대로 갱신돼야 한다. 순차 await 로 묶으면 앞단 하나가 reject 할 때
/// 뒷단이 통째로 건너뛰어져 UI 가 옛 상태로 남는다.
const REFRESH_STEPS: { name: string; run: () => Promise<unknown> }[] = [
  { name: "항상 위 표시", run: renderAlwaysOnTop },
  { name: "시작프로그램", run: refreshAutostart },
  { name: "연결 상태", run: refreshConnection },
  { name: "로그인 정보", run: renderLoginInfo },
];

async function refreshView() {
  clearActionStatus();
  prefs = loadPreferences();
  applyTheme(prefs.theme);
  renderDisplayMode();
  renderTheme();
  // 지난번에 그려 둔 내용으로 먼저 맞춘다. 값을 다시 읽은 뒤에 맞추면 창이 뜬 뒤
  // 크기가 튀는 시간이 길어진다.
  await fitToScreen();

  const results = await Promise.allSettled(REFRESH_STEPS.map((step) => step.run()));
  // 숨겨져 있으면(앱 시작 직후) 다음에 열릴 크기를 맞춰 두고, 떠 있으면 로그인 정보
  // 줄 등이 늘어난 만큼만 키운다.
  const visible = await getCurrentWindow()
    .isVisible()
    .catch(() => true);
  void (visible ? growToContent() : fitToScreen());

  // 실패를 삼키지 않는다. 콘솔에는 원인을, 상태줄에는 어느 단계인지 남긴다.
  const failed: string[] = [];
  results.forEach((result, i) => {
    if (result.status !== "rejected") return;
    const { name } = REFRESH_STEPS[i];
    failed.push(name);
    console.error(`[settings] ${name} 갱신 실패`, result.reason);
  });
  if (failed.length > 0) {
    setActionStatus(`${failed.join(", ")} 갱신에 실패했습니다. 새로고침을 눌러보세요.`, "error");
  }
}

async function closeWindow() {
  await invoke("close_settings_window");
}

async function onLogin() {
  clearActionStatus();
  setActionStatus("전용 Chrome 창을 여는 중…");
  setConnectionButtonsDisabled(true);
  try {
    const snap = await invoke<WorkSnapshot>("login_with_browser");
    await afterSessionChange(snap);
    if (snap.state === "NeedLogin" || snap.state === "FetchError") {
      setActionStatus(tidyMessage(snap.error || snap.label || "로그인 확인 실패"), "error");
    } else {
      setActionStatus("로그인했습니다. 위젯에 근무시간이 반영됩니다.", "ok");
    }
  } catch (e) {
    setActionStatus(tidyMessage(String(e)), "error");
  } finally {
    setConnectionButtonsDisabled(false);
  }
}

async function onDisconnect() {
  clearActionStatus();
  try {
    await afterSessionChange(await invoke<WorkSnapshot>("disconnect_session"));
    setActionStatus("연결을 해제했습니다. 다시 쓰려면 브라우저로 로그인하세요.", "ok");
  } catch (e) {
    setActionStatus(tidyMessage(String(e)), "error");
  }
}

/// 프로필까지 지우는 동작이라 한 번 더 눌러야 실행한다.
let resetArmedUntil = 0;

async function onResetProfile() {
  const btn = $("btn-reset-profile") as HTMLButtonElement;
  if (Date.now() > resetArmedUntil) {
    resetArmedUntil = Date.now() + 4000;
    btn.textContent = "한 번 더 누르면 초기화";
    window.setTimeout(() => {
      btn.textContent = "로그인 프로필 초기화";
    }, 4000);
    return;
  }
  resetArmedUntil = 0;
  btn.textContent = "로그인 프로필 초기화";
  clearActionStatus();
  try {
    await afterSessionChange(await invoke<WorkSnapshot>("reset_login_profile"));
    setActionStatus("로그인 프로필을 초기화했습니다. 다음 로그인은 처음부터 합니다.", "ok");
  } catch (e) {
    setActionStatus(tidyMessage(String(e)), "error");
  }
}

async function onRefresh() {
  clearActionStatus();
  const btn = $("btn-refresh") as HTMLButtonElement;
  btn.disabled = true;
  try {
    const snap = await refreshConnection();
    if (snap.state === "FetchError") {
      setActionStatus(tidyMessage(snap.error || "갱신 실패"), "error");
    } else {
      setActionStatus("근무시간을 갱신했습니다.", "ok");
    }
  } finally {
    btn.disabled = false;
  }
}

async function onOpenFlex() {
  clearActionStatus();
  try {
    await invoke("open_flex_home");
    setActionStatus("브라우저에서 flex.team을 열었습니다.");
  } catch (e) {
    setActionStatus(tidyMessage(String(e)), "error");
  }
}

async function boot() {
  try {
    await refreshView();

    $("btn-close").addEventListener("click", () => {
      void closeWindow();
    });
    $("btn-login").addEventListener("click", () => {
      void onLogin();
    });
    $("btn-disconnect").addEventListener("click", () => {
      void onDisconnect();
    });
    $("btn-reset-profile").addEventListener("click", () => {
      void onResetProfile();
    });
    await listen<string>("login-progress", (event) => {
      setActionStatus(event.payload);
    });
    $("btn-refresh").addEventListener("click", () => {
      void onRefresh();
    });
    $("btn-open-flex").addEventListener("click", () => {
      void onOpenFlex();
    });

    const alwaysOnTopToggle = $("always-on-top-toggle") as HTMLInputElement;
    alwaysOnTopToggle.addEventListener("change", async () => {
      const enabled = alwaysOnTopToggle.checked;
      try {
        await setAlwaysOnTop(enabled);
        setActionStatus(enabled ? "위젯을 항상 위에 표시합니다." : "위젯을 일반 창처럼 표시합니다.", "ok");
      } catch (e) {
        alwaysOnTopToggle.checked = prefs.alwaysOnTop;
        setActionStatus(tidyMessage(String(e)), "error");
      }
    });

    const autostartToggle = $("autostart-toggle") as HTMLInputElement;
    autostartToggle.addEventListener("change", async () => {
      if (isDevBuild) {
        syncAutostartUi();
        return;
      }
      autostartToggle.disabled = true;
      try {
        if (autostartToggle.checked) {
          await invoke("enable_autostart");
        } else {
          await invoke("disable_autostart");
        }
        autostartEnabled = await invoke<boolean>("is_autostart_enabled");
        syncAutostartUi();
        setActionStatus(autostartEnabled ? "시작프로그램을 켰습니다." : "시작프로그램을 껐습니다.", "ok");
      } catch (e) {
        autostartToggle.checked = autostartEnabled;
        setActionStatus(tidyMessage(String(e)), "error");
      } finally {
        autostartToggle.disabled = isDevBuild;
      }
    });

    await listen("settings-open", () => {
      void refreshView();
    });

    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape") void closeWindow();
    });
  } catch (e) {
    showBootError(String(e));
  }
}

void boot();
