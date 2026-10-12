use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::browser_login::{self, cookie_value, Account, Cookie, LoginSpec, StoredSession};

const FLEX_ORIGIN: &str = "https://flex.team";
const ME_PATH: &str = "/api/v2/workspace/users/me/workspace-users";
const STATUS_PATH: &str = "/api/v2/time-tracking/work-clock/users/{userIdHash}/current-status";
/// 요청에 싣는 쿠키. 실측(2026-10-12): 인증은 AID(7일)로 되고 V2_WS_AID(12시간) 단독은 401.
/// V2_WS_RID 는 새 로그인에서 생기지 않았지만 있으면 예전처럼 함께 보낸다.
const AUTH_COOKIES: &[&str] = &["AID", "V2_WS_AID", "V2_WS_RID"];
pub const LOGIN_HINT: &str = "우클릭 → 설정 → 브라우저로 로그인";

/// flex 전용 로그인 설정. 공통 로그인 모듈(browser_login.rs)에 넘기는 서비스별 값은 여기뿐이다.
pub const FLEX_LOGIN: LoginSpec = LoginSpec {
    app_dir: "FlexWorkWidget",
    login_url: "https://flex.team/",
    home_url: "https://flex.team/home",
    cookie_domain: "flex.team",
    // 옛 하베스트 조건과 같다. 실제로 통하는지는 validate 가 가린다.
    required_cookies: &["AID", "V2_WS_AID"],
    key_cookie: "AID",
    refresh_margin: Duration::from_secs(24 * 60 * 60),
    browser_env: "FLEX_BROWSER_PATH",
    validate: validate_flex,
};

/// 옛 rookiepy 하베스트가 남긴 파일. 평문 쿠키·계정 이름이 들어 있어 새 세션 저장에 성공하면 지운다.
const LEGACY_FILES: &[&str] = &[
    "session.json",
    "candidates.json",
    "harvest-pref.json",
    "session-error.txt",
];

#[derive(Debug, Error)]
pub enum WorkError {
    #[error("NeedLogin: {0}")]
    NeedLogin(String),
    #[error("FetchError: {0}")]
    Fetch(String),
    #[error("FetchError: parse failed: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSnapshot {
    pub state: String,
    pub worked_seconds: Option<i64>,
    pub label: String,
    pub started_at: Option<String>,
    pub error: Option<String>,
    pub fetched_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurrentStatusResponse {
    #[serde(default)]
    work_clock_record_packs: Vec<WorkClockPack>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WorkClockPack {
    #[serde(default)]
    on_going: Option<bool>,
    #[serde(default)]
    applied_date: Option<String>,
    #[serde(default)]
    start_record: Option<ClockEvent>,
    #[serde(default)]
    stop_record: Option<ClockEvent>,
    #[serde(default)]
    switch_records: Vec<ClockEvent>,
    #[serde(default)]
    rest_records: Vec<RestRecord>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ClockEvent {
    #[serde(default)]
    real_time: Option<i64>,
    #[serde(default)]
    target_time: Option<i64>,
    #[serde(default)]
    zone_id: Option<String>,
    #[serde(default)]
    event_type: Option<String>,
    #[serde(default)]
    record_type: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RestRecord {
    #[serde(default)]
    rest_start_record: Option<ClockEvent>,
    #[serde(default)]
    rest_stop_record: Option<ClockEvent>,
}

/// DPAPI 봉인 세션(`session.dat`)의 메모리 사본. 디스크 쓰기는 `save`/`clear`에서만 한다.
pub struct SessionStore {
    session: Mutex<Option<StoredSession>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(browser_login::load_session(&FLEX_LOGIN)),
        }
    }

    pub fn get(&self) -> Option<StoredSession> {
        self.session.lock().ok().and_then(|g| g.clone())
    }

    pub fn save(&self, session: StoredSession) -> Result<(), String> {
        browser_login::save_session(&FLEX_LOGIN, &session)?;
        remove_legacy_files();
        if let Ok(mut guard) = self.session.lock() {
            *guard = Some(session);
        }
        Ok(())
    }

    /// "연결 해제". 다음 조회부터 `NeedLogin`.
    pub fn clear(&self) -> Result<(), String> {
        browser_login::clear_session(&FLEX_LOGIN)?;
        if let Ok(mut guard) = self.session.lock() {
            *guard = None;
        }
        Ok(())
    }
}

fn remove_legacy_files() {
    let dir = session_dir();
    for name in LEGACY_FILES {
        let _ = fs::remove_file(dir.join(name));
    }
}

pub fn session_dir() -> PathBuf {
    browser_login::app_dir(&FLEX_LOGIN)
}

fn cookie_header(cookies: &[Cookie]) -> String {
    AUTH_COOKIES
        .iter()
        .filter_map(|name| cookie_value(cookies, name).map(|v| format!("{name}={v}")))
        .collect::<Vec<_>>()
        .join("; ")
}

fn flex_aid(cookies: &[Cookie]) -> Option<&str> {
    cookie_value(cookies, "AID").or_else(|| cookie_value(cookies, "V2_WS_AID"))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        // 쿠키가 다른 호스트로 따라가지 않게 리디렉션을 따르지 않는다.
        .redirects(0)
        .build()
}

/// 오류 문구에는 상태 코드만 남긴다(응답 본문·쿠키 값 금지).
fn get_json(cookies: &[Cookie], path: &str) -> Result<serde_json::Value, WorkError> {
    let aid = flex_aid(cookies).ok_or_else(|| WorkError::NeedLogin("로그인 쿠키가 없습니다".into()))?;
    let url = format!("{FLEX_ORIGIN}{path}");
    let resp = agent()
        .get(&url)
        .set("Origin", FLEX_ORIGIN)
        .set("Referer", &format!("{FLEX_ORIGIN}/home"))
        .set("Accept", "application/json")
        .set("User-Agent", "flex-work-widget/0.1")
        .set("Cookie", &cookie_header(cookies))
        .set("x-flex-aid", aid)
        .set("FlexTeam-Version", "V2")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code @ (401 | 403), _) => {
                WorkError::NeedLogin(format!("인증이 만료됐습니다 (HTTP {code})"))
            }
            ureq::Error::Status(code, _) => WorkError::Fetch(format!("HTTP {code}")),
            ureq::Error::Transport(t) => WorkError::Fetch(t.kind().to_string()),
        })?;

    // 리디렉션은 로그인 페이지로 보내는 경우다.
    if (300..400).contains(&resp.status()) {
        return Err(WorkError::NeedLogin(format!(
            "인증이 만료됐습니다 (HTTP {})",
            resp.status()
        )));
    }
    resp.into_json::<serde_json::Value>()
        .map_err(|e| WorkError::Parse(e.to_string()))
}

/// `LoginSpec.validate`: workspace-users 200 + userIdHash. 계정 표시는 이름 · 회사(이메일은 쓰지 않는다).
fn validate_flex(cookies: &[Cookie]) -> Result<Account, String> {
    let val = get_json(cookies, ME_PATH).map_err(|e| e.to_string())?;
    let user_id = extract_user_id_hash(&val).ok_or("userIdHash를 찾지 못했습니다")?;
    Ok(Account {
        label: account_label(&val),
        user_id: Some(user_id),
    })
}

fn account_label(val: &serde_json::Value) -> Option<String> {
    let current = val.get("currentUser")?;
    let user = current.get("user");
    let name = user
        .and_then(|u| u.get("displayName").or_else(|| u.get("name")))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let company = current
        .get("customer")
        .and_then(|c| c.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    match (name, company) {
        (Some(n), Some(c)) => Some(format!("{n} · {c}")),
        (Some(n), None) => Some(n.to_string()),
        (None, Some(c)) => Some(c.to_string()),
        (None, None) => None,
    }
}

fn extract_user_id_hash(val: &serde_json::Value) -> Option<String> {
    // 현재 응답: { currentUser: { user: { userIdHash } }, users: [...] }
    if let Some(h) = val
        .pointer("/currentUser/user/userIdHash")
        .and_then(|v| v.as_str())
    {
        return Some(h.to_string());
    }
    // Common shapes: { workspaceUsers: [...] } or { data: [...] } or bare array
    let users = val
        .get("workspaceUsers")
        .or_else(|| val.get("workspace_users"))
        .or_else(|| val.get("data"))
        .cloned()
        .unwrap_or_else(|| val.clone());

    let arr = if let Some(a) = users.as_array() {
        a.clone()
    } else if let Some(a) = users.get("workspaceUsers").and_then(|v| v.as_array()) {
        a.clone()
    } else if let Some(a) = users.get("items").and_then(|v| v.as_array()) {
        a.clone()
    } else {
        Vec::new()
    };

    for u in &arr {
        if u.get("isCurrent").and_then(|v| v.as_bool()) == Some(true)
            || u.get("is_current").and_then(|v| v.as_bool()) == Some(true)
        {
            if let Some(h) = u
                .get("userIdHash")
                .or_else(|| u.get("user_id_hash"))
                .or_else(|| u.get("idHash"))
                .and_then(|v| v.as_str())
            {
                return Some(h.to_string());
            }
        }
    }

    for u in &arr {
        if let Some(h) = u
            .get("userIdHash")
            .or_else(|| u.get("user_id_hash"))
            .or_else(|| u.get("idHash"))
            .and_then(|v| v.as_str())
        {
            return Some(h.to_string());
        }
    }

    // Deep search fallback
    fn walk(v: &serde_json::Value) -> Option<String> {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(h) = map.get("userIdHash").and_then(|x| x.as_str()) {
                    return Some(h.to_string());
                }
                for child in map.values() {
                    if let Some(h) = walk(child) {
                        return Some(h);
                    }
                }
                None
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    if let Some(h) = walk(child) {
                        return Some(h);
                    }
                }
                None
            }
            _ => None,
        }
    }
    walk(val)
}

fn event_ms(ev: &ClockEvent) -> Option<i64> {
    ev.real_time.or(ev.target_time)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn today_local() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn compute_from_packs(packs: &[WorkClockPack]) -> WorkSnapshot {
    let today = today_local();
    let today_packs: Vec<&WorkClockPack> = packs
        .iter()
        .filter(|p| {
            p.applied_date
                .as_ref()
                .map(|d| d.starts_with(&today) || d == &today)
                .unwrap_or(true)
        })
        .collect();

    let packs_ref: Vec<&WorkClockPack> = if today_packs.is_empty() {
        packs.iter().collect()
    } else {
        today_packs
    };

    if packs_ref.is_empty() {
        return WorkSnapshot {
            state: "NotStarted".into(),
            worked_seconds: Some(0),
            label: "아직 출근하지 않았습니다".into(),
            started_at: None,
            error: None,
            fetched_at_ms: Some(now_ms()),
        };
    }

    let mut worked_ms: i64 = 0;
    let mut any_ongoing = false;
    let mut any_resting = false;
    let mut any_finished = false;
    let mut first_start: Option<i64> = None;
    let end_cap = now_ms();

    for pack in packs_ref {
        let Some(start) = pack.start_record.as_ref().and_then(event_ms) else {
            continue;
        };
        first_start = Some(first_start.map_or(start, |s| s.min(start)));

        let ongoing = pack.on_going.unwrap_or(false);
        if ongoing {
            any_ongoing = true;
        }

        // Build work intervals from start + switches until stop / now
        let mut points: Vec<i64> = vec![start];
        let mut switches = pack.switch_records.clone();
        switches.sort_by_key(|e| event_ms(e).unwrap_or(0));
        for sw in &switches {
            if let Some(t) = event_ms(sw) {
                if t >= start {
                    points.push(t);
                }
            }
        }

        let stop = pack.stop_record.as_ref().and_then(event_ms);
        let end = if ongoing {
            end_cap
        } else if let Some(s) = stop {
            any_finished = true;
            s
        } else {
            // No stop and not ongoing → treat as not started / incomplete
            continue;
        };

        // Sum work segments between consecutive points, clipped to end
        for window in points.windows(2) {
            let a = window[0];
            let b = window[1].min(end);
            if b > a {
                worked_ms += b - a;
            }
        }
        if let Some(last) = points.last().copied() {
            if end > last {
                worked_ms += end - last;
            }
        }

        // Subtract rests (PLAN_BY_AUTO may have future stop times)
        for rest in &pack.rest_records {
            let Some(rs) = rest.rest_start_record.as_ref().and_then(event_ms) else {
                continue;
            };
            let re = rest
                .rest_stop_record
                .as_ref()
                .and_then(event_ms)
                .unwrap_or(if ongoing { end_cap } else { end });
            if ongoing && rs <= end_cap && end_cap < re {
                any_resting = true;
            }
            let a = rs.max(start);
            let b = re.min(end);
            if b > a {
                worked_ms -= b - a;
            }
        }
    }

    let worked_seconds = Some(worked_ms.max(0) / 1000);
    let started_at = first_start.map(|ms| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|dt| dt.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default()
    });

    if any_ongoing && any_resting {
        return WorkSnapshot {
            state: "Resting".into(),
            worked_seconds,
            label: started_at
                .as_ref()
                .map(|t| format!("휴게 중 · 출근 {t}"))
                .unwrap_or_else(|| "휴게 중".into()),
            started_at,
            error: None,
            fetched_at_ms: Some(now_ms()),
        };
    }

    if any_ongoing {
        return WorkSnapshot {
            state: "Working".into(),
            worked_seconds,
            label: started_at
                .as_ref()
                .map(|t| format!("오늘 누적 · 출근 {t}"))
                .unwrap_or_else(|| "오늘 누적 근무시간".into()),
            started_at,
            error: None,
            fetched_at_ms: Some(now_ms()),
        };
    }

    if any_finished || worked_ms > 0 {
        return WorkSnapshot {
            state: "Done".into(),
            worked_seconds,
            label: "오늘 누적 근무시간".into(),
            started_at,
            error: None,
            fetched_at_ms: Some(now_ms()),
        };
    }

    WorkSnapshot {
        state: "NotStarted".into(),
        worked_seconds: Some(0),
        label: "아직 출근하지 않았습니다".into(),
        started_at: None,
        error: None,
        fetched_at_ms: Some(now_ms()),
    }
}

fn packs_from_status_value(val: &serde_json::Value) -> Vec<WorkClockPack> {
    // Current Flex API: onGoingRecordPack (singular object).
    // Older / alternate shapes: workClockRecordPacks (array).
    if let Some(pack) = val
        .get("onGoingRecordPack")
        .or_else(|| val.get("on_going_record_pack"))
    {
        if pack.is_null() {
            // Explicit null → not clocked in
            return Vec::new();
        }
        if let Ok(mut parsed) = serde_json::from_value::<WorkClockPack>(pack.clone()) {
            if parsed.applied_date.is_none() {
                if let Some(d) = val.get("targetDate").and_then(|v| v.as_str()) {
                    parsed.applied_date = Some(d.to_string());
                }
            }
            return vec![parsed];
        }
    }

    let packs_val = val
        .get("workClockRecordPacks")
        .or_else(|| val.get("work_clock_record_packs"))
        .cloned()
        .or_else(|| {
            fn find(v: &serde_json::Value) -> Option<serde_json::Value> {
                match v {
                    serde_json::Value::Object(map) => {
                        if let Some(p) = map.get("workClockRecordPacks") {
                            return Some(p.clone());
                        }
                        if let Some(p) = map.get("onGoingRecordPack") {
                            return Some(serde_json::Value::Array(vec![p.clone()]));
                        }
                        for child in map.values() {
                            if let Some(p) = find(child) {
                                return Some(p);
                            }
                        }
                        None
                    }
                    serde_json::Value::Array(items) => {
                        for child in items {
                            if let Some(p) = find(child) {
                                return Some(p);
                            }
                        }
                        None
                    }
                    _ => None,
                }
            }
            find(val)
        });

    if let Some(packs_val) = packs_val {
        return serde_json::from_value(packs_val).unwrap_or_default();
    }

    if let Ok(parsed) = serde_json::from_value::<CurrentStatusResponse>(val.clone()) {
        return parsed.work_clock_record_packs;
    }

    Vec::new()
}

fn parse_status_value(val: &serde_json::Value) -> WorkSnapshot {
    if val.as_object().map(|o| o.is_empty()).unwrap_or(false) {
        return need_not_started();
    }

    let packs = packs_from_status_value(val);
    if packs.is_empty()
        && val.get("onGoingRecordPack").is_none()
        && val.get("workClockRecordPacks").is_none()
        && val.get("targetDate").is_none()
    {
        return WorkSnapshot {
            state: "FetchError".into(),
            worked_seconds: None,
            label: "응답 형식을 해석하지 못했습니다".into(),
            started_at: None,
            error: Some("unexpected current-status payload".into()),
            fetched_at_ms: Some(now_ms()),
        };
    }

    compute_from_packs(&packs)
}

fn need_not_started() -> WorkSnapshot {
    WorkSnapshot {
        state: "NotStarted".into(),
        worked_seconds: Some(0),
        label: "아직 출근하지 않았습니다".into(),
        started_at: None,
        error: None,
        fetched_at_ms: Some(now_ms()),
    }
}

pub fn need_login(message: String) -> WorkSnapshot {
    WorkSnapshot {
        state: "NeedLogin".into(),
        worked_seconds: None,
        label: "flex 로그인이 필요합니다".into(),
        started_at: None,
        error: Some(message),
        fetched_at_ms: Some(now_ms()),
    }
}

pub fn fetch_error(message: String) -> WorkSnapshot {
    WorkSnapshot {
        state: "FetchError".into(),
        worked_seconds: None,
        label: "근무시간을 불러오지 못했습니다".into(),
        started_at: None,
        error: Some(message),
        fetched_at_ms: Some(now_ms()),
    }
}

/// 저장된 세션으로 한 번 조회한다. 갱신은 하지 않는다.
pub fn fetch_work(store: &SessionStore) -> WorkSnapshot {
    let Some(mut session) = store.get() else {
        return need_login(LOGIN_HINT.into());
    };

    let user_id = match session.account.as_ref().and_then(|a| a.user_id.clone()) {
        Some(h) => h,
        None => match get_json(&session.cookies, ME_PATH) {
            Ok(val) => match extract_user_id_hash(&val) {
                Some(h) => {
                    session.account.get_or_insert_with(Account::default).user_id = Some(h.clone());
                    let _ = store.save(session.clone());
                    h
                }
                None => {
                    return fetch_error("userIdHash를 찾지 못했습니다".into());
                }
            },
            Err(WorkError::NeedLogin(m)) => return need_login(m),
            Err(e) => return fetch_error(e.to_string()),
        },
    };

    let path = STATUS_PATH.replace("{userIdHash}", &urlencoding_loose(&user_id));
    match get_json(&session.cookies, &path) {
        Ok(val) => parse_status_value(&val),
        Err(WorkError::NeedLogin(m)) => need_login(m),
        Err(e) => fetch_error(e.to_string()),
    }
}

/// 위젯 폴링 경로. 핵심 쿠키 만료가 임박하면 미리, 401/403이면 한 번 무인 갱신(headless)한 뒤 다시 조회한다.
/// 갱신은 Chrome을 띄우므로 블로킹 스레드에서 부른다.
pub fn fetch_work_with_refresh(store: &SessionStore) -> WorkSnapshot {
    let Some(session) = store.get() else {
        return need_login(LOGIN_HINT.into());
    };
    if let Some(fresh) = browser_login::refresh_if_expiring(&FLEX_LOGIN, &session) {
        let _ = store.save(fresh);
    }

    let snap = fetch_work(store);
    if snap.state != "NeedLogin" {
        return snap;
    }
    let refreshed = browser_login::refresh_headless(&FLEX_LOGIN)
        .ok()
        .is_some_and(|fresh| store.save(fresh).is_ok());
    if refreshed {
        return fetch_work(store);
    }
    need_login(format!(
        "{} — {LOGIN_HINT}",
        snap.error.unwrap_or_else(|| "인증이 만료됐습니다".into())
    ))
}

fn urlencoding_loose(s: &str) -> String {
    // userIdHash is typically URL-safe; still encode reserved chars
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ongoing_record_pack() {
        let raw = r#"{
            "targetDate": "2026-08-04",
            "onGoingRecordPack": {
                "startRecord": {
                    "eventType": "START",
                    "targetTime": 1785802980000,
                    "realTime": 1785802980000,
                    "recordType": "RECORD",
                    "zoneId": "Asia/Seoul"
                },
                "switchRecords": [],
                "restRecords": [{
                    "restStartRecord": {
                        "eventType": "REST_START",
                        "targetTime": 1785816000000,
                        "recordType": "PLAN_BY_AUTO",
                        "zoneId": "Asia/Seoul"
                    },
                    "restStopRecord": {
                        "eventType": "REST_STOP",
                        "targetTime": 1785819600000,
                        "recordType": "PLAN_BY_AUTO",
                        "zoneId": "Asia/Seoul"
                    }
                }],
                "onGoing": true
            }
        }"#;
        let val: serde_json::Value = serde_json::from_str(raw).unwrap();
        let snap = parse_status_value(&val);
        assert!(
            snap.state == "Working" || snap.state == "Resting",
            "expected Working/Resting, got {} ({:?})",
            snap.state,
            snap.error
        );
        assert!(snap.worked_seconds.unwrap_or(0) > 0);
    }

    fn cookie(name: &str, value: &str) -> Cookie {
        serde_json::from_value(serde_json::json!({
            "name": name, "value": value, "domain": "flex.team"
        }))
        .unwrap()
    }

    #[test]
    fn cookie_header_sends_present_auth_cookies_only() {
        let cookies = vec![cookie("_ga", "x"), cookie("V2_WS_AID", "w"), cookie("AID", "a")];
        assert_eq!(cookie_header(&cookies), "AID=a; V2_WS_AID=w");
        assert_eq!(flex_aid(&cookies), Some("a"));
        assert_eq!(flex_aid(&[cookie("V2_WS_AID", "w")]), Some("w"));
        assert_eq!(flex_aid(&[cookie("AID", "")]), None);
    }

    #[test]
    fn reads_current_user_from_workspace_users() {
        let val = serde_json::json!({
            "currentUser": {
                "customer": { "customerIdHash": "c1", "name": "회사" },
                "user": { "userIdHash": "u-current", "name": "이름", "displayName": "표시" }
            },
            "users": [{ "user": { "userIdHash": "u-other" } }]
        });
        assert_eq!(extract_user_id_hash(&val).as_deref(), Some("u-current"));
        assert_eq!(account_label(&val).as_deref(), Some("표시 · 회사"));
    }

    /// 실측: 전용 프로필에 로그인이 남아 있어야 한다. headless Chrome을 띄운다.
    /// `cargo test -- --ignored live_` (출력에 쿠키 값·계정 정보를 남기지 않는다)
    #[test]
    #[ignore]
    fn live_refresh_headless_validates() {
        let session = browser_login::refresh_headless(&FLEX_LOGIN).expect("refresh");
        assert!(flex_aid(&session.cookies).is_some());
        assert!(session.account.and_then(|a| a.user_id).is_some());
    }

    #[test]
    fn null_ongoing_means_not_started() {
        let raw = r#"{
            "targetDate": "2026-08-04",
            "onGoingRecordPack": null,
            "targetDayWorkSchedule": { "date": "2026-08-04", "workRecords": [], "timeOffs": [] }
        }"#;
        let val: serde_json::Value = serde_json::from_str(raw).unwrap();
        let snap = parse_status_value(&val);
        assert_eq!(snap.state, "NotStarted");
    }
}
