use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const FLEX_ORIGIN: &str = "https://flex.team";
const ME_PATH: &str = "/api/v2/workspace/users/me/workspace-users";
const STATUS_PATH: &str = "/api/v2/time-tracking/work-clock/users/{userIdHash}/current-status";

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionTokens {
    pub aid: String,
    pub ws_aid: String,
    pub ws_rid: String,
    pub user_id_hash: Option<String>,
    pub updated_at_ms: Option<i64>,
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

pub struct SessionStore {
    path: PathBuf,
    tokens: Mutex<Option<SessionTokens>>,
}

impl SessionStore {
    pub fn new(app_data: PathBuf) -> Self {
        let path = app_data.join("session.json");
        let tokens = read_session(&path);
        Self {
            path,
            tokens: Mutex::new(tokens),
        }
    }

    pub fn get(&self) -> Option<SessionTokens> {
        self.tokens.lock().ok().and_then(|g| g.clone())
    }

    pub fn save(&self, tokens: SessionTokens) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&tokens).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).map_err(|e| e.to_string())?;
        if let Ok(mut guard) = self.tokens.lock() {
            *guard = Some(tokens);
        }
        Ok(())
    }
}

fn read_session(path: &PathBuf) -> Option<SessionTokens> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn session_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("FlexWorkWidget")
}

fn cookie_header(tokens: &SessionTokens) -> String {
    format!(
        "AID={}; V2_WS_AID={}; V2_WS_RID={}",
        tokens.aid, tokens.ws_aid, tokens.ws_rid
    )
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build()
}

fn get_json(tokens: &SessionTokens, path: &str) -> Result<serde_json::Value, WorkError> {
    let url = format!("{FLEX_ORIGIN}{path}");
    let aid = if !tokens.aid.is_empty() {
        tokens.aid.clone()
    } else {
        tokens.ws_aid.clone()
    };
    let resp = agent()
        .get(&url)
        .set("Origin", FLEX_ORIGIN)
        .set("Referer", &format!("{FLEX_ORIGIN}/home"))
        .set("Accept", "application/json")
        .set("User-Agent", "flex-work-widget/0.1")
        .set("Cookie", &cookie_header(tokens))
        .set("x-flex-aid", &aid)
        .set("FlexTeam-Version", "V2")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) | ureq::Error::Status(403, _) => {
                WorkError::NeedLogin(format!("auth failed: {e}"))
            }
            other => WorkError::Fetch(other.to_string()),
        })?;

    resp.into_json::<serde_json::Value>()
        .map_err(|e| WorkError::Parse(e.to_string()))
}

fn extract_user_id_hash(val: &serde_json::Value) -> Option<String> {
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

        // Subtract rests
        for rest in &pack.rest_records {
            let Some(rs) = rest.rest_start_record.as_ref().and_then(event_ms) else {
                continue;
            };
            let re = rest
                .rest_stop_record
                .as_ref()
                .and_then(event_ms)
                .unwrap_or(if ongoing { end_cap } else { end });
            if ongoing && rest.rest_stop_record.is_none() {
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

    if any_finished || worked_seconds.unwrap_or(0) > 0 {
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

fn parse_status_value(val: &serde_json::Value) -> WorkSnapshot {
    // Prefer known field, else deep-find workClockRecordPacks
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

    let Some(packs_val) = packs_val else {
        // Maybe empty object means not started
        if val.as_object().map(|o| o.is_empty()).unwrap_or(false) {
            return need_not_started();
        }
        // Try deserialize whole as CurrentStatusResponse
        if let Ok(parsed) = serde_json::from_value::<CurrentStatusResponse>(val.clone()) {
            return compute_from_packs(&parsed.work_clock_record_packs);
        }
        return WorkSnapshot {
            state: "FetchError".into(),
            worked_seconds: None,
            label: "응답 형식을 해석하지 못했습니다".into(),
            started_at: None,
            error: Some("unexpected current-status payload".into()),
            fetched_at_ms: Some(now_ms()),
        };
    };

    let packs: Vec<WorkClockPack> = serde_json::from_value(packs_val).unwrap_or_default();
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

pub fn fetch_work(store: &SessionStore) -> WorkSnapshot {
    let Some(mut tokens) = store.get() else {
        return need_login("세션 없음".into());
    };

    if tokens.aid.is_empty() && tokens.ws_aid.is_empty() {
        return need_login("세션 토큰 없음".into());
    }

    let user_id = if let Some(h) = tokens.user_id_hash.clone() {
        h
    } else {
        match get_json(&tokens, ME_PATH) {
            Ok(val) => match extract_user_id_hash(&val) {
                Some(h) => {
                    tokens.user_id_hash = Some(h.clone());
                    let _ = store.save(tokens.clone());
                    h
                }
                None => {
                    return fetch_error("userIdHash를 찾지 못했습니다".into());
                }
            },
            Err(WorkError::NeedLogin(m)) => return need_login(m),
            Err(e) => return fetch_error(e.to_string()),
        }
    };

    let path = STATUS_PATH.replace("{userIdHash}", &urlencoding_loose(&user_id));
    match get_json(&tokens, &path) {
        Ok(val) => parse_status_value(&val),
        Err(WorkError::NeedLogin(m)) => need_login(m),
        Err(e) => fetch_error(e.to_string()),
    }
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

/// Parse cookies pasted or captured from the login webview document/cookie bridge.
pub fn tokens_from_cookie_header(cookie_header: &str) -> SessionTokens {
    let mut tokens = SessionTokens::default();
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "AID" => tokens.aid = v.trim().to_string(),
                "V2_WS_AID" => tokens.ws_aid = v.trim().to_string(),
                "V2_WS_RID" => tokens.ws_rid = v.trim().to_string(),
                _ => {}
            }
        }
    }
    tokens.updated_at_ms = Some(now_ms());
    tokens
}

pub fn tokens_from_parts(aid: String, ws_aid: String, ws_rid: String) -> SessionTokens {
    SessionTokens {
        aid,
        ws_aid,
        ws_rid,
        user_id_hash: None,
        updated_at_ms: Some(now_ms()),
    }
}
