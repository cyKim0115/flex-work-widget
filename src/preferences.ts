import { invoke } from "@tauri-apps/api/core";

export type DisplayMode = "worked" | "remaining";
export type ThemeMode = "system" | "light" | "dark";

export type Preferences = {
  displayMode: DisplayMode;
  theme: ThemeMode;
  /** Off means the widget behaves like a normal window and can go behind others. */
  alwaysOnTop: boolean;
};

export const MODE_KEY = "flex-work-display-mode";
export const THEME_KEY = "flex-work-theme";
export const ALWAYS_ON_TOP_KEY = "flex-work-always-on-top";

export function loadDisplayMode(): DisplayMode {
  const raw = localStorage.getItem(MODE_KEY);
  return raw === "remaining" ? "remaining" : "worked";
}

export function saveDisplayMode(mode: DisplayMode) {
  localStorage.setItem(MODE_KEY, mode);
}

export function loadTheme(): ThemeMode {
  const raw = localStorage.getItem(THEME_KEY);
  if (raw === "light" || raw === "dark" || raw === "system") return raw;
  return "system";
}

export function saveTheme(theme: ThemeMode) {
  localStorage.setItem(THEME_KEY, theme);
}

/** The widget shipped always-on-top, so an unset key must stay on. */
export function loadAlwaysOnTop(): boolean {
  return localStorage.getItem(ALWAYS_ON_TOP_KEY) !== "false";
}

export function saveAlwaysOnTop(enabled: boolean) {
  localStorage.setItem(ALWAYS_ON_TOP_KEY, enabled ? "true" : "false");
}

export function loadPreferences(): Preferences {
  return {
    displayMode: loadDisplayMode(),
    theme: loadTheme(),
    alwaysOnTop: loadAlwaysOnTop(),
  };
}

export function savePreferences(prefs: Preferences) {
  saveDisplayMode(prefs.displayMode);
  saveTheme(prefs.theme);
  saveAlwaysOnTop(prefs.alwaysOnTop);
}

/**
 * 배경 흐림(유리 효과) 저장값. 항상 위 표시처럼 백엔드(ui-pref.json)가 가져서,
 * 두 창과 다음 실행이 같은 값을 본다. 저장값이 없으면 켬·12px이다.
 */
export type GlassPref = { enabled: boolean; blur: number };

export async function loadGlassPref(): Promise<GlassPref> {
  return invoke<GlassPref>("get_glass_pref");
}

/** 저장하고 위젯 창에 반영한다. 실제로 켜졌는지 돌려준다(지원하지 않는 Windows면 false). */
export async function applyGlass(enabled: boolean): Promise<boolean> {
  return invoke<boolean>("set_glass", { enabled });
}

/** 흐림 반경(CSS px, 0~32). 백엔드가 저장하고 캡처 여백을 맞춘 뒤 위젯에 반경을 알린다. */
export async function applyGlassBlur(px: number) {
  await invoke("set_glass_blur", { px });
}

export function applyTheme(theme: ThemeMode) {
  document.documentElement.dataset.theme = theme;
}

export function modeLabel(mode: DisplayMode): string {
  return mode === "remaining" ? "남은" : "누적";
}

export function themeLabel(theme: ThemeMode): string {
  switch (theme) {
    case "light":
      return "라이트";
    case "dark":
      return "다크";
    default:
      return "시스템";
  }
}
