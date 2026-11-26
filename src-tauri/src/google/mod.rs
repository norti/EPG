pub mod auth;
pub mod calendar;
pub mod commands;
pub mod drive;
#[cfg(test)]
mod tests;

use std::fmt;
use std::time::Duration;
use tokio::sync::{watch, Mutex};
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct GoogleToken {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Zeroizing<String>,
    pub expires_at: i64,
}

impl fmt::Debug for GoogleToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleToken")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub(crate) struct StagedDriveDownload {
    pub file_id: String,
    pub modified_time: Option<String>,
    pub payload: drive::SyncPayload,
}

pub struct GoogleState {
    pub client: reqwest::Client,
    pub token: Mutex<Option<GoogleToken>>,
    pub auth_cancel: Mutex<Option<watch::Sender<bool>>>,
    pub sync_lock: Mutex<()>,
    pub(crate) staged_download: Mutex<Option<StagedDriveDownload>>,
}

impl GoogleState {
    pub fn new() -> Self {
        let ua = format!("EPGViewer/{}", env!("CARGO_PKG_VERSION"));
        let client = reqwest::Client::builder()
            .user_agent(ua)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(5))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(6)
            .build()
            .expect("Google HTTP kliens létrehozási hiba");

        Self {
            client,
            token: Mutex::new(None),
            auth_cancel: Mutex::new(None),
            sync_lock: Mutex::new(()),
            staged_download: Mutex::new(None),
        }
    }
}

pub fn prune_http_cache(conn: &rusqlite::Connection) -> usize {
    conn.execute(
        "DELETE FROM google_http_cache
         WHERE CAST(strftime('%s', fetched_at) AS INTEGER) + ttl_seconds
               < CAST(strftime('%s', 'now') AS INTEGER)",
        [],
    )
    .unwrap_or(0)
}
