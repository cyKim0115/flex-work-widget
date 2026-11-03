import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  applyGlass,
  applyGlassBlur,
  applyTheme,
  loadDisplayMode,
  loadGlassPref,
  loadTheme,
  modeLabel,
  saveDisplayMode,
  type DisplayMode,
  type ThemeMode,
} from "./preferences";
import { phaseLabel, type WorkPhase, type WorkSnapshot } from "./work-types";

const COMPACT = { width: 280, height: 128 };
const MESSAGE = { width: 420, height: 260 };
const TARGET_WORK_SECONDS = 8 * 60 * 60;

function $(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

/** 카드 투명도는 배경 흐림이 실제로 켜졌을 때만 낮춘다. 블러 없이 낮추면 뒤 글자가 선명하게 비친다. */
function setGlassClass(applied: boolean) {
  document.documentElement.classList.toggle("glass", applied);
  if (!applied) {
    const canvas = $("glass-backdrop") as HTMLCanvasElement;
    canvas.width = 0;
    canvas.height = 0;
  }
}

function setGlassBlur(px: number) {
  document.documentElement.style.setProperty("--glass-blur", `${px}px`);
}

/** glass.rs가 보내는 캡처 정보. 크기와 여백은 물리 픽셀이다. */
interface GlassFrame {
  seq: number;
  width: number;
  height: number;
  margin: number;
}

let glassBusy = false;
let glassPending: GlassFrame | null = null;

/**
 * 위젯 뒤를 캡처한 화면을 카드 아래 캔버스에 그린다. 블러는 CSS(--glass-blur)가 건다.
 * 받아 오는 중에 새 프레임이 오면 마지막 것만 남겨 두었다가 이어서 그린다.
 */
async function drawGlassFrame(frame: GlassFrame) {
  glassBusy = true;
  try {
    const response = await fetch(`http://glassbg.localhost/frame?${frame.seq}`);
    const bytes = new Uint8ClampedArray(await response.arrayBuffer());
    if (bytes.length !== frame.width * frame.height * 4) return;
    const canvas = $("glass-backdrop") as HTMLCanvasElement;
    if (canvas.width !== frame.width || canvas.height !== frame.height) {
      canvas.width = frame.width;
      canvas.height = frame.height;
    }
    // 캔버스 왼쪽 위가 창 왼쪽 위에서 여백만큼 밖에 오도록 둔다. 카드 테두리 두께도 뺀다.
    const scale = window.devicePixelRatio || 1;
    const border = (document.querySelector(".widget") as HTMLElement).clientLeft;
    canvas.style.width = `${frame.width / scale}px`;
    canvas.style.height = `${frame.height / scale}px`;
    canvas.style.left = `${-frame.margin / scale - border}px`;
    canvas.style.top = `${-frame.margin / scale - border}px`;
    canvas.getContext("2d")?.putImageData(new ImageData(bytes, frame.width, frame.height), 0, 0);
  } catch {
    /* 다음 프레임에서 다시 그린다 */
  } finally {
    glassBusy = false;
    const next = glassPending;
    glassPending = null;
    if (next && document.documentElement.classList.contains("glass")) void drawGlassFrame(next);
  }
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
  $("msg-body").textContent = body;
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
  badge.textContent = phaseLabel(snap.state);

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
    status.textContent = "우클릭 → 설정 → 브라우저로 로그인";
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
  $("context-menu").classList.add("hidden");
  $("context-backdrop").classList.add("hidden");
}

function showContextMenu(x: number, y: number) {
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
  // Always-on-top is owned by the backend: it applies the saved choice in setup(),
  // so the main window no longer reads it from (per-window) localStorage here.

  window.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    if (!$("msg-backdrop").classList.contains("hidden")) return;
    showContextMenu(event.clientX, event.clientY);
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

  $("menu-settings").addEventListener("click", (event) => {
    event.stopPropagation();
    void openSettings();
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

  // 설정 창에서 배경 흐림을 바꾸면 Rust가 캡처를 켜고 끈 뒤 결과를 보내 온다.
  await listen<boolean>("glass-applied", (event) => {
    setGlassClass(event.payload);
  });
  await listen<number>("glass-blur", (event) => {
    setGlassBlur(event.payload);
  });
  await listen<GlassFrame>("glass-frame", (event) => {
    if (!document.documentElement.classList.contains("glass")) return;
    if (glassBusy) {
      glassPending = event.payload;
      return;
    }
    void drawGlassFrame(event.payload);
  });

  // 근무시간 조회(네트워크)를 기다리지 않고 먼저 켠다. 저장값은 백엔드(ui-pref.json)에 있다.
  try {
    const glass = await loadGlassPref();
    // 반경을 먼저 넘겨야 첫 캡처부터 그 반경에 맞는 여백으로 찍는다.
    await applyGlassBlur(glass.blur);
    setGlassClass(await applyGlass(glass.enabled));
  } catch {
    /* browser preview */
  }

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
