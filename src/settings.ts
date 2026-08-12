import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import {
  applyTheme,
  loadPreferences,
  savePreferences,
  type DisplayMode,
  type Preferences,
  type ThemeMode,
} from "./preferences";
import { phaseLabel, tidyMessage, type WorkPhase, type WorkSnapshot } from "./work-types";

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

function setActionStatus(message: string, tone: "info" | "ok" | "error" = "info") {
  const el = $("action-status");
  el.textContent = message;
  el.classList.remove("hidden", "settings-status--ok", "settings-status--error");
  if (tone === "ok") el.classList.add("settings-status--ok");
  else if (tone === "error") el.classList.add("settings-status--error");
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
  if (snap.startedAt) {
    detail.textContent = `${snap.label || "연결됨"} · 출근 ${snap.startedAt}`;
    return;
  }
  detail.textContent = snap.label || "연결됨";
}

async function fetchWork(): Promise<WorkSnapshot> {
  return invoke<WorkSnapshot>("get_work");
}

async function pushWorkToMain(snap: WorkSnapshot) {
  await emit("work-updated", snap);
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

async function refreshView() {
  clearActionStatus();
  prefs = loadPreferences();
  applyTheme(prefs.theme);
  renderDisplayMode();
  renderTheme();
  await refreshAutostart();
  await refreshConnection();
}

async function closeWindow() {
  await invoke("close_settings_window");
}

async function onLogin() {
  clearActionStatus();
  try {
    await invoke("open_login_system");
    setActionStatus("브라우저에서 flex.team 로그인을 완료한 뒤 세션 가져오기를 눌러주세요.");
  } catch (e) {
    setActionStatus(tidyMessage(String(e)), "error");
  }
}

async function onHarvest() {
  clearActionStatus();
  setActionStatus("세션 가져오기 중… UAC 창이 뜨면 허용해주세요.");
  const btn = $("btn-harvest") as HTMLButtonElement;
  btn.disabled = true;
  try {
    const snap = await invoke<WorkSnapshot>("harvest_browser_session");
    renderConnection(snap);
    await pushWorkToMain(snap);
    if (snap.state === "NeedLogin" || snap.state === "FetchError") {
      setActionStatus(
        tidyMessage(snap.error || snap.label || "세션 가져오기 실패") +
          "\n\nEdge에서 flex.team에 로그인한 뒤 다시 시도하세요.",
        "error",
      );
    } else {
      setActionStatus("세션을 가져왔습니다. 위젯에 근무시간이 반영됩니다.", "ok");
    }
  } catch (e) {
    setActionStatus(tidyMessage(String(e)), "error");
  } finally {
    btn.disabled = false;
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
    applyTheme(prefs.theme);
    renderDisplayMode();
    renderTheme();
    await refreshAutostart();
    await refreshConnection();

    $("btn-close").addEventListener("click", () => {
      void closeWindow();
    });
    $("btn-login").addEventListener("click", () => {
      void onLogin();
    });
    $("btn-harvest").addEventListener("click", () => {
      void onHarvest();
    });
    $("btn-refresh").addEventListener("click", () => {
      void onRefresh();
    });
    $("btn-open-flex").addEventListener("click", () => {
      void onOpenFlex();
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
