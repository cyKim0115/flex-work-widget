mod install;
mod work;

use std::path::PathBuf;
use std::process::Command;

use install::{
    autostart_disable, autostart_enable, autostart_is_enabled, cleanup_stale_debug_autostart,
    ensure_installed_release, guard_debug_requires_vite,
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_window_state::StateFlags;

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

fn always_on_top_pref_path() -> PathBuf {
    session_dir().join("ui-pref.json")
}

/// Backend owns this native window flag so the two webviews can't disagree and
/// the tauri.conf default can't re-force it on. Shipped on → default true.
fn load_always_on_top_pref() -> bool {
    std::fs::read_to_string(always_on_top_pref_path())
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("alwaysOnTop").and_then(|b| b.as_bool()))
        .unwrap_or(true)
}

fn save_always_on_top_pref(enabled: bool) -> Result<(), String> {
    let json = serde_json::json!({ "alwaysOnTop": enabled });
    let text = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(always_on_top_pref_path(), text).map_err(|e| e.to_string())
}

/// Apply to the main window AND persist, so the choice survives restarts.
#[tauri::command]
fn set_always_on_top(app: AppHandle, enabled: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    window.set_always_on_top(enabled).map_err(|e| e.to_string())?;
    save_always_on_top_pref(enabled)?;
    Ok(())
}

#[tauri::command]
fn get_always_on_top() -> bool {
    load_always_on_top_pref()
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

/// One detected flex session (metadata only — no cookie values).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarvestCandidate {
    id: String,
    browser: String,
    profile: String,
    account: String,
    #[serde(default)]
    is_last_used: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct HarvestPref {
    id: Option<String>,
}

fn pref_path() -> PathBuf {
    session_dir().join("harvest-pref.json")
}

fn load_harvest_pref() -> Option<String> {
    let raw = std::fs::read_to_string(pref_path()).ok()?;
    let pref: HarvestPref = serde_json::from_str(&raw).ok()?;
    pref.id.filter(|s| !s.is_empty())
}

fn save_harvest_pref(id: &str) -> Result<(), String> {
    let pref = HarvestPref {
        id: Some(id.to_string()),
    };
    let json = serde_json::to_string_pretty(&pref).map_err(|e| e.to_string())?;
    std::fs::write(pref_path(), json).map_err(|e| e.to_string())
}

/// PowerShell `-ArgumentList` wants a comma-separated list of quoted strings.
fn harvest_arglist(script: &str, prefer: Option<&str>) -> String {
    let mut parts = vec![ps_quote(script)];
    if let Some(p) = prefer {
        parts.push(ps_quote("--prefer"));
        parts.push(ps_quote(p));
    }
    parts.join(",")
}

/// Run the elevated cookie harvester, optionally preferring a specific profile,
/// then load session.json into the store. UAC prompt is expected (Chrome
/// app-bound encryption needs admin to decrypt on v130+).
fn run_harvest(
    app: &AppHandle,
    store: &State<'_, SessionStore>,
    prefer: Option<&str>,
) -> Result<WorkSnapshot, String> {
    let script = project_harvest_script().ok_or_else(|| {
        "harvest_browser_session.py 를 찾지 못했습니다. 프로젝트 루트에서 실행하세요.".to_string()
    })?;
    let script = dunce_canonicalize(&script)?;
    let python = dunce_canonicalize(&python_launcher()).unwrap_or_else(|_| python_launcher());

    let arg_list = harvest_arglist(&script.to_string_lossy(), prefer);
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
    let snap = fetch_work(store);
    let _ = app.emit("work-updated", &snap);
    Ok(snap)
}

/// Import session from Chrome/Edge, honouring a saved profile preference.
#[tauri::command]
fn harvest_browser_session(app: AppHandle, store: State<'_, SessionStore>) -> Result<WorkSnapshot, String> {
    let prefer = load_harvest_pref();
    run_harvest(&app, &store, prefer.as_deref())
}

#[tauri::command]
fn harvest_login_cookies(app: AppHandle, store: State<'_, SessionStore>) -> Result<WorkSnapshot, String> {
    harvest_browser_session(app, store)
}

/// Detected flex sessions from the most recent harvest, best-ranked first.
#[tauri::command]
fn list_harvest_candidates() -> Vec<HarvestCandidate> {
    let path = session_dir().join("candidates.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Which profile the last harvest actually used (session.json `source`).
#[tauri::command]
fn current_harvest_source() -> Option<String> {
    let raw = std::fs::read_to_string(session_dir().join("session.json")).ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    val.get("source")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Remember the chosen browser/profile and re-harvest from it (UAC prompt).
#[tauri::command]
fn set_harvest_preference(
    app: AppHandle,
    store: State<'_, SessionStore>,
    id: String,
) -> Result<WorkSnapshot, String> {
    save_harvest_pref(&id)?;
    run_harvest(&app, &store, Some(&id))
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
        // 위치만 복원한다. 창 크기는 메시지 표시 여부에 따라 프런트가 매번 정하므로
        // (COMPACT ↔ MESSAGE), 크기까지 저장하면 지난 실행 상태가 이번 실행을 덮어쓴다.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION)
                .with_denylist(&[SETTINGS_LABEL])
                .build(),
        )
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
            harvest_browser_session,
            list_harvest_candidates,
            current_harvest_source,
            set_harvest_preference,
            set_always_on_top,
            get_always_on_top
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
        .setup(|app| {
            cleanup_stale_debug_autostart();
            // Release builds keep a stable copy under LOCALAPPDATA for shortcuts/autostart.
            let _ = ensure_installed_release();
            // Honour the saved always-on-top choice; the conf default only sets the
            // initial state, so a user who turned it off keeps it off after restart.
            let enabled = load_always_on_top_pref();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_always_on_top(enabled);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
