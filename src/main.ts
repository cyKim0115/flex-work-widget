import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

type WorkPhase = "NeedLogin" | "NotStarted" | "Working" | "Resting" | "Done" | "FetchError";
type DisplayMode = "worked" | "remaining";
type ThemeMode = "system" | "light" | "dark";

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
const MENU = { width: 280, height: 360 };
const TARGET_WORK_SECONDS = 8 * 60 * 60;
const MODE_KEY = "flex-work-display-mode";
const THEME_KEY = "flex-work-theme";

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

function loadMode(): DisplayMode {
  const raw = localStorage.getItem(MODE_KEY);
  return raw === "remaining" ? "remaining" : "worked";
}

function saveMode(mode: DisplayMode) {
  localStorage.setItem(MODE_KEY, mode);
}

function loadTheme(): ThemeMode {
  const raw = localStorage.getItem(THEME_KEY);
  if (raw === "light" || raw === "dark" || raw === "system") return raw;
  return "system";
}

function saveTheme(theme: ThemeMode) {
  localStorage.setItem(THEME_KEY, theme);
}

function applyTheme(theme: ThemeMode) {
  document.documentElement.dataset.theme = theme;
}

let latest: WorkSnapshot = {
  state: "NeedLogin",
  workedSeconds: null,
  label: "flex 로그인이 필요합니다",
  startedAt: null,
  error: null,
  fetchedAtMs: null,
};

let displayMode: DisplayMode = loadMode();
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

function modeLabel(mode: DisplayMode): string {
  return mode === "remaining" ? "남은" : "누적";
}

function themeLabel(theme: ThemeMode): string {
  switch (theme) {
    case "light":
      return "라이트";
    case "dark":
      return "다크";
    default:
      return "시스템";
  }
}

function syncModeMenu() {
  $("menu-mode-worked").classList.toggle("checked", displayMode === "worked");
  $("menu-mode-remaining").classList.toggle("checked", displayMode === "remaining");
  $("menu-mode-summary").textContent = modeLabel(displayMode);
}

function syncThemeMenu() {
  $("menu-theme-system").classList.toggle("checked", themeMode === "system");
  $("menu-theme-light").classList.toggle("checked", themeMode === "light");
  $("menu-theme-dark").classList.toggle("checked", themeMode === "dark");
  $("menu-theme-summary").textContent = themeLabel(themeMode);
}

function setGroupOpen(groupName: string, open: boolean) {
  const group = document.querySelector(`.menu-group[data-group="${groupName}"]`);
  if (!group) return;
  const toggle = group.querySelector(".menu-group-toggle") as HTMLButtonElement | null;
  const items = group.querySelector(".menu-group-items");
  if (!toggle || !items) return;
  group.classList.toggle("open", open);
  toggle.setAttribute("aria-expanded", open ? "true" : "false");
  items.classList.toggle("hidden", !open);
}

function collapseAllGroups() {
  document.querySelectorAll(".menu-group").forEach((el) => {
    const name = (el as HTMLElement).dataset.group;
    if (name) setGroupOpen(name, false);
  });
}

function toggleGroup(groupName: string) {
  const group = document.querySelector(`.menu-group[data-group="${groupName}"]`);
  if (!group) return;
  const willOpen = !group.classList.contains("open");
  // Accordion: only one category open at a time
  collapseAllGroups();
  if (willOpen) setGroupOpen(groupName, true);
}

function setDisplayMode(mode: DisplayMode) {
  displayMode = mode;
  saveMode(mode);
  syncModeMenu();
  render(latest);
}

function setThemeMode(theme: ThemeMode) {
  themeMode = theme;
  saveTheme(theme);
  applyTheme(theme);
  syncThemeMenu();
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
    const modeHint = displayMode === "remaining" ? "남은" : "누적";
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
  collapseAllGroups();
  if (wasOpen && $("msg-backdrop").classList.contains("hidden")) {
    void setWidgetSize(COMPACT);
  }
}

async function showContextMenu(x: number, y: number) {
  syncModeMenu();
  syncThemeMenu();
  collapseAllGroups();
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
  syncModeMenu();
  syncThemeMenu();

  window.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    if (!$("msg-backdrop").classList.contains("hidden")) return;
    void showContextMenu(event.clientX, event.clientY);
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

  // Click time area to toggle display mode
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

  $("menu-group-mode").addEventListener("click", (event) => {
    event.stopPropagation();
    toggleGroup("mode");
  });

  $("menu-group-theme").addEventListener("click", (event) => {
    event.stopPropagation();
    toggleGroup("theme");
  });

  $("menu-mode-worked").addEventListener("click", (event) => {
    event.stopPropagation();
    hideContextMenu();
    setDisplayMode("worked");
  });

  $("menu-mode-remaining").addEventListener("click", (event) => {
    event.stopPropagation();
    hideContextMenu();
    setDisplayMode("remaining");
  });

  $("menu-theme-system").addEventListener("click", (event) => {
    event.stopPropagation();
    hideContextMenu();
    setThemeMode("system");
  });

  $("menu-theme-light").addEventListener("click", (event) => {
    event.stopPropagation();
    hideContextMenu();
    setThemeMode("light");
  });

  $("menu-theme-dark").addEventListener("click", (event) => {
    event.stopPropagation();
    hideContextMenu();
    setThemeMode("dark");
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
