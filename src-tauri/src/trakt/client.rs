use crate::trakt::auth::{ensure_valid_token, force_refresh_token, get_client_id};
use crate::trakt::{is_trakt_enabled, iso_utc_to_epoch, now_epoch_secs, now_iso_utc};
use crate::AppState;
use rusqlite::params;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

pub const TRAKT_GET_RATE_PER_5MIN: u64 = 450;
pub const TRAKT_WRITE_RATE_PER_SEC: f64 = 1.0;

pub struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_per_ms: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_per_ms: refill_per_sec / 1000.0,
            last_refill: Instant::now(),
        }
    }

    pub async fn acquire(&mut self) {
        let now = Instant::now();
        let elapsed_ms = now.duration_since(self.last_refill).as_millis() as f64;
        self.tokens = (self.tokens + elapsed_ms * self.refill_per_ms).min(self.capacity);
        self.last_refill = now;

        if self.tokens < 1.0 {
            let wait_ms = ((1.0 - self.tokens) / self.refill_per_ms).ceil() as u64;
            tokio::time::sleep(Duration::from_millis(wait_ms.max(1))).await;
            self.tokens = 0.0;
            self.last_refill = Instant::now();
        } else {
            self.tokens -= 1.0;
        }
    }
}

pub struct RateLimiter {
    pub get_bucket: Mutex<TokenBucket>,
    pub write_bucket: Mutex<TokenBucket>,
}

impl RateLimiter {
    pub fn new() -> Self {
        let get_per_sec = TRAKT_GET_RATE_PER_5MIN as f64 / 300.0;
        Self {
            get_bucket: Mutex::new(TokenBucket::new(15.0, get_per_sec)),
            write_bucket: Mutex::new(TokenBucket::new(1.0, TRAKT_WRITE_RATE_PER_SEC)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TraktHttpResponse {
    #[allow(dead_code)]
    pub status: u16,
    pub body: String,
    pub page_count: Option<u32>,
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
pub enum TraktApiError {
    Disabled,
    NotLoggedIn,
    AuthExpired,
    AccountLimitExceeded(String),
    VipRequired(String),
    LockedAccount(String),
    RateLimited(u64),
    Network(String),
    Http(u16, String),
}

impl TraktApiError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Network(_) | Self::RateLimited(_))
    }

    pub fn to_user_string(&self) -> String {
        match self {
            Self::Disabled => "TRAKT_DISABLED: A Trakt integráció ki van kapcsolva.".to_string(),
            Self::NotLoggedIn => "TRAKT_NOT_LOGGED_IN: Nincs bejelentkezve Trakt fiókba.".to_string(),
            Self::AuthExpired => "TRAKT_AUTH_EXPIRED: A Trakt munkamenet lejárt, jelentkezz be újra.".to_string(),
            Self::AccountLimitExceeded(m) => format!("TRAKT_ERR_420: {}", m),
            Self::VipRequired(m) => format!("TRAKT_ERR_426: {}", m),
            Self::LockedAccount(m) => format!("TRAKT_ERR_423: {}", m),
            Self::RateLimited(s) => format!("TRAKT_ERR_429: Túl sok kérés, várakozás {} mp.", s),
            Self::Network(m) => format!("TRAKT_OFFLINE: {}", m),
            Self::Http(code, m) => format!("Trakt API hiba ({}): {}", code, m),
        }
    }
}

impl From<TraktApiError> for String {
    fn from(e: TraktApiError) -> Self {
        e.to_user_string()
    }
}

struct CachedHttpEntry {
    etag: Option<String>,
    body: String,
    fetched_at: String,
    ttl_seconds: i64,
    /// `X-Pagination-Page-Count` of the cached answer; 0 = the answer had no pagination header.
    page_count: u32,
}

impl CachedHttpEntry {
    fn page_count_opt(&self) -> Option<u32> {
        if self.page_count > 0 {
            Some(self.page_count)
        } else {
            None
        }
    }
}

/// Rows written before the `page_count` column existed (NULL) are ignored on purpose: their pagination
/// info is unknown, and serving them would silently truncate paginated lists (e.g. > 100 hidden shows).
/// Ignoring them costs one full download per path; the row is then rewritten with a known page count.
fn read_http_cache(conn: &rusqlite::Connection, key: &str) -> Option<CachedHttpEntry> {
    conn.query_row(
        "SELECT etag, body, fetched_at, ttl_seconds, page_count FROM trakt_http_cache
         WHERE key = ?1 AND page_count IS NOT NULL",
        params![key],
        |r| {
            Ok(CachedHttpEntry {
                etag: r.get(0)?,
                body: r.get(1)?,
                fetched_at: r.get(2)?,
                ttl_seconds: r.get(3)?,
                page_count: r.get::<_, i64>(4)?.max(0) as u32,
            })
        },
    )
    .ok()
}

fn is_cache_fresh(entry: &CachedHttpEntry) -> bool {
    if entry.ttl_seconds <= 0 {
        return false;
    }
    let Some(fetched_epoch) = iso_utc_to_epoch(&entry.fetched_at) else {
        return false;
    };
    fetched_epoch + entry.ttl_seconds > now_epoch_secs()
}

fn write_http_cache(
    conn: &rusqlite::Connection,
    key: &str,
    etag: Option<&str>,
    body: &str,
    ttl_seconds: i64,
    page_count: Option<u32>,
) {
    // The ETag always belongs to the body stored next to it: an answer without ETag must not keep
    // the previous body's ETag (the next If-None-Match would validate against the wrong content).
    let _ = conn.execute(
        "INSERT INTO trakt_http_cache (key, etag, body, fetched_at, ttl_seconds, page_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(key) DO UPDATE SET
            etag = excluded.etag,
            body = excluded.body,
            fetched_at = excluded.fetched_at,
            ttl_seconds = excluded.ttl_seconds,
            page_count = excluded.page_count",
        params![key, etag, body, now_iso_utc(), ttl_seconds, page_count.unwrap_or(0) as i64],
    );
}

/// Expired rows are only useful for ETag revalidation, so they are kept for this long after expiry.
pub const HTTP_CACHE_PRUNE_GRACE_SECS: i64 = 7 * 86_400;

/// Deletes cache rows that expired more than `HTTP_CACHE_PRUNE_GRACE_SECS` ago. Fresh rows and rows still
/// inside the grace period are untouched, so no cached answer that could still be served is lost.
/// Returns the number of deleted rows.
pub fn prune_http_cache(conn: &rusqlite::Connection) -> usize {
    conn.execute(
        "DELETE FROM trakt_http_cache
         WHERE CAST(strftime('%s', fetched_at) AS INTEGER) + ttl_seconds + ?1
               < CAST(strftime('%s', 'now') AS INTEGER)",
        params![HTTP_CACHE_PRUNE_GRACE_SECS],
    )
    .unwrap_or(0)
}

/// Calendar paths carry the start date, so every day creates a new multi-MB row. After a successful
/// download only the current key of that calendar is kept.
pub fn prune_stale_calendar_keys(conn: &rusqlite::Connection, prefix: &str, keep_key: &str) -> usize {
    conn.execute(
        "DELETE FROM trakt_http_cache WHERE key LIKE ?1 AND key != ?2",
        params![format!("{}%", prefix), keep_key],
    )
    .unwrap_or(0)
}

fn touch_http_cache(conn: &rusqlite::Connection, key: &str, ttl_seconds: i64) {
    let _ = conn.execute(
        "UPDATE trakt_http_cache SET fetched_at = ?1, ttl_seconds = ?2 WHERE key = ?3",
        params![now_iso_utc(), ttl_seconds, key],
    );
}

pub async fn single_flight<F, Fut>(
    state: &AppState,
    key: &str,
    fetch: F,
) -> Result<TraktHttpResponse, TraktApiError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<TraktHttpResponse, TraktApiError>>,
{
    let lock = {
        let mut map = state.trakt.inflight.lock().unwrap();
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().await;
    let res = fetch().await;
    {
        let mut map = state.trakt.inflight.lock().unwrap();
        if let Some(existing) = map.get(key) {
            if Arc::strong_count(existing) <= 2 {
                map.remove(key);
            }
        }
    }
    res
}

pub async fn trakt_get_raw(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    path: &str,
    use_auth: bool,
) -> Result<TraktHttpResponse, TraktApiError> {
    trakt_get_with_ttl(state, app, path, use_auth, 0, false).await
}

pub async fn trakt_get_cached(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    path: &str,
    use_auth: bool,
    ttl_seconds: i64,
    force_refresh: bool,
) -> Result<TraktHttpResponse, TraktApiError> {
    single_flight(state, path, || {
        trakt_get_with_ttl(state, app, path, use_auth, ttl_seconds, force_refresh)
    })
    .await
}

async fn trakt_get_with_ttl(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    path: &str,
    use_auth: bool,
    ttl_seconds: i64,
    force_refresh: bool,
) -> Result<TraktHttpResponse, TraktApiError> {
    let (enabled, client_id, cached_entry) = {
        let conn = state.db.lock().await;
        let en = is_trakt_enabled(&conn);
        let cid = get_client_id(&conn);
        let cached = if ttl_seconds > 0 {
            read_http_cache(&conn, path)
        } else {
            None
        };
        (en, cid, cached)
    };

    if !enabled {
        return Err(TraktApiError::Disabled);
    }

    if !force_refresh {
        if let Some(ref entry) = cached_entry {
            if is_cache_fresh(entry) {
                return Ok(TraktHttpResponse {
                    status: 200,
                    body: entry.body.clone(),
                    page_count: entry.page_count_opt(),
                    from_cache: true,
                });
            }
        }
    }

    let mut token_opt = if use_auth {
        Some(
            ensure_valid_token(state, app)
                .await
                .map_err(|e| {
                    if e.contains("TRAKT_NOT_LOGGED_IN") {
                        TraktApiError::NotLoggedIn
                    } else if e.contains("TRAKT_AUTH_EXPIRED") {
                        TraktApiError::AuthExpired
                    } else {
                        TraktApiError::Network(e)
                    }
                })?,
        )
    } else {
        None
    };

    let url = format!("{}{}", state.trakt.api_base, path);
    let mut refreshed_once = false;
    let mut retries = 0u8;

    loop {
        state.trakt.limiter.get_bucket.lock().await.acquire().await;

        let mut req = state
            .trakt
            .client
            .get(&url)
            .header("Content-Type", "application/json")
            .header("trakt-api-version", "2")
            .header("trakt-api-key", &client_id);

        if let Some(ref tok) = token_opt {
            req = req.header("Authorization", format!("Bearer {}", tok.as_str()));
        }
        if !force_refresh {
            if let Some(ref entry) = cached_entry {
                if let Some(ref etag) = entry.etag {
                    req = req.header("If-None-Match", etag);
                }
            }
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                if retries < 1 {
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                return Err(TraktApiError::Network(e.to_string()));
            }
        };

        let status = resp.status().as_u16();

        if status == 304 {
            if let Some(ref entry) = cached_entry {
                let conn = state.db.lock().await;
                touch_http_cache(&conn, path, ttl_seconds);
                return Ok(TraktHttpResponse {
                    status: 200,
                    body: entry.body.clone(),
                    page_count: entry.page_count_opt(),
                    from_cache: true,
                });
            }
        }

        if status == 401 && use_auth && !refreshed_once {
            refreshed_once = true;
            let new_tok = force_refresh_token(state, app).await.map_err(|e| {
                if e.contains("TRAKT_AUTH_EXPIRED") {
                    TraktApiError::AuthExpired
                } else {
                    TraktApiError::Network(e)
                }
            })?;
            token_opt = Some(new_tok);
            continue;
        }

        if status == 429 {
            let retry_secs = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(5)
                .min(60);
            if retries < 2 {
                retries += 1;
                tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                continue;
            }
            return Err(TraktApiError::RateLimited(retry_secs));
        }

        if (500..=599).contains(&status) {
            if retries < 2 {
                retries += 1;
                let backoff = Duration::from_secs(1 << retries);
                tokio::time::sleep(backoff).await;
                continue;
            }
            let body = resp.text().await.unwrap_or_default();
            return Err(TraktApiError::Network(format!("Szerverhiba ({}): {}", status, body)));
        }

        let etag = resp
            .headers()
            .get("ETag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let page_count = resp
            .headers()
            .get("X-Pagination-Page-Count")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u32>().ok());

        let body = resp
            .text()
            .await
            .map_err(|e| TraktApiError::Network(e.to_string()))?;

        if (200..300).contains(&status) {
            if ttl_seconds > 0 {
                let conn = state.db.lock().await;
                write_http_cache(&conn, path, etag.as_deref(), &body, ttl_seconds, page_count);
            }
            return Ok(TraktHttpResponse {
                status,
                body,
                page_count,
                from_cache: false,
            });
        }

        return Err(map_error_status(status, body));
    }
}

pub async fn trakt_post(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    path: &str,
    payload: &serde_json::Value,
) -> Result<(u16, String), TraktApiError> {
    let (enabled, client_id) = {
        let conn = state.db.lock().await;
        (is_trakt_enabled(&conn), get_client_id(&conn))
    };
    if !enabled {
        return Err(TraktApiError::Disabled);
    }

    let mut token = ensure_valid_token(state, app).await.map_err(|e| {
        if e.contains("TRAKT_NOT_LOGGED_IN") {
            TraktApiError::NotLoggedIn
        } else if e.contains("TRAKT_AUTH_EXPIRED") {
            TraktApiError::AuthExpired
        } else {
            TraktApiError::Network(e)
        }
    })?;

    let url = format!("{}{}", state.trakt.api_base, path);
    let mut refreshed_once = false;
    let mut retries = 0u8;

    loop {
        state.trakt.limiter.write_bucket.lock().await.acquire().await;

        let resp = match state
            .trakt
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("trakt-api-version", "2")
            .header("trakt-api-key", &client_id)
            .header("Authorization", format!("Bearer {}", token.as_str()))
            .json(payload)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Err(TraktApiError::Network(e.to_string())),
        };

        let status = resp.status().as_u16();

        if status == 401 && !refreshed_once {
            refreshed_once = true;
            token = force_refresh_token(state, app).await.map_err(|e| {
                if e.contains("TRAKT_AUTH_EXPIRED") {
                    TraktApiError::AuthExpired
                } else {
                    TraktApiError::Network(e)
                }
            })?;
            continue;
        }

        if status == 429 {
            let retry_secs = resp
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(5)
                .min(60);
            if retries < 1 {
                retries += 1;
                tokio::time::sleep(Duration::from_secs(retry_secs)).await;
                continue;
            }
            return Err(TraktApiError::RateLimited(retry_secs));
        }

        let body = resp
            .text()
            .await
            .map_err(|e| TraktApiError::Network(e.to_string()))?;

        if (200..300).contains(&status) {
            return Ok((status, body));
        }

        if (500..=599).contains(&status) {
            return Err(TraktApiError::Network(format!("Szerverhiba ({}): {}", status, body)));
        }

        return Err(map_error_status(status, body));
    }
}

fn map_error_status(status: u16, body: String) -> TraktApiError {
    match status {
        401 => TraktApiError::AuthExpired,
        420 => TraktApiError::AccountLimitExceeded(
            "Elérted a Trakt fiókod lista/Watchlist korlátját (420).".to_string(),
        ),
        423 => TraktApiError::LockedAccount(
            "A Trakt fiókod zárolva van (423). Lépj kapcsolatba a Trakt támogatással.".to_string(),
        ),
        426 => TraktApiError::VipRequired(
            "Ez a funkció Trakt VIP előfizetést igényel (426).".to_string(),
        ),
        _ => TraktApiError::Http(status, body),
    }
}
