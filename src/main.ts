import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  applyTheme,
  loadDisplayMode,
  loadTheme,
  modeLabel,
  saveDisplayMode,
  type DisplayMode,
  type ThemeMode,
} from "./preferences";

type WorkPhase = "NeedLogin" | "NotStarted" | "Working" | "Resting" | "Done" | "FetchError";

type WorkSnapshot = {
  state: WorkPhase;
  workedSeconds: number | null;
  label: string;
  startedAt: string | null;
  error: string | null;
  fetchedAtMs: number | null;
};

const COMPACT = { width: 280, height: 128 };
const MESSAGE = { width: 420, height: 260 };
const MENU = { width: 280, height: 280 };
const TARGET_WORK_SECONDS = 8 * 60 * 60;

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

function formatHms(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const hh = Math.floor(s / 3600);
  const mm = Math.floor((s % 3600) / 60);
  const ss = s % 60;
  return `${String(hh).padStart(2, "0")}:${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
}

function phaseBadgeClass(state: WorkPhase): string {
  switch (state) {
    case "NotStarted":
      return "not-started";
    case "Working":
      return "working";
    case "Resting":
      return "resting";
    case "Done":
      return "done";
    case "NeedLogin":
      return "need-login";
    default:
      return "error";
  }
}

function phaseBadgeText(state: WorkPhase): string {
  switch (state) {
    case "NotStarted":
      return "출근 전";
    case "Working":
      return "근무 중";
    case "Resting":
      return "휴게 중";
    case "Done":
      return "퇴근";
    case "NeedLogin":
      return "로그인 필요";
    default:
      return "오류";
  }
}

function tidyMessage(raw: string): string {
  return raw
    .replace(/\r\n/g, "\n")
    .replace(/\nLocation:\s*\n\s*rookie-rs[\s\S]*$/gim, "")
    .replace(/chrome:\s*decrypt_encrypted_value failed/gi, "Chrome 쿠키 복호화 실패")
    .replace(/edge:\s*decrypt_encrypted_value failed/gi, "Edge 쿠키 복호화 실패")
    .replace(/can be decrypted only when running as admin[^\n]*/gi, "관리자 권한(UAC) 필요")
    .trim();
}

let latest: WorkSnapshot = {
  state: "NeedLogin",
  workedSeconds: null,
  label: "flex 로그인이 필요합니다",
  startedAt: null,
  error: null,
  fetchedAtMs: null,
};

let displayMode: DisplayMode = loadDisplayMode();
let themeMode: ThemeMode = loadTheme();
let tickHandle: number | null = null;
let msgResolver: (() => void) | null = null;

async function setWidgetSize(size: { width: number; height: number }) {
  try {
    await getCurrentWindow().setSize(new LogicalSize(size.width, size.height));
  } catch {
    /* ignore */
  }
}

function hideMessage() {
  $("msg-backdrop").classList.add("hidden");
  void setWidgetSize(COMPACT);
  if (msgResolver) {
    const done = msgResolver;
    msgResolver = null;
    done();
  }
}

async function showMessage(title: string, body: string): Promise<void> {
  hideContextMenu();
  $("msg-title").textContent = title;
  $("msg-body").textContent = tidyMessage(body);
  await setWidgetSize(MESSAGE);
  $("msg-backdrop").classList.remove("hidden");
  return new Promise((resolve) => {
    msgResolver = resolve;
  });
}

function liveWorkedSeconds(snap: WorkSnapshot): number | null {
  if (snap.workedSeconds == null) return null;
  if (snap.state !== "Working" && snap.state !== "Resting") return snap.workedSeconds;
  if (snap.fetchedAtMs == null) return snap.workedSeconds;
  if (snap.state === "Resting") return snap.workedSeconds;
  const elapsed = Math.floor((Date.now() - snap.fetchedAtMs) / 1000);
  return snap.workedSeconds + Math.max(0, elapsed);
}

function displaySeconds(worked: number | null): number | null {
  if (worked == null) return null;
  if (displayMode === "remaining") {
    return Math.max(0, TARGET_WORK_SECONDS - worked);
  }
  return worked;
}

function modeSubtitle(snap: WorkSnapshot): string {
  if (displayMode === "remaining") {
    if (snap.startedAt) return `남은 근무시간 · 출근 ${snap.startedAt}`;
    return "남은 근무시간 (8시간 기준)";
  }
  return snap.label || "오늘 누적 근무시간";
}

function setDisplayMode(mode: DisplayMode) {
  displayMode = mode;
  saveDisplayMode(mode);
  render(latest);
}

function render(snap: WorkSnapshot) {
  const widget = document.querySelector(".widget") as HTMLElement;
  widget.classList.toggle("error", snap.state === "FetchError" || snap.state === "NeedLogin");

  const badge = $("state-badge");
  badge.className = `badge ${phaseBadgeClass(snap.state)}`;
  badge.textContent = phaseBadgeText(snap.state);

  const worked = liveWorkedSeconds(snap);
  if (snap.state === "NotStarted") {
    const seconds = displayMode === "remaining" ? TARGET_WORK_SECONDS : 0;
    $("time").textContent = formatHms(seconds);
    $("subtitle").textContent =
      displayMode === "remaining" ? "남은 근무시간 (8시간 기준)" : "아직 출근하지 않았습니다";
  } else if (worked != null) {
    $("time").textContent = formatHms(displaySeconds(worked) ?? 0);
    $("subtitle").textContent = modeSubtitle(snap);
  } else {
    $("time").textContent = "——:——";
    $("subtitle").textContent = snap.label || snap.error || "—";
  }

  const status = $("status");
  const now = new Date();
  const hhmm = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (snap.state === "NeedLogin") {
    status.textContent = "우클릭 → 브라우저 로그인 → 세션 가져오기";
  } else if (snap.state === "FetchError") {
    status.textContent = `갱신 실패 · ${hhmm}`;
  } else {
    const modeHint = modeLabel(displayMode);
    status.textContent = `${modeHint} · updated ${hhmm}`;
  }
}

function ensureTicker() {
  if (tickHandle != null) return;
  tickHandle = window.setInterval(() => {
    if (latest.state === "Working") render(latest);
  }, 1000);
}

function hideContextMenu() {
  const wasOpen = !$("context-menu").classList.contains("hidden");
  $("context-menu").classList.add("hidden");
  $("context-backdrop").classList.add("hidden");
  if (wasOpen && $("msg-backdrop").classList.contains("hidden")) {
    void setWidgetSize(COMPACT);
  }
}

async function showContextMenu(x: number, y: number) {
  await setWidgetSize(MENU);
  const backdrop = $("context-backdrop");
  const menu = $("context-menu");
  backdrop.classList.remove("hidden");
  menu.classList.remove("hidden");
  requestAnimationFrame(() => {
    const menuRect = menu.getBoundingClientRect();
    const maxX = Math.max(8, window.innerWidth - menuRect.width - 8);
    const maxY = Math.max(8, window.innerHeight - menuRect.height - 8);
    menu.style.left = `${Math.min(Math.max(8, x), maxX)}px`;
    menu.style.top = `${Math.min(Math.max(8, y), maxY)}px`;
  });
}

async function openSettings() {
  hideContextMenu();
  try {
    await invoke("open_settings_window");
  } catch (e) {
    await showMessage("오류", String(e));
  }
}

async function applySnap(snap: WorkSnapshot) {
  latest = snap;
  render(snap);
  ensureTicker();
}

async function refresh() {
  try {
    const snap = await invoke<WorkSnapshot>("get_work");
    await applySnap(snap);
  } catch (e) {
    await applySnap({
      state: "FetchError",
      workedSeconds: null,
      label: "갱신 실패",
      startedAt: null,
      error: String(e),
      fetchedAtMs: Date.now(),
    });
  }
}

async function boot() {
  const backdrop = $("context-backdrop");
  applyTheme(themeMode);

  window.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    if (!$("msg-backdrop").classList.contains("hidden")) return;
    void (async () => {
      await showContextMenu(event.clientX, event.clientY);
    })();
  });

  backdrop.addEventListener("pointerdown", (event) => {
    if (event.target === backdrop) hideContextMenu();
  });

  $("context-menu").addEventListener("pointerdown", (event) => {
    event.stopPropagation();
  });

  $("msg-ok").addEventListener("click", (event) => {
    event.stopPropagation();
    hideMessage();
  });

  $("msg-backdrop").addEventListener("pointerdown", (event) => {
    if (event.target === $("msg-backdrop")) hideMessage();
  });

  const timeEl = $("time");
  timeEl.title = "클릭: 누적 ↔ 남은 근무시간";
  timeEl.addEventListener("click", (event) => {
    event.stopPropagation();
    if (!$("msg-backdrop").classList.contains("hidden")) return;
    setDisplayMode(displayMode === "worked" ? "remaining" : "worked");
  });

  window.addEventListener("blur", () => {
    if ($("msg-backdrop").classList.contains("hidden")) hideContextMenu();
  });
  window.addEventListener("resize", () => {
    if ($("msg-backdrop").classList.contains("hidden")) hideContextMenu();
  });
  window.addEventListener("click", () => {
    if ($("msg-backdrop").classList.contains("hidden")) hideContextMenu();
  });
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      if (!$("msg-backdrop").classList.contains("hidden")) hideMessage();
      else hideContextMenu();
    }
  });

  $("menu-login-system").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    try {
      await invoke("open_login_system");
    } catch (e) {
      await showMessage("오류", String(e));
    }
  });

  $("menu-harvest").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    try {
      await showMessage(
        "세션 가져오기",
        "Chrome/Edge 쿠키를 읽기 위해 관리자 권한(UAC) 확인이 뜹니다.\n허용한 뒤 근무시간이 갱신됩니다.",
      );
      const snap = await invoke<WorkSnapshot>("harvest_browser_session");
      await applySnap(snap);
      if (snap.state === "NeedLogin" || snap.state === "FetchError") {
        await showMessage(
          "세션 가져오기 실패",
          `${snap.error || snap.label || "실패"}\n\nEdge에서 flex.team에 로그인한 뒤 다시 시도하세요.`,
        );
      }
    } catch (e) {
      await showMessage("오류", String(e));
    }
  });

  $("menu-settings").addEventListener("click", (event) => {
    event.stopPropagation();
    void openSettings();
  });

  $("menu-refresh").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    await refresh();
  });

  $("menu-open").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    await invoke("open_flex_home");
  });

  $("menu-quit").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    await invoke("quit_app");
  });

  await listen<{ displayMode: DisplayMode; theme: ThemeMode }>("settings-changed", (event) => {
    displayMode = event.payload.displayMode;
    themeMode = event.payload.theme;
    applyTheme(themeMode);
    render(latest);
  });

  await listen<WorkSnapshot>("work-updated", (event) => {
    void applySnap(event.payload);
  });

  await refresh();
  let interval = 60_000;
  try {
    interval = await invoke<number>("get_poll_interval_ms");
  } catch {
    /* keep default */
  }
  window.setInterval(() => {
    void refresh();
  }, interval);
}

void boot();
