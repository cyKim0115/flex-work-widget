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

function renderRadioGroup<T extends string>(
  container: HTMLElement,
  name: string,
  options: { value: T; label: string }[],
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

    const text = document.createElement("span");
    text.textContent = opt.label;

    input.addEventListener("change", () => {
      if (!input.checked) return;
      onChange(opt.value);
    });

    label.append(input, text);
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
      { value: "worked", label: "누적 근무시간" },
      { value: "remaining", label: "남은 근무시간 (8시간 기준)" },
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
  prefs = loadPreferences();
  applyTheme(prefs.theme);
  renderDisplayMode();
  renderTheme();
  await refreshAutostart();
}

async function closeWindow() {
  await invoke("close_settings_window");
}

async function boot() {
  try {
    applyTheme(prefs.theme);
    renderDisplayMode();
    renderTheme();
    await refreshAutostart();

    $("btn-close").addEventListener("click", () => {
      void closeWindow();
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
      } catch (e) {
        autostartToggle.checked = autostartEnabled;
        showBootError(String(e));
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
