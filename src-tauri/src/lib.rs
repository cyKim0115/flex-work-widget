mod install;
mod work;

use std::path::PathBuf;
use std::process::Command;

use install::{
    autostart_disable, autostart_enable, autostart_is_enabled, cleanup_stale_debug_autostart,
    ensure_installed_release, guard_debug_requires_vite,
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

const SETTINGS_LABEL: &str = "settings";
use work::{fetch_work, need_login, session_dir, SessionStore, WorkSnapshot};

const POLL_INTERVAL_MS: u64 = 60_000;
const FLEX_HOME: &str = "https://flex.team/home";
const FLEX_LOGIN: &str = "https://flex.team/auth/login?nextUrl=%2Fhome";

#[tauri::command]
fn get_poll_interval_ms() -> u64 {
    POLL_INTERVAL_MS
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn open_settings_window(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window(SETTINGS_LABEL)
        .ok_or_else(|| "settings window not found".to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.unminimize().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    window
        .emit("settings-open", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn close_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

#[tauri::command]
fn enable_autostart() -> Result<(), String> {
    autostart_enable()
}

#[tauri::command]
fn disable_autostart() -> Result<(), String> {
    autostart_disable()
}

#[tauri::command]
fn is_autostart_enabled() -> Result<bool, String> {
    autostart_is_enabled()
}

#[tauri::command]
fn install_release_copy() -> Result<String, String> {
    ensure_installed_release().map(|p| p.display().to_string())
}

#[tauri::command]
fn open_flex_home() -> Result<(), String> {
    open::that(FLEX_HOME).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_login_system() -> Result<(), String> {
    open::that(FLEX_LOGIN).map_err(|e| e.to_string())
}

/// WebView2 login is blank on flex — always use the system browser.
#[tauri::command]
fn open_login() -> Result<(), String> {
    open_login_system()
}

fn project_harvest_script() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("../../../scripts/harvest_browser_session.py"));
            candidates.push(parent.join("../../scripts/harvest_browser_session.py"));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts/harvest_browser_session.py"));
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("scripts/harvest_browser_session.py"));
        candidates.push(cwd.join("../scripts/harvest_browser_session.py"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn python_launcher() -> PathBuf {
    which_bin("python").unwrap_or_else(|| PathBuf::from("python"))
}

fn which_bin(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(format!("{name}.exe"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn dunce_canonicalize(path: &PathBuf) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map(|p| {
            let s = p.to_string_lossy();
            if let Some(stripped) = s.strip_prefix(r"\\?\") {
                PathBuf::from(stripped)
            } else {
                p
            }
        })
        .map_err(|e| e.to_string())
}

/// Import session from Chrome/Edge. UAC prompt expected (Chrome app-bound encryption).
#[tauri::command]
fn harvest_browser_session(app: AppHandle, store: State<'_, SessionStore>) -> Result<WorkSnapshot, String> {
    let script = project_harvest_script().ok_or_else(|| {
        "harvest_browser_session.py 를 찾지 못했습니다. 프로젝트 루트에서 실행하세요.".to_string()
    })?;
    let script = dunce_canonicalize(&script)?;
    let python = dunce_canonicalize(&python_launcher()).unwrap_or_else(|_| python_launcher());

    let arg_list = ps_quote(&script.to_string_lossy());
    let file = ps_quote(&python.to_string_lossy());
    let ps = format!(
        "$p = Start-Process -FilePath {file} -ArgumentList {arg_list} -Verb RunAs -Wait -PassThru; if ($null -eq $p) {{ exit 1 }}; exit $p.ExitCode"
    );

    let status = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
        .status()
        .map_err(|e| format!("관리자 권한 실행 실패: {e}"))?;

    let err_path = session_dir().join("session-error.txt");
    if !status.success() {
        let detail = std::fs::read_to_string(&err_path).unwrap_or_else(|_| {
            "Chrome/Edge 쿠키를 읽지 못했습니다. UAC 허용 여부 / flex.team 로그인 상태를 확인하세요.".into()
        });
        let snap = need_login(detail);
        let _ = app.emit("work-updated", &snap);
        return Ok(snap);
    }

    let path = session_dir().join("session.json");
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("session.json 읽기 실패: {e}"))?;
    let tokens: work::SessionTokens =
        serde_json::from_str(&raw).map_err(|e| format!("session.json 파싱 실패: {e}"))?;
    if tokens.aid.is_empty() && tokens.ws_aid.is_empty() {
        let snap = need_login("세션 파일에 AID 쿠키가 없습니다".into());
        let _ = app.emit("work-updated", &snap);
        return Ok(snap);
    }
    store.save(tokens)?;
    let snap = fetch_work(&store);
    let _ = app.emit("work-updated", &snap);
    Ok(snap)
}

#[tauri::command]
fn harvest_login_cookies(app: AppHandle, store: State<'_, SessionStore>) -> Result<WorkSnapshot, String> {
    harvest_browser_session(app, store)
}

#[tauri::command]
fn get_work(_app: AppHandle, store: State<'_, SessionStore>) -> WorkSnapshot {
    match store.get() {
        None => need_login("1) 브라우저 로그인  2) 세션 가져오기".into()),
        Some(_) => fetch_work(&store),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    guard_debug_requires_vite();

    let _ = std::fs::create_dir_all(session_dir());
    let store = SessionStore::new(session_dir());

    tauri::Builder::default()
        .manage(store)
        .invoke_handler(tauri::generate_handler![
            get_work,
            get_poll_interval_ms,
            quit_app,
            open_settings_window,
            close_settings_window,
            is_dev_build,
            enable_autostart,
            disable_autostart,
            is_autostart_enabled,
            install_release_copy,
            open_login,
            open_login_system,
            open_flex_home,
            harvest_login_cookies,
            harvest_browser_session
        ])
        .on_window_event(|window, event| {
            if window.label() != SETTINGS_LABEL {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|_app| {
            cleanup_stale_debug_autostart();
            // Release builds keep a stable copy under LOCALAPPDATA for shortcuts/autostart.
            let _ = ensure_installed_release();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
