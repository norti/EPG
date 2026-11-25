use crate::trakt::secret::{default_secret_store, protect_str, unprotect_str, SecretStore};
use crate::trakt::{
    is_trakt_enabled, now_epoch_secs, now_iso_utc, DEFAULT_TRAKT_CLIENT_ID,
};
use crate::AppState;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::sync::watch;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct TraktToken {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Zeroizing<String>,
    pub expires_at: i64,
}

impl fmt::Debug for TraktToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraktToken")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodeResponse {
    #[serde(alias = "device_code")]
    pub device_code: String,
    #[serde(alias = "user_code")]
    pub user_code: String,
    #[serde(alias = "verification_url")]
    pub verification_url: String,
    #[serde(alias = "expires_in")]
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Deserialize, Debug)]
struct TokenApiResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    #[serde(default)]
    created_at: i64,
}

#[derive(Deserialize, Debug)]
struct OAuthErrorBody {
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktProfile {
    pub username: String,
    #[serde(default)]
    pub slug: Option<String>,
    pub name: Option<String>,
    pub vip: bool,
    pub vip_ep: bool,
    pub joined_at: Option<String>,
    pub avatar_url: Option<String>,
    pub watchlist_limit: Option<i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktAuthStatus {
    pub logged_in: bool,
    pub enabled: bool,
    pub profile: Option<TraktProfile>,
    pub expires_at: Option<i64>,
    pub has_custom_client_id: bool,
    pub has_custom_client_secret: bool,
    pub custom_client_id: Option<String>,
}

pub fn get_client_id(conn: &Connection) -> String {
    let custom: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'trakt_client_id_override'",
            [],
            |r| r.get(0),
        )
        .ok();
    match custom {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => DEFAULT_TRAKT_CLIENT_ID.to_string(),
    }
}

pub fn get_custom_client_id(conn: &Connection) -> Option<String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'trakt_client_id_override'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

pub fn get_custom_client_secret(
    conn: &Connection,
    store: &dyn SecretStore,
) -> Option<Zeroizing<String>> {
    let blob: Option<String> = conn
        .query_row(
            "SELECT client_secret_blob FROM trakt_auth WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    blob.and_then(|b| {
        if b.trim().is_empty() {
            None
        } else {
            unprotect_str(store, &b).ok().filter(|s| !s.trim().is_empty())
        }
    })
}

pub fn has_custom_client_secret(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT client_secret_blob FROM trakt_auth WHERE id = 1",
        [],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
    .map(|s| !s.trim().is_empty())
    .unwrap_or(false)
}

pub fn save_custom_client_secret(
    conn: &Connection,
    store: &dyn SecretStore,
    secret: Option<&str>,
) -> Result<(), String> {
    let clean = secret.map(|s| s.trim()).filter(|s| !s.is_empty());
    let blob = match clean {
        Some(s) => Some(protect_str(store, s)?),
        None => None,
    };

    let exists: bool = conn
        .query_row("SELECT EXISTS(SELECT 1 FROM trakt_auth WHERE id = 1)", [], |r| {
            r.get(0)
        })
        .unwrap_or(false);

    if exists {
        conn.execute(
            "UPDATE trakt_auth SET client_secret_blob = ?1 WHERE id = 1",
            params![blob],
        )
        .map_err(|e| e.to_string())?;
    } else if blob.is_some() {
        conn.execute(
            "INSERT INTO trakt_auth (id, access_blob, refresh_blob, client_secret_blob, expires_at, created_at)
             VALUES (1, '', '', ?1, 0, ?2)",
            params![blob, now_iso_utc()],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn save_tokens_tx(
    conn: &mut Connection,
    store: &dyn SecretStore,
    access_token: &str,
    refresh_token: &str,
    expires_at: i64,
) -> Result<(), String> {
    let access_blob = protect_str(store, access_token)?;
    let refresh_blob = protect_str(store, refresh_token)?;
    let now = now_iso_utc();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO trakt_auth (id, access_blob, refresh_blob, client_secret_blob, expires_at, created_at)
         VALUES (1, ?1, ?2, NULL, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
            access_blob = excluded.access_blob,
            refresh_blob = excluded.refresh_blob,
            expires_at = excluded.expires_at,
            created_at = excluded.created_at",
        params![access_blob, refresh_blob, expires_at, now],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_tokens_from_db(
    conn: &Connection,
    store: &dyn SecretStore,
) -> Result<Option<TraktToken>, String> {
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT access_blob, refresh_blob, expires_at FROM trakt_auth WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();

    let Some((access_blob, refresh_blob, expires_at)) = row else {
        return Ok(None);
    };

    if access_blob.trim().is_empty() || refresh_blob.trim().is_empty() {
        return Ok(None);
    }

    let access_token = unprotect_str(store, &access_blob)?;
    let refresh_token = unprotect_str(store, &refresh_blob)?;
    Ok(Some(TraktToken {
        access_token,
        refresh_token,
        expires_at,
    }))
}

pub fn clear_tokens_from_db(conn: &Connection) -> Result<(), String> {
    let has_secret = has_custom_client_secret(conn);
    if has_secret {
        conn.execute(
            "UPDATE trakt_auth SET access_blob = '', refresh_blob = '', expires_at = 0 WHERE id = 1",
            [],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute("DELETE FROM trakt_auth WHERE id = 1", [])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn load_cached_profile(conn: &Connection) -> Option<TraktProfile> {
    let json_str: String = conn
        .query_row(
            "SELECT value FROM trakt_sync_state WHERE key = 'profile_json'",
            [],
            |r| r.get(0),
        )
        .ok()?;
    serde_json::from_str(&json_str).ok()
}

pub fn save_cached_profile(conn: &Connection, profile: &TraktProfile) -> Result<(), String> {
    let json_str = serde_json::to_string(profile).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO trakt_sync_state (key, value) VALUES ('profile_json', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![json_str],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_auth_status(state: &AppState) -> Result<TraktAuthStatus, String> {
    let store = default_secret_store();
    let conn = state.db.lock().await;
    let enabled = is_trakt_enabled(&conn);
    let custom_client_id = get_custom_client_id(&conn);
    let has_custom_secret = has_custom_client_secret(&conn);
    let profile = load_cached_profile(&conn);

    let tok_opt = load_tokens_from_db(&conn, store.as_ref()).ok().flatten();
    let (logged_in, expires_at) = match tok_opt {
        Some(t) => (true, Some(t.expires_at)),
        None => (false, None),
    };

    Ok(TraktAuthStatus {
        logged_in,
        enabled,
        profile,
        expires_at,
        has_custom_client_id: custom_client_id.is_some(),
        has_custom_client_secret: has_custom_secret,
        custom_client_id,
    })
}

pub async fn cancel_device_flow(state: &AppState) {
    let mut guard = state.trakt.login_cancel.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(true);
    }
}

pub async fn start_device_flow(
    state: &AppState,
    app: Option<tauri::AppHandle>,
) -> Result<DeviceCodeResponse, String> {
    cancel_device_flow(state).await;

    let client_id = {
        let conn = state.db.lock().await;
        if !is_trakt_enabled(&conn) {
            return Err("A Trakt integráció ki van kapcsolva.".to_string());
        }
        get_client_id(&conn)
    };

    let url = format!("{}/oauth/device/code", state.trakt.auth_base);
    let resp = state
        .trakt
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("trakt-api-version", "2")
        .header("trakt-api-key", &client_id)
        .json(&json!({ "client_id": client_id }))
        .send()
        .await
        .map_err(|e| format!("Trakt hálózati hiba: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Trakt Device Code hiba ({}): {}", status, body));
    }

    let data: DeviceCodeResponse = resp
        .json()
        .await
        .map_err(|e| format!("Trakt válasz feldolgozási hiba: {}", e))?;

    if let Some(app_handle) = app {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        {
            let mut guard = state.trakt.login_cancel.lock().await;
            *guard = Some(cancel_tx);
        }

        let device_code = data.device_code.clone();
        let interval = data.interval.max(2);
        let expires_in = data.expires_in.max(30);

        tauri::async_runtime::spawn(async move {
            poll_device_token_task(app_handle, device_code, interval, expires_in, cancel_rx).await;
        });
    }

    Ok(data)
}

#[derive(Debug, PartialEq)]
pub enum PollStepResult {
    Pending,
    SlowDown,
    Success,
    TerminalError(u16, String),
}

pub async fn poll_device_token_once(
    state: &AppState,
    device_code: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<PollStepResult, String> {
    let url = format!("{}/oauth/device/token", state.trakt.auth_base);
    let mut payload = json!({
        "code": device_code,
        "client_id": client_id,
    });
    if let Some(sec) = client_secret {
        if !sec.trim().is_empty() {
            payload["client_secret"] = json!(sec.trim());
        }
    }

    let resp = state
        .trakt
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("trakt-api-version", "2")
        .header("trakt-api-key", client_id)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Hálózati hiba: {}", e))?;

    let status = resp.status().as_u16();
    match status {
        200 => {
            let tok_resp: TokenApiResponse = resp
                .json()
                .await
                .map_err(|e| format!("Token JSON hiba: {}", e))?;
            let base_ts = if tok_resp.created_at > 0 {
                tok_resp.created_at
            } else {
                now_epoch_secs()
            };
            let expires_at = base_ts + tok_resp.expires_in;

            let store = default_secret_store();
            {
                let mut conn = state.db.lock().await;
                save_tokens_tx(
                    &mut conn,
                    store.as_ref(),
                    &tok_resp.access_token,
                    &tok_resp.refresh_token,
                    expires_at,
                )?;
            }
            {
                let mut guard = state.trakt.token.lock().await;
                *guard = Some(TraktToken {
                    access_token: Zeroizing::new(tok_resp.access_token),
                    refresh_token: Zeroizing::new(tok_resp.refresh_token),
                    expires_at,
                });
            }
            Ok(PollStepResult::Success)
        }
        400 => Ok(PollStepResult::Pending),
        429 => Ok(PollStepResult::SlowDown),
        404 | 409 | 410 | 418 => {
            let msg = match status {
                404 => "Érvénytelen aktiválási kód (404)",
                409 => "A kód már felhasználásra került (409)",
                410 => "Az aktiválási kód lejárt (410)",
                418 => "A felhasználó elutasította a hozzáférést (418)",
                _ => "Hitelesítési hiba",
            };
            Ok(PollStepResult::TerminalError(status, msg.to_string()))
        }
        _ => {
            let body = resp.text().await.unwrap_or_default();
            Ok(PollStepResult::TerminalError(
                status,
                format!("Váratlan Trakt válasz ({}): {}", status, body),
            ))
        }
    }
}

async fn poll_device_token_task(
    app: tauri::AppHandle,
    device_code: String,
    interval: u64,
    expires_in: u64,
    mut cancel_rx: watch::Receiver<bool>,
) {
    let state = app.state::<AppState>();
    let (client_id, client_secret) = {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        (
            get_client_id(&conn),
            get_custom_client_secret(&conn, store.as_ref()),
        )
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(expires_in);
    let mut delay = interval;

    loop {
        if *cancel_rx.borrow() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = app.emit(
                "trakt-login-state",
                json!({ "status": "error", "code": 410, "message": "Az aktiválási kód lejárt." }),
            );
            break;
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(delay)) => {}
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    break;
                }
            }
        }

        if *cancel_rx.borrow() {
            break;
        }

        match poll_device_token_once(
            state.inner(),
            &device_code,
            &client_id,
            client_secret.as_ref().map(|s| s.as_str()),
        )
        .await
        {
            Ok(PollStepResult::Pending) => {}
            Ok(PollStepResult::SlowDown) => {
                delay = (delay + interval).min(60);
            }
            Ok(PollStepResult::Success) => {
                let profile = fetch_and_save_profile(state.inner()).await.ok();
                let _ = app.emit(
                    "trakt-login-state",
                    json!({ "status": "ok", "profile": profile }),
                );
                let app_clone = app.clone();
                tauri::async_runtime::spawn(async move {
                    let st = app_clone.state::<AppState>();
                    let _ = crate::trakt::sync::run_sync(st.inner(), Some(&app_clone), true).await;
                });
                break;
            }
            Ok(PollStepResult::TerminalError(code, msg)) => {
                let _ = app.emit(
                    "trakt-login-state",
                    json!({ "status": "error", "code": code, "message": msg }),
                );
                break;
            }
            Err(_) => {
                // Transient network error during poll: keep trying until deadline or cancel
            }
        }
    }
}

pub async fn fetch_and_save_profile(state: &AppState) -> Result<TraktProfile, String> {
    let resp = crate::trakt::client::trakt_get_revalidated(state, None, "/users/settings", true).await?;
    let val: serde_json::Value = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Profil JSON hiba: {}", e))?;

    let user = val.get("user").unwrap_or(&val);
    let username = user
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("Trakt User")
        .to_string();
    let slug = user
        .pointer("/ids/slug")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(username.clone()));
    let name = user
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let vip = user.get("vip").and_then(|v| v.as_bool()).unwrap_or(false);
    let vip_ep = user.get("vip_ep").and_then(|v| v.as_bool()).unwrap_or(false);
    let joined_at = user
        .get("joined_at")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let avatar_url = user
        .pointer("/images/avatar/full")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let watchlist_limit = val
        .pointer("/limits/watchlist/item_count")
        .and_then(|v| v.as_i64());

    let profile = TraktProfile {
        username,
        slug,
        name,
        vip,
        vip_ep,
        joined_at,
        avatar_url,
        watchlist_limit,
    };

    let conn = state.db.lock().await;
    save_cached_profile(&conn, &profile)?;
    Ok(profile)
}

pub async fn ensure_valid_token(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
) -> Result<Zeroizing<String>, String> {
    let mut guard = state.trakt.token.lock().await;
    if guard.is_none() {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        *guard = load_tokens_from_db(&conn, store.as_ref())?;
    }

    let Some(ref tok) = *guard else {
        return Err("TRAKT_NOT_LOGGED_IN".to_string());
    };

    if tok.expires_at > now_epoch_secs() + 300 {
        return Ok(tok.access_token.clone());
    }

    refresh_token_locked(state, &mut guard, app).await
}

pub async fn force_refresh_token(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    failed_access_token: &str,
) -> Result<Zeroizing<String>, String> {
    let mut guard = state.trakt.token.lock().await;
    if guard.is_none() {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        *guard = load_tokens_from_db(&conn, store.as_ref())?;
    }
    let Some(ref current) = *guard else {
        return Err("TRAKT_NOT_LOGGED_IN".to_string());
    };
    if !failed_access_token.is_empty() && current.access_token.as_str() != failed_access_token {
        return Ok(current.access_token.clone());
    }
    refresh_token_locked(state, &mut guard, app).await
}

async fn refresh_token_locked(
    state: &AppState,
    guard: &mut Option<TraktToken>,
    app: Option<&tauri::AppHandle>,
) -> Result<Zeroizing<String>, String> {
    let refresh_tok = match guard.as_ref() {
        Some(t) => t.refresh_token.clone(),
        None => return Err("TRAKT_NOT_LOGGED_IN".to_string()),
    };

    let (client_id, client_secret) = {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        (
            get_client_id(&conn),
            get_custom_client_secret(&conn, store.as_ref()),
        )
    };

    let url = format!("{}/oauth/token", state.trakt.auth_base);
    let mut payload = json!({
        "refresh_token": refresh_tok.as_str(),
        "client_id": client_id,
        "grant_type": "refresh_token",
    });
    if let Some(ref sec) = client_secret {
        if !sec.trim().is_empty() {
            payload["client_secret"] = json!(sec.trim());
        }
    }

    let resp = state
        .trakt
        .client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("trakt-api-version", "2")
        .header("trakt-api-key", &client_id)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Token frissítési hálózati hiba: {}", e))?;

    let status = resp.status().as_u16();
    if status == 200 {
        let tok_resp: TokenApiResponse = resp
            .json()
            .await
            .map_err(|e| format!("Token frissítési JSON hiba: {}", e))?;
        let base_ts = if tok_resp.created_at > 0 {
            tok_resp.created_at
        } else {
            now_epoch_secs()
        };
        let expires_at = base_ts + tok_resp.expires_in;

        let store = default_secret_store();
        {
            let mut conn = state.db.lock().await;
            save_tokens_tx(
                &mut conn,
                store.as_ref(),
                &tok_resp.access_token,
                &tok_resp.refresh_token,
                expires_at,
            )?;
        }

        let new_tok = TraktToken {
            access_token: Zeroizing::new(tok_resp.access_token),
            refresh_token: Zeroizing::new(tok_resp.refresh_token),
            expires_at,
        };
        let access = new_tok.access_token.clone();
        *guard = Some(new_tok);
        return Ok(access);
    }

    let body = resp.text().await.unwrap_or_default();
    let parsed_err = serde_json::from_str::<OAuthErrorBody>(&body).ok();
    let is_invalid_grant = status == 401
        || (status == 400
            && parsed_err
                .as_ref()
                .and_then(|b| b.error.as_deref())
                == Some("invalid_grant"));

    if is_invalid_grant {
        {
            let conn = state.db.lock().await;
            let _ = clear_tokens_from_db(&conn);
        }
        *guard = None;
        if let Some(app_handle) = app {
            let _ = app_handle.emit(
                "trakt-login-state",
                json!({ "status": "logged_out", "reason": "expired" }),
            );
        }
        return Err("TRAKT_AUTH_EXPIRED".to_string());
    }

    let desc = parsed_err
        .and_then(|b| b.error_description)
        .unwrap_or(body);
    Err(format!("Token frissítési hiba ({}): {}", status, desc))
}

pub async fn logout(state: &AppState, app: Option<&tauri::AppHandle>) -> Result<(), String> {
    cancel_device_flow(state).await;

    let (token_opt, client_id, client_secret) = {
        let store = default_secret_store();
        let mut guard = state.trakt.token.lock().await;
        let conn = state.db.lock().await;
        let tok = guard
            .take()
            .or_else(|| load_tokens_from_db(&conn, store.as_ref()).ok().flatten());
        let cid = get_client_id(&conn);
        let csec = get_custom_client_secret(&conn, store.as_ref());
        (tok, cid, csec)
    };

    if let Some(tok) = token_opt {
        let url = format!("{}/oauth/revoke", state.trakt.auth_base);
        let mut payload = json!({
            "token": tok.access_token.as_str(),
            "client_id": client_id,
        });
        if let Some(ref sec) = client_secret {
            if !sec.trim().is_empty() {
                payload["client_secret"] = json!(sec.trim());
            }
        }
        let _ = state
            .trakt
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("trakt-api-version", "2")
            .header("trakt-api-key", &client_id)
            .json(&payload)
            .send()
            .await;
    }

    {
        let mut conn = state.db.lock().await;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let has_secret = has_custom_client_secret(&tx);
        if has_secret {
            tx.execute(
                "UPDATE trakt_auth SET access_blob = '', refresh_blob = '', expires_at = 0 WHERE id = 1",
                [],
            )
            .ok();
        } else {
            tx.execute("DELETE FROM trakt_auth WHERE id = 1", []).ok();
        }
        tx.execute("DELETE FROM trakt_sync_state", []).ok();
        tx.execute("DELETE FROM trakt_watchlist_cache", []).ok();
        tx.execute("DELETE FROM trakt_history_cache", []).ok();
        tx.execute("DELETE FROM trakt_calendar_cache WHERE scope = 'my'", [])
            .ok();
        tx.execute("DELETE FROM trakt_episodes", []).ok();
        tx.execute("DELETE FROM trakt_show_progress", []).ok();
        tx.execute("DELETE FROM trakt_pending_actions", []).ok();
        tx.execute("DELETE FROM trakt_http_cache", []).ok();
        tx.commit().map_err(|e| e.to_string())?;
    }

    let new_ver = {
        let ver = state.trakt.watchlist_version.fetch_add(1, Ordering::SeqCst) + 1;
        let mut idx = state.trakt.title_index.write().await;
        *idx = crate::trakt::matcher::TitleIndex::default();
        let mut mc = state.trakt.match_cache.write().await;
        mc.0 = ver;
        mc.1.clear();
        ver
    };

    if let Some(app_handle) = app {
        let _ = app_handle.emit("trakt-login-state", json!({ "status": "logged_out" }));
        let _ = app_handle.emit("trakt-watchlist-changed", new_ver);
    }

    Ok(())
}
