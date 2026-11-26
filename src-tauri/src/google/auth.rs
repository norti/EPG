use crate::google::GoogleToken;
use crate::trakt::secret::{default_secret_store, protect_str, unprotect_str, SecretStore};
use crate::trakt::{now_epoch_secs, now_iso_utc};
use crate::AppState;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use zeroize::Zeroizing;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAuthStartData {
    pub auth_url: String,
    pub port: u16,
    pub opened: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleProfile {
    pub sub: Option<String>,
    pub email: String,
    pub name: String,
    pub picture: Option<String>,
}

pub struct PkceData {
    pub code_verifier: String,
    pub code_challenge: String,
}

#[derive(Deserialize)]
pub(crate) struct GoogleTokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub id_token: Option<String>,
}

#[derive(Deserialize)]
struct GoogleOAuthError {
    error: Option<String>,
}

pub enum CallbackOutcome {
    Code(String),
    OAuthError(String),
    Cancelled,
}

pub fn get_google_client_credentials(
    conn: &Connection,
    store: &dyn SecretStore,
) -> Result<(String, Zeroizing<String>), String> {
    let custom_id: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'google_client_id_override'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let custom_secret: Option<Zeroizing<String>> = conn
        .query_row(
            "SELECT client_secret_blob FROM google_auth WHERE id = 1",
            [],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .and_then(|b| unprotect_str(store, &b).ok())
        .filter(|s| !s.trim().is_empty());

    let client_id = custom_id
        .or_else(|| {
            option_env!("GOOGLE_CLIENT_ID")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| "GOOGLE_NOT_CONFIGURED".to_string())?;

    let client_secret = custom_secret
        .or_else(|| {
            option_env!("GOOGLE_CLIENT_SECRET")
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| Zeroizing::new(s.to_string()))
        })
        .ok_or_else(|| "GOOGLE_NOT_CONFIGURED".to_string())?;

    Ok((client_id, client_secret))
}

pub fn generate_pkce() -> PkceData {
    let mut verifier_bytes = [0u8; 64];
    OsRng.fill_bytes(&mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let challenge_bytes = Sha256::digest(code_verifier.as_bytes());
    let code_challenge = URL_SAFE_NO_PAD.encode(challenge_bytes);

    PkceData {
        code_verifier,
        code_challenge,
    }
}

pub fn generate_state_nonce() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn parse_callback_target(
    request_target: &str,
    expected_state: &str,
) -> Option<Result<String, String>> {
    let parsed = reqwest::Url::parse(&format!("http://127.0.0.1{}", request_target)).ok()?;
    if parsed.path() != "/callback" {
        return None;
    }

    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut error: Option<String> = None;

    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }

    if let Some(err) = error {
        return Some(Err(format!("Google hozzáférés megtagadva: {err}")));
    }
    if state.as_deref() != Some(expected_state) {
        return Some(Err(
            "Érvénytelen OAuth state paraméter (CSRF ellenőrzés sikertelen)".into(),
        ));
    }
    match code.filter(|c| !c.is_empty()) {
        Some(c) => Some(Ok(c)),
        None => Some(Err("Hiányzó auth kód a callback kérésben".into())),
    }
}

async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await.ok()?;
    if n == 0 {
        return None;
    }
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next()?;
    first_line.split_whitespace().nth(1).map(str::to_string)
}

async fn send_http_response(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub async fn start_loopback_server(
    expected_state: String,
    mut cancel_rx: watch::Receiver<bool>,
) -> Result<(u16, oneshot::Receiver<CallbackOutcome>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Nem sikerült loopback szervert indítani: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let (result_tx, result_rx) = oneshot::channel::<CallbackOutcome>();

    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                accept_res = listener.accept() => {
                    let Ok((mut stream, _)) = accept_res else {
                        break;
                    };
                    let Some(target) = read_request_target(&mut stream).await else {
                        continue;
                    };
                    match parse_callback_target(&target, &expected_state) {
                        None => {
                            send_http_response(&mut stream, "404 Not Found", "Not Found").await;
                        }
                        Some(Ok(code)) => {
                            let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>EPG Viewer</title></head>\
                                <body style=\"font-family:sans-serif;background:#181920;color:#fff;padding:40px;text-align:center\">\
                                <h2>Sikeres Google bejelentkezés!</h2>\
                                <p>Bezárhatja ezt a lapot, és visszatérhet az EPG Viewer alkalmazáshoz.</p></body></html>";
                            send_http_response(&mut stream, "200 OK", html).await;
                            let _ = result_tx.send(CallbackOutcome::Code(code));
                            break;
                        }
                        Some(Err(err_msg)) => {
                            let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>EPG Viewer</title></head>\
                                <body style=\"font-family:sans-serif;background:#181920;color:#fff;padding:40px;text-align:center\">\
                                <h2>Bejelentkezés sikertelen</h2>\
                                <p>Bezárhatja ezt a lapot, és újrapróbálhatja az EPG Viewer alkalmazásban.</p></body></html>";
                            send_http_response(&mut stream, "400 Bad Request", html).await;
                            let _ = result_tx.send(CallbackOutcome::OAuthError(err_msg));
                            break;
                        }
                    }
                }
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        let _ = result_tx.send(CallbackOutcome::Cancelled);
                        break;
                    }
                }
            }
        }
    });

    Ok((port, result_rx))
}

pub async fn cancel_google_auth(state: &AppState) {
    let mut guard = state.google.auth_cancel.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(true);
    }
}

pub async fn start_google_auth(
    state: &AppState,
    app: tauri::AppHandle,
) -> Result<GoogleAuthStartData, String> {
    cancel_google_auth(state).await;

    // Fast path: avoid blocking behind background DB writes if compiled-in credentials exist.
    let (client_id, _client_secret) = if let Ok(conn) = state.db.try_lock() {
        let store = default_secret_store();
        get_google_client_credentials(&conn, store.as_ref())?
    } else if let (Some(id), Some(sec)) = (
        option_env!("GOOGLE_CLIENT_ID")
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        option_env!("GOOGLE_CLIENT_SECRET")
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (id.to_string(), Zeroizing::new(sec.to_string()))
    } else {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        get_google_client_credentials(&conn, store.as_ref())?
    };

    let pkce = generate_pkce();
    let csrf_state = generate_state_nonce();

    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (port, result_rx) = start_loopback_server(csrf_state.clone(), cancel_rx).await?;
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let scopes = [
        "openid",
        "https://www.googleapis.com/auth/userinfo.profile",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/drive.appdata",
        "https://www.googleapis.com/auth/calendar.events",
        "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
    ]
    .join(" ");

    let auth_url = reqwest::Url::parse_with_params(
        "https://accounts.google.com/o/oauth2/v2/auth",
        &[
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("response_type", "code"),
            ("scope", scopes.as_str()),
            ("code_challenge", pkce.code_challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", csrf_state.as_str()),
        ],
    )
    .map_err(|e| e.to_string())?
    .to_string();

    {
        let mut guard = state.google.auth_cancel.lock().await;
        *guard = Some(cancel_tx);
    }

    // Launch browser immediately in a detached blocking task (saves a second IPC roundtrip
    // and avoids blocking the Tauri command thread on Windows ShellExecuteW).
    let url_to_open = auth_url.clone();
    let open_res = tokio::task::spawn_blocking(move || open::that_detached(&url_to_open))
        .await
        .ok()
        .and_then(|r| r.ok())
        .is_some();

    let verifier = pkce.code_verifier;
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let st = app_handle.state::<AppState>();
        let outcome = match tokio::time::timeout(Duration::from_secs(300), result_rx).await {
            Ok(Ok(o)) => o,
            Ok(Err(_)) => CallbackOutcome::Cancelled,
            Err(_) => {
                cancel_google_auth(st.inner()).await;
                CallbackOutcome::OAuthError("google_login_timeout".to_string())
            }
        };

        {
            let mut guard = st.google.auth_cancel.lock().await;
            *guard = None;
        }

        match outcome {
            CallbackOutcome::Code(code) => {
                match exchange_code_for_tokens(st.inner(), &code, &verifier, &redirect_uri).await {
                    Ok(tokens) => {
                        if let Err(e) = save_google_tokens(st.inner(), &tokens).await {
                            let _ = app_handle.emit(
                                "google-auth-state",
                                json!({ "status": "error", "message": e }),
                            );
                            return;
                        }
                        let profile = fetch_and_save_google_profile(st.inner()).await.ok();
                        let _ = app_handle.emit(
                            "google-auth-state",
                            json!({ "status": "ok", "profile": profile }),
                        );
                    }
                    Err(e) => {
                        let _ = app_handle.emit(
                            "google-auth-state",
                            json!({ "status": "error", "message": e }),
                        );
                    }
                }
            }
            CallbackOutcome::OAuthError(msg) => {
                let _ = app_handle.emit(
                    "google-auth-state",
                    json!({ "status": "error", "message": msg }),
                );
            }
            CallbackOutcome::Cancelled => {
                let _ = app_handle.emit("google-auth-state", json!({ "status": "cancelled" }));
            }
        }
    });

    Ok(GoogleAuthStartData {
        auth_url,
        port,
        opened: open_res,
    })
}

async fn exchange_code_for_tokens(
    state: &AppState,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResponse, String> {
    let (client_id, client_secret) = {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        get_google_client_credentials(&conn, store.as_ref())?
    };

    let resp = state
        .google
        .client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

    if !resp.status().is_success() {
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!("Token csere sikertelen: {err_body}"));
    }

    resp.json::<GoogleTokenResponse>()
        .await
        .map_err(|e| e.to_string())
}

pub(crate) fn save_google_tokens_in_conn(
    conn: &mut Connection,
    store: &dyn SecretStore,
    tokens: &GoogleTokenResponse,
) -> Result<GoogleToken, String> {
    let expires_at = now_epoch_secs() + tokens.expires_in - 60;
    let access_blob = protect_str(store, &tokens.access_token)?;

    let (refresh_plain, refresh_blob) = if let Some(ref rt) = tokens.refresh_token {
        (Zeroizing::new(rt.clone()), protect_str(store, rt)?)
    } else {
        let existing_blob: String = conn
            .query_row(
                "SELECT refresh_blob FROM google_auth WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "Hiányzó Google refresh token".to_string())?;
        let existing_plain = unprotect_str(store, &existing_blob)?;
        (existing_plain, existing_blob)
    };

    let id_sub = tokens.id_token.as_deref().and_then(extract_jwt_sub);
    let now = now_iso_utc();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO google_auth (id, access_blob, refresh_blob, client_secret_blob, expires_at, id_token_sub, created_at)
         VALUES (1, ?1, ?2, NULL, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
            access_blob = excluded.access_blob,
            refresh_blob = excluded.refresh_blob,
            expires_at = excluded.expires_at,
            id_token_sub = COALESCE(excluded.id_token_sub, google_auth.id_token_sub),
            created_at = excluded.created_at",
        params![access_blob, refresh_blob, expires_at, id_sub, now],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(GoogleToken {
        access_token: Zeroizing::new(tokens.access_token.clone()),
        refresh_token: refresh_plain,
        expires_at,
    })
}

async fn save_google_tokens(state: &AppState, tokens: &GoogleTokenResponse) -> Result<(), String> {
    let store = default_secret_store();
    let mem_tok = {
        let mut conn = state.db.lock().await;
        save_google_tokens_in_conn(&mut conn, store.as_ref(), tokens)?
    };
    let mut guard = state.google.token.lock().await;
    *guard = Some(mem_tok);
    Ok(())
}

pub async fn fetch_and_save_google_profile(state: &AppState) -> Result<GoogleProfile, String> {
    let token = ensure_valid_google_token(state).await?;
    let resp = state
        .google
        .client
        .get("https://www.googleapis.com/oauth2/v3/userinfo")
        .bearer_auth(token.as_str())
        .send()
        .await
        .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Google profil lekérés sikertelen ({})", resp.status()));
    }

    let val: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let email = val["email"].as_str().unwrap_or("").to_string();
    let name = val["name"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if email.is_empty() {
                "Google User".to_string()
            } else {
                email.clone()
            }
        });
    let profile = GoogleProfile {
        sub: val["sub"].as_str().map(str::to_string),
        email,
        name,
        picture: val["picture"].as_str().map(str::to_string),
    };

    let json_str = serde_json::to_string(&profile).map_err(|e| e.to_string())?;
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO google_sync_state (key, value) VALUES ('profile_json', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![json_str],
    )
    .map_err(|e| e.to_string())?;

    Ok(profile)
}

fn extract_jwt_sub(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let val: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    val["sub"].as_str().map(str::to_string)
}

fn clear_google_tokens_in_conn(conn: &Connection) -> Result<(), String> {
    let has_custom_secret: bool = conn
        .query_row(
            "SELECT client_secret_blob IS NOT NULL AND TRIM(client_secret_blob) != '' FROM google_auth WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);

    if has_custom_secret {
        conn.execute(
            "UPDATE google_auth SET access_blob = '', refresh_blob = '', expires_at = 0, id_token_sub = NULL WHERE id = 1",
            [],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute("DELETE FROM google_auth WHERE id = 1", [])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn load_google_tokens_from_db(
    conn: &Connection,
    store: &dyn SecretStore,
) -> Result<Option<GoogleToken>, String> {
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT access_blob, refresh_blob, expires_at FROM google_auth WHERE id = 1",
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

    match (
        unprotect_str(store, &access_blob),
        unprotect_str(store, &refresh_blob),
    ) {
        (Ok(access_token), Ok(refresh_token)) => Ok(Some(GoogleToken {
            access_token,
            refresh_token,
            expires_at,
        })),
        _ => {
            // Unreadable DPAPI blob (e.g. DB moved to another Windows account/machine): clear cleanly.
            let _ = clear_google_tokens_in_conn(conn);
            let _ = conn.execute("DELETE FROM google_sync_state", []);
            let _ = conn.execute("DELETE FROM google_calendar_events", []);
            Ok(None)
        }
    }
}

pub async fn ensure_valid_google_token(state: &AppState) -> Result<Zeroizing<String>, String> {
    let mut guard = state.google.token.lock().await;
    if guard.is_none() {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        *guard = load_google_tokens_from_db(&conn, store.as_ref())?;
    }

    let Some(ref tok) = *guard else {
        return Err("GOOGLE_NOT_LOGGED_IN".to_string());
    };

    if tok.expires_at > now_epoch_secs() + 300 {
        return Ok(tok.access_token.clone());
    }

    refresh_token_locked(state, &mut guard).await
}

pub async fn force_refresh_google_token(
    state: &AppState,
    failed_access_token: &str,
) -> Result<Zeroizing<String>, String> {
    let mut guard = state.google.token.lock().await;
    if guard.is_none() {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        *guard = load_google_tokens_from_db(&conn, store.as_ref())?;
    }
    let Some(ref current) = *guard else {
        return Err("GOOGLE_NOT_LOGGED_IN".to_string());
    };
    if !failed_access_token.is_empty() && current.access_token.as_str() != failed_access_token {
        return Ok(current.access_token.clone());
    }
    refresh_token_locked(state, &mut guard).await
}

async fn refresh_token_locked(
    state: &AppState,
    guard: &mut Option<GoogleToken>,
) -> Result<Zeroizing<String>, String> {
    let refresh_token = match guard.as_ref() {
        Some(t) => t.refresh_token.clone(),
        None => return Err("GOOGLE_NOT_LOGGED_IN".to_string()),
    };

    let (client_id, client_secret) = {
        let store = default_secret_store();
        let conn = state.db.lock().await;
        get_google_client_credentials(&conn, store.as_ref())?
    };

    let resp = state
        .google
        .client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("refresh_token", refresh_token.as_str()),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

    let status = resp.status().as_u16();
    if status == 200 {
        let new_tokens: GoogleTokenResponse = resp.json().await.map_err(|e| e.to_string())?;
        let store = default_secret_store();
        let updated = {
            let mut conn = state.db.lock().await;
            save_google_tokens_in_conn(&mut conn, store.as_ref(), &new_tokens)?
        };
        let access = updated.access_token.clone();
        *guard = Some(updated);
        return Ok(access);
    }

    let body = resp.text().await.unwrap_or_default();
    let parsed_err = serde_json::from_str::<GoogleOAuthError>(&body).ok();
    let is_invalid_grant = status == 401
        || (status == 400
            && parsed_err.as_ref().and_then(|e| e.error.as_deref()) == Some("invalid_grant"));

    if is_invalid_grant {
        *guard = None;
        let conn = state.db.lock().await;
        let _ = clear_google_tokens_in_conn(&conn);
        let _ = conn.execute("DELETE FROM google_sync_state", []);
        return Err("GOOGLE_SESSION_EXPIRED".to_string());
    }

    Err(format!("Google token frissítés sikertelen ({status}): {body}"))
}

pub async fn google_logout(state: &AppState) -> Result<(), String> {
    cancel_google_auth(state).await;

    let token_to_revoke: Option<Zeroizing<String>> = {
        let mut guard = state.google.token.lock().await;
        if let Some(t) = guard.take() {
            Some(t.refresh_token)
        } else {
            let store = default_secret_store();
            let conn = state.db.lock().await;
            load_google_tokens_from_db(&conn, store.as_ref())
                .ok()
                .flatten()
                .map(|t| t.refresh_token)
        }
    };

    if let Some(tok) = token_to_revoke {
        let _ = state
            .google
            .client
            .post("https://oauth2.googleapis.com/revoke")
            .form(&[("token", tok.as_str())])
            .send()
            .await;
    }

    {
        let mut staged = state.google.staged_download.lock().await;
        *staged = None;
    }

    let conn = state.db.lock().await;
    clear_google_tokens_in_conn(&conn)?;
    let _ = conn.execute("DELETE FROM google_sync_state", []);
    let _ = conn.execute("DELETE FROM google_pending_actions", []);
    let _ = conn.execute("DELETE FROM google_http_cache", []);
    let _ = conn.execute("DELETE FROM google_calendar_events", []);
    Ok(())
}
