mod work;

use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use work::{
    fetch_work, need_login, session_dir, tokens_from_cookie_header, tokens_from_parts, SessionStore,
    WorkSnapshot,
};

const POLL_INTERVAL_MS: u64 = 60_000;
const FLEX_HOME: &str = "https://flex.team/home";
const FLEX_LOGIN: &str = "https://flex.team/auth/login?nextUrl=%2Fhome";
const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[tauri::command]
fn get_poll_interval_ms() -> u64 {
    POLL_INTERVAL_MS
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn open_flex_home(_app: AppHandle) -> Result<(), String> {
    open::that(FLEX_HOME).map_err(|e| e.to_string())
}

/// Prefer the system browser — Google OAuth often shows a blank page inside WebView2.
#[tauri::command]
fn open_login_system() -> Result<(), String> {
    open::that(FLEX_LOGIN).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_login(app: AppHandle) -> Result<(), String> {
    open_or_focus_login_webview(&app, FLEX_LOGIN)
}

fn is_app_url(url: &str) -> bool {
    if url.contains("/auth/login") {
        return false;
    }
    url.contains("flex.team/home")
        || url.contains("flex.team/time-tracking")
        || url.contains("flex.team/main")
        || (url.contains("://flex.team/") && !url.contains("/auth/"))
}

fn open_or_focus_login_webview(app: &AppHandle, url: &str) -> Result<(), String> {
    let parsed = url
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;

    if let Some(win) = app.get_webview_window("login") {
        let _ = win.show();
        let _ = win.set_focus();
        let js = format!(
            "window.location.replace({})",
            serde_json::to_string(url).unwrap()
        );
        let _ = win.eval(&js);
        return Ok(());
    }

    // Fresh profile each session avoids a corrupted WebView2 data dir blanking the page.
    let data_dir = session_dir().join("webview-login");
    let _ = std::fs::remove_dir_all(&data_dir);
    std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;

    let builder_app = app.clone();
    let tip = concat!(
        "(function(){",
        "try{",
        "if(document.getElementById('flex-work-widget-tip'))return;",
        "var b=document.createElement('div');",
        "b.id='flex-work-widget-tip';",
        "b.style.cssText='position:fixed;z-index:2147483647;left:12px;right:12px;bottom:12px;",
        "padding:12px 14px;border-radius:12px;font:13px/1.4 Segoe UI,sans-serif;",
        "background:#111827;color:#f9fafb;box-shadow:0 10px 30px rgba(0,0,0,.35)';",
        "b.innerHTML='<b>Flex Work Widget</b><br/>Google 로그인이 하얀 화면이면 ",
        "<b>이메일 로그인</b>을 쓰거나, 위젯 메뉴의 <b>시스템 브라우저로 로그인</b>을 사용하세요. ",
        "홈 화면까지 들어가면 세션을 자동으로 가져옵니다.';",
        "document.documentElement.appendChild(b);",
        "setTimeout(function(){b.remove()},16000);",
        "}catch(e){}",
        "})();"
    );

    WebviewWindowBuilder::new(app, "login", WebviewUrl::External(parsed))
        .title("flex 로그인")
        .inner_size(1100.0, 780.0)
        .min_inner_size(800.0, 600.0)
        .resizable(true)
        .maximizable(true)
        .decorations(true)
        .transparent(false)
        .always_on_top(false)
        .skip_taskbar(false)
        .focused(true)
        .visible(true)
        .user_agent(CHROME_UA)
        .data_directory(data_dir)
        .initialization_script(tip)
        .on_page_load(move |window, payload| {
            let url = payload.url().to_string();
            // Keep tip visible on login; harvest when we reach the app shell.
            if is_app_url(&url) {
                schedule_cookie_harvest(builder_app.clone(), window);
            }
        })
        .build()
        .map_err(|e| e.to_string())?;

    Ok(())
}

fn schedule_cookie_harvest(app: AppHandle, window: WebviewWindow) {
    thread::spawn(move || {
        // Give auth cookies a moment to settle after redirect.
        thread::sleep(Duration::from_millis(900));
        let Ok(url) = "https://flex.team/".parse() else {
            return;
        };
        let cookies = match window.cookies_for_url(url) {
            Ok(c) => c,
            Err(e) => {
                let snap = need_login(format!("쿠키 읽기 실패: {e}"));
                let _ = app.emit("work-updated", &snap);
                return;
            }
        };

        let mut aid = String::new();
        let mut ws_aid = String::new();
        let mut ws_rid = String::new();
        let mut header = String::new();

        for c in cookies {
            let name = c.name();
            let value = c.value();
            if !header.is_empty() {
                header.push_str("; ");
            }
            header.push_str(&format!("{name}={value}"));
            match name {
                "AID" => aid = value.to_string(),
                "V2_WS_AID" => ws_aid = value.to_string(),
                "V2_WS_RID" => ws_rid = value.to_string(),
                _ => {}
            }
        }

        if aid.is_empty() && ws_aid.is_empty() {
            let parsed = tokens_from_cookie_header(&header);
            aid = parsed.aid;
            ws_aid = parsed.ws_aid;
            ws_rid = parsed.ws_rid;
        }

        if aid.is_empty() && ws_aid.is_empty() {
            let snap = need_login("로그인 쿠키(AID/V2_WS_AID)를 찾지 못했습니다".into());
            let _ = app.emit("work-updated", &snap);
            return;
        }

        let store = app.state::<SessionStore>();
        let tokens = tokens_from_parts(aid, ws_aid, ws_rid);
        if let Err(e) = store.save(tokens) {
            let snap = need_login(format!("세션 저장 실패: {e}"));
            let _ = app.emit("work-updated", &snap);
            return;
        }

        let snap = fetch_work(&store);
        let _ = app.emit("work-updated", &snap);
        if snap.state != "NeedLogin" && snap.state != "FetchError" {
            let _ = window.close();
        }
    });
}

#[tauri::command]
async fn harvest_login_cookies(
    app: AppHandle,
    store: State<'_, SessionStore>,
) -> Result<WorkSnapshot, String> {
    let Some(window) = app.get_webview_window("login") else {
        return Ok(need_login(
            "로그인 창이 없습니다. 먼저「flex 로그인(앱)」또는 시스템 브라우저 로그인 후 앱 로그인 창에서 홈까지 이동하세요."
                .into(),
        ));
    };

    let url = "https://flex.team/"
        .parse()
        .map_err(|e: url::ParseError| e.to_string())?;

    let cookies = window.cookies_for_url(url).map_err(|e| e.to_string())?;

    let mut aid = String::new();
    let mut ws_aid = String::new();
    let mut ws_rid = String::new();
    for c in cookies {
        match c.name() {
            "AID" => aid = c.value().to_string(),
            "V2_WS_AID" => ws_aid = c.value().to_string(),
            "V2_WS_RID" => ws_rid = c.value().to_string(),
            _ => {}
        }
    }

    if aid.is_empty() && ws_aid.is_empty() {
        return Ok(need_login(
            "로그인 쿠키를 찾지 못했습니다. 앱 로그인 창에서 flex 홈까지 이동했는지 확인하세요."
                .into(),
        ));
    }

    store.save(tokens_from_parts(aid, ws_aid, ws_rid))?;
    let snap = fetch_work(&store);
    let _ = app.emit("work-updated", &snap);
    if snap.state != "NeedLogin" && snap.state != "FetchError" {
        let _ = window.close();
    }
    Ok(snap)
}

#[tauri::command]
fn get_work(_app: AppHandle, store: State<'_, SessionStore>) -> WorkSnapshot {
    match store.get() {
        None => need_login("우클릭 → 앱에서 로그인".into()),
        Some(_) => fetch_work(&store),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let store = SessionStore::new(session_dir());

    tauri::Builder::default()
        .manage(store)
        .invoke_handler(tauri::generate_handler![
            get_work,
            get_poll_interval_ms,
            quit_app,
            open_login,
            open_login_system,
            open_flex_home,
            harvest_login_cookies
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
