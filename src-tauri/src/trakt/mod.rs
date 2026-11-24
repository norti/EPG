pub mod auth;
pub mod calendar;
pub mod client;
pub mod commands;
pub mod matcher;
pub mod secret;
pub mod sync;
#[cfg(test)]
pub(crate) mod tests;

use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex, RwLock};

pub const DEFAULT_TRAKT_CLIENT_ID: &str = "qM0BgANPA04KDdCV_LgKeJkZ1LBR3tCnwCUGTtwzyfM";
pub const TRAKT_API_BASE: &str = "https://api.trakt.tv";
pub const TRAKT_AUTH_BASE: &str = "https://auth.trakt.tv";

pub struct TraktState {
    pub client: reqwest::Client,
    pub api_base: String,
    pub auth_base: String,
    pub limiter: Arc<client::RateLimiter>,
    pub inflight: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub watchlist_version: AtomicU64,
    pub token: Mutex<Option<auth::TraktToken>>,
    pub title_index: RwLock<matcher::TitleIndex>,
    pub match_cache: RwLock<(u64, HashMap<String, bool>)>,
    pub login_cancel: Mutex<Option<watch::Sender<bool>>>,
    pub sync_lock: Mutex<()>,
    pub last_focus_sync: Mutex<Option<Instant>>,
}

impl TraktState {
    pub fn new(conn: &Connection) -> Self {
        Self::with_bases(conn, TRAKT_API_BASE.to_string(), TRAKT_AUTH_BASE.to_string())
    }

    pub fn with_bases(conn: &Connection, api_base: String, auth_base: String) -> Self {
        let ua = format!("EPGViewer/{}", env!("CARGO_PKG_VERSION"));
        let client = reqwest::Client::builder()
            .user_agent(ua)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(6)
            .build()
            .expect("Trakt HTTP kliens létrehozási hiba");

        let title_index = matcher::TitleIndex::build_from_db(conn).unwrap_or_default();

        let initial_version: u64 = conn
            .query_row(
                "SELECT value FROM trakt_sync_state WHERE key = 'watchlist_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1);

        Self {
            client,
            api_base,
            auth_base,
            limiter: Arc::new(client::RateLimiter::new()),
            inflight: std::sync::Mutex::new(HashMap::new()),
            watchlist_version: AtomicU64::new(initial_version),
            token: Mutex::new(None),
            title_index: RwLock::new(title_index),
            match_cache: RwLock::new((0, HashMap::new())),
            login_cancel: Mutex::new(None),
            sync_lock: Mutex::new(()),
            last_focus_sync: Mutex::new(None),
        }
    }
}

pub fn is_trakt_enabled(conn: &Connection) -> bool {
    let val: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'trakt_enabled'",
            [],
            |r| r.get(0),
        )
        .ok();
    match val.as_deref() {
        Some("false") | Some("0") => false,
        _ => true,
    }
}

pub fn now_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_epoch_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn epoch_to_iso_utc(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86400);
    let rem = epoch_secs.rem_euclid(86400);
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if mo <= 2 { 1 } else { 0 };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, mo, d, h, m, s
    )
}

pub fn now_iso_utc() -> String {
    epoch_to_iso_utc(now_epoch_secs())
}

pub fn iso_utc_to_epoch(iso: &str) -> Option<i64> {
    let s = iso.trim();
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let h: i64 = s.get(11..13)?.parse().ok()?;
    let mi: i64 = s.get(14..16)?.parse().ok()?;
    let se: i64 = s.get(17..19)?.parse().ok()?;

    let y_adj = y - if mo <= 2 { 1 } else { 0 };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + se)
}
