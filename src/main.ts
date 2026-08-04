import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type WorkPhase = "NeedLogin" | "NotStarted" | "Working" | "Resting" | "Done" | "FetchError";

type WorkSnapshot = {
  state: WorkPhase;
  workedSeconds: number | null;
  label: string;
  startedAt: string | null;
  error: string | null;
  fetchedAtMs: number | null;
};

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

let latest: WorkSnapshot = {
  state: "NeedLogin",
  workedSeconds: null,
  label: "flex 로그인이 필요합니다",
  startedAt: null,
  error: null,
  fetchedAtMs: null,
};

let tickHandle: number | null = null;

function liveWorkedSeconds(snap: WorkSnapshot): number | null {
  if (snap.workedSeconds == null) return null;
  if (snap.state !== "Working" && snap.state !== "Resting") return snap.workedSeconds;
  if (snap.fetchedAtMs == null) return snap.workedSeconds;
  if (snap.state === "Resting") return snap.workedSeconds;
  const elapsed = Math.floor((Date.now() - snap.fetchedAtMs) / 1000);
  return snap.workedSeconds + Math.max(0, elapsed);
}

function render(snap: WorkSnapshot) {
  const widget = document.querySelector(".widget") as HTMLElement;
  widget.classList.toggle("error", snap.state === "FetchError" || snap.state === "NeedLogin");

  const badge = $("state-badge");
  badge.className = `badge ${phaseBadgeClass(snap.state)}`;
  badge.textContent = phaseBadgeText(snap.state);

  const seconds = liveWorkedSeconds(snap);
  if (snap.state === "NotStarted") {
    $("time").textContent = "00:00:00";
    $("subtitle").textContent = "아직 출근하지 않았습니다";
  } else if (seconds != null) {
    $("time").textContent = formatHms(seconds);
    $("subtitle").textContent = snap.label || "오늘 누적 근무시간";
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
    status.textContent = `updated ${hhmm}`;
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
  const menuRect = menu.getBoundingClientRect();
  const maxX = Math.max(8, window.innerWidth - menuRect.width - 8);
  const maxY = Math.max(8, window.innerHeight - menuRect.height - 8);
  menu.style.left = `${Math.min(x, maxX)}px`;
  menu.style.top = `${Math.min(y, maxY)}px`;
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

  window.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    showContextMenu(event.clientX, event.clientY);
  });

  backdrop.addEventListener("pointerdown", (event) => {
    if (event.target === backdrop) hideContextMenu();
  });

  $("context-menu").addEventListener("pointerdown", (event) => {
    event.stopPropagation();
  });

  window.addEventListener("blur", hideContextMenu);
  window.addEventListener("resize", hideContextMenu);
  window.addEventListener("click", hideContextMenu);
  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape") hideContextMenu();
  });

  $("menu-login-system").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    try {
      await invoke("open_login_system");
    } catch (e) {
      window.alert(String(e));
    }
  });

  $("menu-harvest").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    try {
      window.alert(
        "Chrome/Edge 쿠키를 읽기 위해 관리자 권한(UAC) 확인이 뜹니다.\n허용한 뒤 flex 근무시간이 갱신됩니다.",
      );
      const snap = await invoke<WorkSnapshot>("harvest_browser_session");
      await applySnap(snap);
      if (snap.state === "NeedLogin" || snap.state === "FetchError") {
        window.alert(snap.error || snap.label || "세션 가져오기 실패");
      }
    } catch (e) {
      window.alert(String(e));
    }
  });

  $("menu-paste").addEventListener("click", async (event) => {
    event.stopPropagation();
    hideContextMenu();
    const cookie = window.prompt(
      "Chrome DevTools → Network → flex.team 요청 → Request Headers의 Cookie 값을 붙여넣으세요.\n(AID / V2_WS_AID 포함)",
    );
    if (!cookie || !cookie.trim()) return;
    try {
      const snap = await invoke<WorkSnapshot>("import_cookie_header", {
        cookieHeader: cookie.trim(),
      });
      await applySnap(snap);
      if (snap.state === "NeedLogin" || snap.state === "FetchError") {
        window.alert(snap.error || snap.label || "Cookie 가져오기 실패");
      }
    } catch (e) {
      window.alert(String(e));
    }
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
