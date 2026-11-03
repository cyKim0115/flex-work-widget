mod browser_login;
#[cfg(windows)]
mod glass;
mod install;
mod secure_store;
mod tray;
mod work;

/// 배경 흐림은 Windows 전용이다(glass.rs). 다른 OS에서는 늘 꺼진 채로 둔다.
#[cfg(not(windows))]
mod glass {
    use tauri::http::{Request, Response};
    use tauri::WebviewWindow;

    pub const MAX_BLUR_PX: f64 = 32.0;
    pub const DEFAULT_BLUR_PX: f64 = 12.0;

    pub fn supported() -> bool {
        false
    }

    pub fn apply(_window: &WebviewWindow, _enabled: bool) -> bool {
        false
    }

    pub fn set_blur(_px: f64) {}

    pub fn wake() {}

    pub fn frame_response(_webview_label: &str, _request: Request<Vec<u8>>) -> Response<Vec<u8>> {
        Response::builder().status(403).body(Vec::new()).unwrap()
    }
}

use std::path::PathBuf;
use std::sync::Arc;

use install::{
    autostart_disable, autostart_enable, autostart_is_enabled, cleanup_stale_debug_autostart,
    ensure_installed_release, guard_debug_requires_vite,
};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_window_state::StateFlags;

const SETTINGS_LABEL: &str = "settings";
use work::{
    fetch_work, fetch_work_with_refresh, need_login, session_dir, SessionStore, WorkSnapshot,
    FLEX_LOGIN, LOGIN_HINT,
};

const POLL_INTERVAL_MS: u64 = 60_000;
const FLEX_HOME: &str = "https://flex.team/home";

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

fn ui_pref_path() -> PathBuf {
    session_dir().join("ui-pref.json")
}

/// 항상 위 표시와 배경 흐림 설정을 함께 담는다. 없거나 깨졌으면 빈 값(모두 기본값)이다.
fn load_ui_pref() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(ui_pref_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// 한 키만 바꿔 쓴다. 통째로 덮으면 다른 설정이 지워진다. 값이 같으면 쓰지 않는다
/// (위젯 부팅이 저장값을 그대로 다시 넘기고, 흐림 반경 슬라이더는 움직이는 동안 계속 부른다).
fn save_ui_pref(key: &str, value: serde_json::Value) -> Result<(), String> {
    let mut pref = load_ui_pref();
    if pref.get(key) == Some(&value) {
        return Ok(());
    }
    pref.insert(key.to_string(), value);
    let text = serde_json::to_string_pretty(&pref).map_err(|e| e.to_string())?;
    std::fs::write(ui_pref_path(), text).map_err(|e| e.to_string())
}

/// Backend owns this native window flag so the two webviews can't disagree and
/// the tauri.conf default can't re-force it on. Shipped on → default true.
fn load_always_on_top_pref() -> bool {
    load_ui_pref()
        .get("alwaysOnTop")
        .and_then(|b| b.as_bool())
        .unwrap_or(true)
}

fn save_always_on_top_pref(enabled: bool) -> Result<(), String> {
    save_ui_pref("alwaysOnTop", serde_json::json!(enabled))
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

/// 배경 흐림 저장값. 항상 위 표시처럼 백엔드(ui-pref.json)가 가진다.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GlassPref {
    enabled: bool,
    blur: f64,
}

/// 저장값이 없으면 켬·12px. 지원하지 않는 Windows에서는 glass::apply가 알아서 끈다.
#[tauri::command]
fn get_glass_pref() -> GlassPref {
    let pref = load_ui_pref();
    GlassPref {
        enabled: pref.get("glass").and_then(|b| b.as_bool()).unwrap_or(true),
        blur: pref
            .get("glassBlur")
            .and_then(|px| px.as_f64())
            .map_or(glass::DEFAULT_BLUR_PX, |px| px.clamp(0.0, glass::MAX_BLUR_PX)),
    }
}

/// 저장하고 위젯 창에 반영한다. 위젯 부팅과 설정 창이 함께 쓴다.
/// 실제로 켜졌는지 돌려주고 위젯 창에도 알려, 카드 투명도를 효과 상태에 맞춘다.
#[tauri::command]
fn set_glass(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window not found".to_string())?;
    save_ui_pref("glass", serde_json::json!(enabled))?;
    let applied = glass::apply(&window, enabled);
    window
        .emit("glass-applied", applied)
        .map_err(|e| e.to_string())?;
    Ok(applied)
}

/// 흐림 반경(CSS px). 저장하고 캡처 여백을 맞춘 뒤, 위젯 창이 CSS blur 반경을 바꾸도록 알린다.
#[tauri::command]
fn set_glass_blur(app: AppHandle, px: f64) -> Result<(), String> {
    let px = px.clamp(0.0, glass::MAX_BLUR_PX);
    save_ui_pref("glassBlur", serde_json::json!(px))?;
    glass::set_blur(px);
    app.emit_to("main", "glass-blur", px)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn glass_supported() -> bool {
    glass::supported()
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

/// 블로킹 작업(HTTP, Chrome 실행)을 메인 스레드 밖에서 돌린다.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

/// 위젯 전용 Chrome 창을 띄워 사용자가 직접 로그인하게 하고, 확인되면 세션을 봉인 저장한다.
#[tauri::command]
async fn login_with_browser(
    app: AppHandle,
    store: State<'_, Arc<SessionStore>>,
) -> Result<WorkSnapshot, String> {
    let store = store.inner().clone();
    let emitter = app.clone();
    let snap = blocking(move || {
        let progress = |msg: &str| {
            let _ = emitter.emit("login-progress", msg);
        };
        let session = browser_login::login(&FLEX_LOGIN, &progress).map_err(|e| e.to_string())?;
        store.save(session)?;
        Ok::<_, String>(fetch_work(&store))
    })
    .await??;
    let _ = app.emit("work-updated", &snap);
    Ok(snap)
}

/// 설정 창 상태 줄용. 쿠키 값은 내보내지 않는다.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginInfo {
    connected: bool,
    account: Option<String>,
    saved_at_ms: Option<i64>,
    expires_at_ms: Option<i64>,
    has_profile: bool,
    browser_found: bool,
}

#[tauri::command]
async fn get_login_info(store: State<'_, Arc<SessionStore>>) -> Result<LoginInfo, String> {
    let session = store.get();
    blocking(move || LoginInfo {
        connected: session.is_some(),
        account: session
            .as_ref()
            .and_then(|s| s.account.as_ref())
            .and_then(|a| a.label.clone()),
        saved_at_ms: session.as_ref().map(|s| s.saved_at_ms),
        expires_at_ms: session
            .as_ref()
            .and_then(|s| s.expires_at_ms(FLEX_LOGIN.key_cookie)),
        has_profile: browser_login::profile_dir(&FLEX_LOGIN).is_dir(),
        browser_found: browser_login::find_browser(&FLEX_LOGIN).is_some(),
    })
    .await
}

/// 저장 세션만 지운다. 전용 프로필의 로그인은 남아 다음 로그인이 대개 클릭 한 번이다.
#[tauri::command]
async fn disconnect_session(
    app: AppHandle,
    store: State<'_, Arc<SessionStore>>,
) -> Result<WorkSnapshot, String> {
    store.clear()?;
    let snap = need_login(LOGIN_HINT.into());
    let _ = app.emit("work-updated", &snap);
    Ok(snap)
}

/// 전용 프로필 폴더와 저장 세션을 모두 지운다.
#[tauri::command]
async fn reset_login_profile(
    app: AppHandle,
    store: State<'_, Arc<SessionStore>>,
) -> Result<WorkSnapshot, String> {
    blocking(|| browser_login::reset_profile(&FLEX_LOGIN)).await??;
    store.clear()?;
    let snap = need_login(LOGIN_HINT.into());
    let _ = app.emit("work-updated", &snap);
    Ok(snap)
}

#[tauri::command]
async fn get_work(store: State<'_, Arc<SessionStore>>) -> Result<WorkSnapshot, String> {
    let store = store.inner().clone();
    blocking(move || fetch_work_with_refresh(&store)).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    guard_debug_requires_vite();

    let _ = std::fs::create_dir_all(session_dir());
    let store = Arc::new(SessionStore::new());

    tauri::Builder::default()
        // 위치만 복원한다. 창 크기는 메시지 표시 여부에 따라 프런트가 매번 정하므로
        // (COMPACT ↔ MESSAGE), 크기까지 저장하면 지난 실행 상태가 이번 실행을 덮어쓴다.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(StateFlags::POSITION)
                .with_denylist(&[SETTINGS_LABEL])
                .build(),
        )
        // 배경 흐림 캡처 프레임. 위젯 페이지가 http://glassbg.localhost/frame 으로 받아 간다.
        .register_uri_scheme_protocol("glassbg", |ctx, request| {
            glass::frame_response(ctx.webview_label(), request)
        })
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
            open_flex_home,
            login_with_browser,
            get_login_info,
            disconnect_session,
            reset_login_profile,
            set_always_on_top,
            get_always_on_top,
            get_glass_pref,
            set_glass,
            set_glass_blur,
            glass_supported
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" {
                // 배경 흐림은 창 뒤를 캡처하므로, 옮기거나 크기가 바뀌면(메시지 표시 때
                // COMPACT ↔ MESSAGE 포함) 바로 다시 찍는다.
                if matches!(event, WindowEvent::Moved(_) | WindowEvent::Resized(_)) {
                    glass::wake();
                }
                return;
            }
            if window.label() != SETTINGS_LABEL {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            tray::install(app)?;
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
