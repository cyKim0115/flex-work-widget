//! 위젯 전용 Chrome 프로필로 사용자가 직접 로그인하고, CDP로 그 창의 쿠키만 받아 봉인 저장한다.
//!
//! - 평소 Chrome 프로필은 읽지도 건드리지도 않는다. 프로필은 `%LOCALAPPDATA%\<app_dir>\login-profile\`.
//! - 브라우저 수준 CDP 명령(`Browser.getVersion`, `Storage.getCookies`, `Browser.close`)만 쓴다.
//!   페이지 타깃에 붙지 않는다(자동화 흔적 최소화).
//! - 쿠키 값·계정 정보는 로그·오류 문구에 넣지 않는다.
//!
//! 서비스별 값은 [`LoginSpec`] 하나로 넘긴다. 이 파일과 `secure_store.rs`는 claude-usage-widget 등이
//! 그대로 복사해 가는 기준 구현이다. 설계: rag `docs/assets/2026-10-12-widget-dedicated-profile-login-plan.md`.

use std::fs;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tungstenite::{Message, WebSocket};

use crate::secure_store;

/// 서비스별 설정. 다른 위젯은 이 값만 바꿔 쓴다.
pub struct LoginSpec {
    /// `%LOCALAPPDATA%` 아래 설치 폴더 이름. 프로필·세션 파일이 여기 생긴다. 위젯끼리 공유하지 않는다.
    pub app_dir: &'static str,
    /// 로그인 창이 처음 여는 주소.
    pub login_url: &'static str,
    /// 무인 갱신 때 headless로 여는 주소.
    pub home_url: &'static str,
    /// 이 도메인(과 하위 도메인) 쿠키만 저장한다.
    pub cookie_domain: &'static str,
    /// 이 중 하나라도 생기면 `validate`를 부른다.
    pub required_cookies: &'static [&'static str],
    /// 만료가 임박하면 무인 갱신을 시도하는 핵심 쿠키.
    pub key_cookie: &'static str,
    /// 핵심 쿠키 만료까지 이만큼 남으면 무인 갱신을 시도한다.
    pub refresh_margin: Duration,
    /// 시험용 브라우저 경로 override 환경변수.
    pub browser_env: &'static str,
    /// 쿠키가 평문 HTTP에서 실제로 통하는지 확인한다. 오류 문구에 쿠키 값·응답 본문을 넣지 않는다.
    pub validate: fn(&[Cookie]) -> Result<Account, String>,
}

const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const LOGIN_POLL: Duration = Duration::from_millis(1500);
const PROGRESS_EVERY: Duration = Duration::from_secs(15);
const PORT_FILE_TIMEOUT: Duration = Duration::from_secs(15);
const REFRESH_TIMEOUT: Duration = Duration::from_secs(20);
/// headless 홈이 로드되며 쿠키를 회전시킬 틈.
const REFRESH_SETTLE: Duration = Duration::from_secs(4);
const REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(30 * 60);
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// 브라우저 지문에 묶여 있어 옮기면 오히려 막힌다. 각 브라우저가 스스로 받게 둔다.
const CLOUDFLARE_COOKIES: &[&str] = &["cf_clearance", "__cf_bm", "_cfuvid", "__cflb"];
/// 설치 버전을 못 읽었을 때의 UA 메이저 버전.
const FALLBACK_CHROME_MAJOR: &str = "140";

/// 같은 프로필로 Chrome을 동시에 띄울 수 없으므로 로그인·갱신을 직렬화한다.
static BROWSER_LOCK: Mutex<()> = Mutex::new(());
static LOGIN_ACTIVE: AtomicBool = AtomicBool::new(false);
static LAST_REFRESH: Mutex<Option<Instant>> = Mutex::new(None);
/// 만료 임박 갱신을 이미 시도한 핵심 쿠키 만료값. 갱신이 만료를 못 밀면 같은 값으로 반복하지 않는다.
static PROACTIVE_TRIED: Mutex<Option<i64>> = Mutex::new(None);

#[derive(Debug, Error)]
pub enum LoginError {
    #[error("Chrome 또는 Edge를 찾지 못했습니다")]
    BrowserNotFound,
    #[error("로그인 창이 이미 떠 있습니다. 그 창을 닫고 다시 시도하세요")]
    ProfileBusy,
    #[error("이미 로그인 또는 갱신을 진행 중입니다")]
    Busy,
    #[error("브라우저 창이 닫혔습니다")]
    WindowClosed,
    #[error("로그인 대기 시간(5분)이 지났습니다")]
    Timeout,
    #[error("로그인한 적이 없습니다")]
    NoProfile,
    #[error("무인 갱신은 30분에 한 번만 시도합니다")]
    Throttled,
    #[error("로그인 확인 실패: {0}")]
    Invalid(String),
    #[error("브라우저 실행 실패: {0}")]
    Launch(String),
    #[error("브라우저 연결 실패: {0}")]
    Cdp(String),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default = "root_path")]
    pub path: String,
    /// Unix 초. session 쿠키는 -1.
    #[serde(default)]
    pub expires: f64,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub session: bool,
}

fn root_path() -> String {
    "/".into()
}

/// 값이 로그에 새지 않게 Debug에서 가린다.
impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("expires", &self.expires)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// 설정 창 표시용 계정 정보. 로그에 남기지 않는다.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub label: Option<String>,
    /// 서비스 API가 쓰는 사용자 식별자(flex: userIdHash).
    pub user_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSession {
    pub cookies: Vec<Cookie>,
    pub saved_at_ms: i64,
    #[serde(default)]
    pub account: Option<Account>,
}

/// 이름이 같은 첫 쿠키의 값. 빈 값은 없는 것으로 본다.
pub fn cookie_value<'a>(cookies: &'a [Cookie], name: &str) -> Option<&'a str> {
    cookies
        .iter()
        .find(|c| c.name == name && !c.value.is_empty())
        .map(|c| c.value.as_str())
}

impl StoredSession {
    /// 쿠키 만료 시각(ms). 없거나 session 쿠키면 `None`.
    pub fn expires_at_ms(&self, name: &str) -> Option<i64> {
        self.cookies
            .iter()
            .find(|c| c.name == name)
            .filter(|c| !c.session && c.expires > 0.0)
            .map(|c| (c.expires * 1000.0) as i64)
    }
}

// ---------------------------------------------------------------- 경로·저장

pub fn app_dir(spec: &LoginSpec) -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(spec.app_dir)
}

pub fn profile_dir(spec: &LoginSpec) -> PathBuf {
    app_dir(spec).join("login-profile")
}

pub fn session_path(spec: &LoginSpec) -> PathBuf {
    app_dir(spec).join("session.dat")
}

/// 봉인 세션을 읽는다. 다른 Windows 계정·손상·형식 불일치는 `None`(로그인 필요).
pub fn load_session(spec: &LoginSpec) -> Option<StoredSession> {
    let plain = secure_store::read_sealed(&session_path(spec))?;
    serde_json::from_slice(&plain).ok()
}

pub fn save_session(spec: &LoginSpec, session: &StoredSession) -> Result<(), String> {
    let plain = serde_json::to_vec(session).map_err(|e| e.to_string())?;
    secure_store::write_sealed(&session_path(spec), &plain)
}

/// "연결 해제": 저장 세션만 지운다. 프로필 안의 로그인은 남는다.
pub fn clear_session(spec: &LoginSpec) -> Result<(), String> {
    secure_store::delete(&session_path(spec))
}

/// "로그인 프로필 초기화": 프로필 폴더와 저장 세션을 모두 지운다(프로필에도 살아 있는 쿠키가 있다).
pub fn reset_profile(spec: &LoginSpec) -> Result<(), String> {
    let _guard = BROWSER_LOCK
        .try_lock()
        .map_err(|_| LoginError::Busy.to_string())?;
    let profile = profile_dir(spec);
    if profile_in_use(&profile) {
        return Err(LoginError::ProfileBusy.to_string());
    }
    clear_session(spec)?;
    match fs::remove_dir_all(&profile) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("로그인 프로필 삭제 실패: {e}")),
    }
}

// ---------------------------------------------------------------- 로그인 (창 표시)

/// 로그인 창을 띄우고 사용자가 로그인을 끝낼 때까지 기다린다. 성공하면 창을 닫고 세션을 돌려준다(저장은 호출 쪽).
/// 오래 걸리므로 비동기 런타임의 블로킹 스레드에서 부른다.
pub fn login(spec: &LoginSpec, progress: &dyn Fn(&str)) -> Result<StoredSession, LoginError> {
    if LOGIN_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err(LoginError::Busy);
    }
    let result = {
        // 진행 중인 무인 갱신(최대 수십 초)이 끝나길 기다린다.
        let _guard = BROWSER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        login_locked(spec, progress)
    };
    LOGIN_ACTIVE.store(false, Ordering::SeqCst);
    result
}

fn login_locked(spec: &LoginSpec, progress: &dyn Fn(&str)) -> Result<StoredSession, LoginError> {
    let profile = profile_dir(spec);
    fs::create_dir_all(&profile).map_err(|e| LoginError::Launch(e.to_string()))?;

    let (mut browser, mut cdp) = if profile_in_use(&profile) {
        // 이전 로그인 창이 아직 떠 있으면 새로 띄우지 않고 그 포트에 붙는다.
        let cdp = read_port_file(&profile)
            .and_then(|(port, path)| Cdp::connect(port, &path).ok())
            .ok_or(LoginError::ProfileBusy)?;
        (LaunchedBrowser(None), cdp)
    } else {
        let exe = find_browser(spec).ok_or(LoginError::BrowserNotFound)?;
        let args = vec![
            "--remote-debugging-port=0".to_string(),
            "--new-window".to_string(),
            spec.login_url.to_string(),
        ];
        let mut browser = LaunchedBrowser(Some(launch(&exe, &profile, &args)?));
        let cdp = browser.connect(&profile)?;
        (browser, cdp)
    };

    progress("브라우저 창에서 로그인하세요. 로그인이 확인되면 창이 저절로 닫힙니다.");
    let started = Instant::now();
    let mut last_note = Instant::now();
    let mut last_error: Option<String> = None;
    while started.elapsed() < LOGIN_TIMEOUT {
        if browser.exited() {
            return Err(LoginError::WindowClosed);
        }
        let cookies = match cdp.cookies(spec.cookie_domain) {
            Ok(c) => c,
            Err(CdpError::Closed) => return Err(LoginError::WindowClosed),
            Err(CdpError::Other(_)) => {
                thread::sleep(LOGIN_POLL);
                continue;
            }
        };
        if has_required(spec, &cookies) {
            match (spec.validate)(&cookies) {
                Ok(account) => {
                    let session = StoredSession {
                        cookies,
                        saved_at_ms: now_ms(),
                        account: Some(account),
                    };
                    cdp.close_browser();
                    browser.wait_or_kill();
                    return Ok(session);
                }
                Err(e) => last_error = Some(e),
            }
        }
        if last_note.elapsed() >= PROGRESS_EVERY {
            last_note = Instant::now();
            let left = LOGIN_TIMEOUT.saturating_sub(started.elapsed()).as_secs();
            progress(&format!(
                "로그인을 기다리는 중… 남은 시간 {}:{:02}",
                left / 60,
                left % 60
            ));
        }
        thread::sleep(LOGIN_POLL);
    }
    cdp.close_browser();
    browser.wait_or_kill();
    match last_error {
        Some(e) => Err(LoginError::Invalid(e)),
        None => Err(LoginError::Timeout),
    }
}

// ---------------------------------------------------------------- 무인 갱신 (headless)

/// 핵심 쿠키 만료가 `refresh_margin` 안으로 들어왔는지.
pub fn key_cookie_expiring(spec: &LoginSpec, session: &StoredSession, now_ms: i64) -> bool {
    match session.expires_at_ms(spec.key_cookie) {
        Some(exp) => exp - now_ms <= spec.refresh_margin.as_millis() as i64,
        None => false,
    }
}

/// 만료 임박 시 무인 갱신을 한 번 시도한다. 같은 만료값으로는 다시 시도하지 않는다
/// (서비스가 홈 방문으로 만료를 밀어주지 않으면 매번 헛수고다). 갱신 결과가 없으면 `None`.
pub fn refresh_if_expiring(spec: &LoginSpec, session: &StoredSession) -> Option<StoredSession> {
    if !key_cookie_expiring(spec, session, now_ms()) {
        return None;
    }
    let exp = session.expires_at_ms(spec.key_cookie);
    {
        let mut tried = PROACTIVE_TRIED.lock().unwrap_or_else(|e| e.into_inner());
        if *tried == exp {
            return None;
        }
        *tried = exp;
    }
    refresh_headless(spec).ok()
}

/// 같은 프로필을 화면 없이 띄워 쿠키를 다시 읽는다. 30분에 한 번 이하, 로그인 창이 떠 있으면 하지 않는다.
pub fn refresh_headless(spec: &LoginSpec) -> Result<StoredSession, LoginError> {
    if LOGIN_ACTIVE.load(Ordering::SeqCst) {
        return Err(LoginError::Busy);
    }
    let _guard = BROWSER_LOCK.try_lock().map_err(|_| LoginError::Busy)?;
    {
        let mut last = LAST_REFRESH.lock().unwrap_or_else(|e| e.into_inner());
        if last.is_some_and(|t| t.elapsed() < REFRESH_MIN_INTERVAL) {
            return Err(LoginError::Throttled);
        }
        *last = Some(Instant::now());
    }

    let profile = profile_dir(spec);
    if !profile.is_dir() {
        return Err(LoginError::NoProfile);
    }
    if profile_in_use(&profile) {
        return Err(LoginError::ProfileBusy);
    }
    let exe = find_browser(spec).ok_or(LoginError::BrowserNotFound)?;
    let version = chrome_version(&profile, &exe);
    let args = vec![
        "--headless=new".to_string(),
        "--remote-debugging-port=0".to_string(),
        format!(
            "--user-agent={}",
            regular_user_agent(version.as_deref().unwrap_or(FALLBACK_CHROME_MAJOR))
        ),
        spec.home_url.to_string(),
    ];
    let mut browser = LaunchedBrowser(Some(launch(&exe, &profile, &args)?));
    let mut cdp = browser.connect(&profile)?;

    let started = Instant::now();
    let mut last_error: Option<String> = None;
    let result = loop {
        if started.elapsed() >= REFRESH_TIMEOUT {
            break Err(last_error.map_or(LoginError::Timeout, LoginError::Invalid));
        }
        thread::sleep(Duration::from_secs(1));
        if started.elapsed() < REFRESH_SETTLE {
            continue;
        }
        let cookies = match cdp.cookies(spec.cookie_domain) {
            Ok(c) => c,
            Err(CdpError::Closed) => break Err(LoginError::Cdp("연결이 끊겼습니다".into())),
            Err(CdpError::Other(e)) => {
                last_error = Some(format!("브라우저 응답 오류: {e}"));
                continue;
            }
        };
        if !has_required(spec, &cookies) {
            last_error = Some("로그인 쿠키가 없습니다".into());
            continue;
        }
        match (spec.validate)(&cookies) {
            Ok(account) => {
                break Ok(StoredSession {
                    cookies,
                    saved_at_ms: now_ms(),
                    account: Some(account),
                })
            }
            Err(e) => last_error = Some(e),
        }
    };
    cdp.close_browser();
    browser.wait_or_kill();
    result
}

// ---------------------------------------------------------------- 브라우저 찾기·실행

/// 우리가 띄운 Chrome 프로세스. 정상 종료를 기다리고, 안 닫히면 그것만 끝낸다.
struct LaunchedBrowser(Option<Child>);

impl LaunchedBrowser {
    fn exited(&mut self) -> bool {
        match self.0.as_mut() {
            Some(child) => !matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    fn connect(&mut self, profile: &Path) -> Result<Cdp, LoginError> {
        let started = Instant::now();
        let (port, path) = loop {
            if let Some(found) = read_port_file(profile) {
                break found;
            }
            if self.exited() {
                return Err(LoginError::Launch(
                    "브라우저가 바로 종료됐습니다(같은 프로필 창이 떠 있는지 확인)".into(),
                ));
            }
            if started.elapsed() >= PORT_FILE_TIMEOUT {
                self.kill();
                return Err(LoginError::Cdp("DevToolsActivePort가 생기지 않았습니다".into()));
            }
            thread::sleep(Duration::from_millis(100));
        };
        Cdp::connect(port, &path).map_err(|e| {
            self.kill();
            LoginError::Cdp(e)
        })
    }

    fn wait_or_kill(&mut self) {
        let started = Instant::now();
        while started.elapsed() < CLOSE_WAIT {
            if self.0.is_none() || self.exited() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        self.kill();
    }

    fn kill(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn launch(exe: &Path, profile: &Path, extra: &[String]) -> Result<Child, LoginError> {
    // 오래된 포트 파일을 남기면 죽은 포트에 붙으려 한다.
    let _ = fs::remove_file(profile.join("DevToolsActivePort"));
    let mut cmd = Command::new(exe);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .args(extra);
    cmd.spawn().map_err(|e| LoginError::Launch(e.to_string()))
}

/// 같은 프로필 Chrome이 떠 있는지. 실행 중인 Chrome은 `lockfile`을 열어 둬서 지울 수 없다.
fn profile_in_use(profile: &Path) -> bool {
    match fs::remove_file(profile.join("lockfile")) {
        Ok(()) => false,
        Err(e) => e.kind() != ErrorKind::NotFound,
    }
}

fn read_port_file(profile: &Path) -> Option<(u16, String)> {
    parse_devtools_active_port(&fs::read_to_string(profile.join("DevToolsActivePort")).ok()?)
}

/// `DevToolsActivePort`: 1행 포트, 2행 `/devtools/browser/<id>`.
fn parse_devtools_active_port(text: &str) -> Option<(u16, String)> {
    let mut lines = text.lines().map(str::trim);
    let port: u16 = lines.next()?.parse().ok().filter(|p| *p != 0)?;
    let path = lines.next()?;
    path.starts_with("/devtools/browser/")
        .then(|| (port, path.to_string()))
}

pub fn find_browser(spec: &LoginSpec) -> Option<PathBuf> {
    let env_override = std::env::var_os(spec.browser_env).map(PathBuf::from);
    let roots = Roots {
        program_files: std::env::var_os("ProgramFiles").map(PathBuf::from),
        program_files_x86: std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    };
    browser_candidates(env_override, registry_app_paths, &roots)
        .into_iter()
        .find(|p| p.is_file())
}

struct Roots {
    program_files: Option<PathBuf>,
    program_files_x86: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
}

/// 탐색 순서: 환경변수 → Chrome(App Paths HKCU→HKLM, 설치 폴더) → Edge(같은 순서).
fn browser_candidates(
    env_override: Option<PathBuf>,
    app_paths: impl Fn(&str) -> Vec<PathBuf>,
    roots: &Roots,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = env_override.into_iter().collect();
    let installs = [
        ("chrome.exe", r"Google\Chrome\Application\chrome.exe"),
        ("msedge.exe", r"Microsoft\Edge\Application\msedge.exe"),
    ];
    for (exe, rel) in installs {
        out.extend(app_paths(exe));
        for root in [&roots.program_files, &roots.program_files_x86, &roots.local_app_data]
            .into_iter()
            .flatten()
        {
            out.push(root.join(rel));
        }
    }
    out
}

#[cfg(windows)]
fn registry_app_paths(exe: &str) -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let sub = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .filter_map(|hive| RegKey::predef(hive).open_subkey(&sub).ok())
        .filter_map(|key| key.get_value::<String, _>("").ok())
        .map(|v| PathBuf::from(v.trim().trim_matches('"')))
        .collect()
}

#[cfg(not(windows))]
fn registry_app_paths(_exe: &str) -> Vec<PathBuf> {
    Vec::new()
}

/// 설치된 브라우저 버전. 프로필의 `Last Version`(마지막으로 이 프로필을 연 버전) → 실행 파일 옆 버전 폴더.
fn chrome_version(profile: &Path, exe: &Path) -> Option<String> {
    if let Ok(v) = fs::read_to_string(profile.join("Last Version")) {
        if parse_version(v.trim()).is_some() {
            return Some(v.trim().to_string());
        }
    }
    let dir = exe.parent()?;
    let names = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok());
    newest_version(names)
}

fn parse_version(s: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = s.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| p.len() == 4)
}

fn newest_version(names: impl Iterator<Item = String>) -> Option<String> {
    names
        .filter_map(|n| parse_version(&n).map(|v| (v, n)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, n)| n)
}

/// headless UA에는 `HeadlessChrome`이 들어가 Cloudflare 등이 막는다. 같은 메이저의 일반 Chrome UA를 만든다.
pub fn regular_user_agent(version: &str) -> String {
    let major = version.split('.').next().unwrap_or(FALLBACK_CHROME_MAJOR);
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{major}.0.0.0 Safari/537.36"
    )
}

// ---------------------------------------------------------------- 쿠키

fn domain_matches(cookie_domain: &str, domain: &str) -> bool {
    let d = cookie_domain.trim_start_matches('.');
    d == domain || d.ends_with(&format!(".{domain}"))
}

/// 해당 도메인 쿠키만 남기고 Cloudflare 쿠키는 버린다.
pub fn filter_cookies(cookies: Vec<Cookie>, domain: &str) -> Vec<Cookie> {
    cookies
        .into_iter()
        .filter(|c| domain_matches(&c.domain, domain))
        .filter(|c| !CLOUDFLARE_COOKIES.contains(&c.name.as_str()))
        .collect()
}

fn has_required(spec: &LoginSpec, cookies: &[Cookie]) -> bool {
    cookies
        .iter()
        .any(|c| spec.required_cookies.contains(&c.name.as_str()) && !c.value.is_empty())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------- CDP (브라우저 웹소켓)

enum CdpError {
    /// 웹소켓이 끊김 = 사용자가 창을 닫았거나 브라우저가 종료됨.
    Closed,
    Other(String),
}

struct Cdp {
    ws: WebSocket<TcpStream>,
    next_id: u64,
}

impl Cdp {
    fn connect(port: u16, path: &str) -> Result<Self, String> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream =
            TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        let url = format!("ws://127.0.0.1:{port}{path}");
        let (ws, _) = tungstenite::client(url.as_str(), stream).map_err(|e| e.to_string())?;
        Ok(Self { ws, next_id: 0 })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, CdpError> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({ "id": id, "method": method, "params": params }).to_string();
        self.ws.send(Message::Text(msg.into())).map_err(map_ws_error)?;
        loop {
            match self.ws.read().map_err(map_ws_error)? {
                Message::Text(text) => {
                    let val: Value = serde_json::from_str(text.as_str())
                        .map_err(|e| CdpError::Other(e.to_string()))?;
                    if val.get("id").and_then(Value::as_u64) != Some(id) {
                        continue; // 이벤트 등 다른 메시지
                    }
                    if let Some(err) = val.get("error") {
                        let msg = err.get("message").and_then(Value::as_str).unwrap_or("error");
                        return Err(CdpError::Other(msg.to_string()));
                    }
                    return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                }
                Message::Close(_) => return Err(CdpError::Closed),
                _ => continue,
            }
        }
    }

    fn cookies(&mut self, domain: &str) -> Result<Vec<Cookie>, CdpError> {
        let result = self.call("Storage.getCookies", json!({}))?;
        let all: Vec<Cookie> = serde_json::from_value(result.get("cookies").cloned().unwrap_or_default())
            .map_err(|e| CdpError::Other(e.to_string()))?;
        Ok(filter_cookies(all, domain))
    }

    fn close_browser(&mut self) {
        let _ = self.call("Browser.close", json!({}));
    }
}

fn map_ws_error(e: tungstenite::Error) -> CdpError {
    match e {
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => CdpError::Closed,
        tungstenite::Error::Io(io)
            if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
        {
            CdpError::Other("응답 시간 초과".into())
        }
        tungstenite::Error::Io(_) | tungstenite::Error::Protocol(_) => CdpError::Closed,
        other => CdpError::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: "v".into(),
            domain: domain.into(),
            path: "/".into(),
            expires: -1.0,
            http_only: false,
            secure: true,
            session: true,
        }
    }

    #[test]
    fn filter_keeps_domain_and_drops_cloudflare() {
        let cookies = vec![
            cookie("AID", "flex.team"),
            cookie("JSESSIONID", ".flex.team"),
            cookie("x", "app.flex.team"),
            cookie("cf_clearance", ".flex.team"),
            cookie("__cf_bm", "flex.team"),
            cookie("AID", "notflex.team"),
            cookie("SID", ".google.com"),
        ];
        let names: Vec<String> = filter_cookies(cookies, "flex.team")
            .into_iter()
            .map(|c| format!("{}@{}", c.name, c.domain))
            .collect();
        assert_eq!(names, ["AID@flex.team", "JSESSIONID@.flex.team", "x@app.flex.team"]);
    }

    #[test]
    fn parses_cdp_cookie_json() {
        let raw = r#"{"name":"AID","value":"secret","domain":"flex.team","path":"/","expires":1792381134.5,
            "size":9,"httpOnly":false,"secure":true,"session":false,"sameSite":"Lax","priority":"Medium"}"#;
        let c: Cookie = serde_json::from_str(raw).unwrap();
        assert!(c.secure && !c.session);
        let s = StoredSession {
            cookies: vec![c],
            saved_at_ms: 0,
            account: None,
        };
        assert_eq!(s.expires_at_ms("AID"), Some(1_792_381_134_500));
        assert_eq!(cookie_value(&s.cookies, "AID"), Some("secret"));
        assert!(!format!("{:?}", s.cookies[0]).contains("secret"));
    }

    #[test]
    fn parses_devtools_active_port() {
        assert_eq!(
            parse_devtools_active_port("51234\r\n/devtools/browser/abc-123\r\n"),
            Some((51234, "/devtools/browser/abc-123".into()))
        );
        assert_eq!(parse_devtools_active_port("51234\n"), None);
        assert_eq!(parse_devtools_active_port("0\n/devtools/browser/x"), None);
        assert_eq!(parse_devtools_active_port("port\n/devtools/browser/x"), None);
        assert_eq!(parse_devtools_active_port("9222\n/json/version"), None);
    }

    #[test]
    fn builds_regular_user_agent() {
        let ua = regular_user_agent("154.0.8037.98");
        assert!(ua.contains("Chrome/154.0.0.0"));
        assert!(!ua.contains("Headless"));
        assert_eq!(
            newest_version(
                ["153.0.1.2", "154.0.8037.98", "154.0.10.1", "SetupMetrics", "1.2"]
                    .into_iter()
                    .map(String::from)
            ),
            Some("154.0.8037.98".into())
        );
    }

    #[test]
    fn browser_search_order() {
        let roots = Roots {
            program_files: Some(PathBuf::from(r"C:\PF")),
            program_files_x86: None,
            local_app_data: Some(PathBuf::from(r"C:\L")),
        };
        let reg = |exe: &str| vec![PathBuf::from(format!(r"C:\Reg\{exe}"))];
        let list = browser_candidates(Some(PathBuf::from(r"C:\env\chrome.exe")), reg, &roots);
        let expect: Vec<PathBuf> = [
            r"C:\env\chrome.exe",
            r"C:\Reg\chrome.exe",
            r"C:\PF\Google\Chrome\Application\chrome.exe",
            r"C:\L\Google\Chrome\Application\chrome.exe",
            r"C:\Reg\msedge.exe",
            r"C:\PF\Microsoft\Edge\Application\msedge.exe",
            r"C:\L\Microsoft\Edge\Application\msedge.exe",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(list, expect);
        assert_eq!(browser_candidates(None, |_| Vec::new(), &roots).len(), 4);
    }

    #[test]
    fn key_cookie_margin() {
        let spec = LoginSpec {
            app_dir: "Test",
            login_url: "",
            home_url: "",
            cookie_domain: "flex.team",
            required_cookies: &["AID"],
            key_cookie: "AID",
            refresh_margin: Duration::from_secs(3600),
            browser_env: "TEST_BROWSER",
            validate: |_| Ok(Account::default()),
        };
        let mut c = cookie("AID", "flex.team");
        c.session = false;
        c.expires = 10_000.0;
        let s = StoredSession {
            cookies: vec![c],
            saved_at_ms: 0,
            account: None,
        };
        assert!(!key_cookie_expiring(&spec, &s, 10_000_000 - 3_600_001));
        assert!(key_cookie_expiring(&spec, &s, 10_000_000 - 3_600_000));
        assert!(has_required(&spec, &s.cookies));
    }
}
