use crate::google::auth::{ensure_valid_google_token, force_refresh_google_token};
use crate::google::StagedDriveDownload;
use crate::trakt::now_iso_utc;
use crate::{export_settings_from_conn, import_settings_into_conn, AppState, BackupData};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

const DRIVE_FILE_NAME: &str = "epg_viewer_sync.json";
const DRIVE_SPACES: &str = "appDataFolder";
pub(crate) const CURRENT_SYNC_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncPayload {
    pub sync_schema_version: u32,
    pub timestamp: String,
    pub client_name: String,
    pub app_version: String,
    pub backup: BackupData,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DriveFileMeta {
    pub id: String,
    pub modified_time: Option<String>,
}

fn get_sync_state(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM google_sync_state WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .ok()
}

fn set_sync_state(conn: &Connection, key: &str, value: &str) {
    let _ = conn.execute(
        "INSERT INTO google_sync_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    );
}

fn clear_cached_drive_file_id(conn: &Connection) {
    let _ = conn.execute("DELETE FROM google_sync_state WHERE key = 'drive_file_id'", []);
}

pub(crate) fn build_sync_payload_from_conn(conn: &Connection) -> Result<SyncPayload, String> {
    let backup = export_settings_from_conn(conn)?;
    Ok(SyncPayload {
        sync_schema_version: CURRENT_SYNC_SCHEMA_VERSION,
        timestamp: now_iso_utc(),
        client_name: "EPG-Viewer-Desktop".to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        backup,
    })
}

pub(crate) async fn build_sync_payload(state: &AppState) -> Result<SyncPayload, String> {
    let conn = state.db.lock().await;
    build_sync_payload_from_conn(&conn)
}

pub async fn fetch_remote_file_meta(state: &AppState) -> Result<Option<DriveFileMeta>, String> {
    let mut token = ensure_valid_google_token(state).await?;

    for attempt in 0..2 {
        let resp = state
            .google
            .client
            .get("https://www.googleapis.com/drive/v3/files")
            .bearer_auth(token.as_str())
            .query(&[
                ("spaces", DRIVE_SPACES),
                ("q", &format!("name='{DRIVE_FILE_NAME}' and trashed=false")),
                ("fields", "files(id,name,modifiedTime)"),
                ("orderBy", "modifiedTime desc"),
                ("pageSize", "1"),
            ])
            .send()
            .await
            .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            token = force_refresh_google_token(state, token.as_str()).await?;
            continue;
        }
        if !resp.status().is_success() {
            return Err(match status {
                401 => "GOOGLE_SESSION_EXPIRED".to_string(),
                _ => format!("Drive keresés sikertelen ({status})"),
            });
        }

        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let meta = body["files"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|f| {
                Some(DriveFileMeta {
                    id: f["id"].as_str()?.to_string(),
                    modified_time: f["modifiedTime"].as_str().map(str::to_string),
                })
            });

        let conn = state.db.lock().await;
        match &meta {
            Some(m) => set_sync_state(&conn, "drive_file_id", &m.id),
            None => clear_cached_drive_file_id(&conn),
        }
        return Ok(meta);
    }

    Err("GOOGLE_SESSION_EXPIRED".to_string())
}

pub async fn save_to_drive(state: &AppState) -> Result<DriveFileMeta, String> {
    let _sync_guard = state.google.sync_lock.lock().await;
    let mut token = ensure_valid_google_token(state).await?;
    let payload = build_sync_payload(state).await?;
    let json_bytes = serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?;

    let cached_id = {
        let conn = state.db.lock().await;
        get_sync_state(&conn, "drive_file_id")
    };
    let existing_id = match cached_id {
        Some(id) => Some(id),
        None => fetch_remote_file_meta(state).await?.map(|m| m.id),
    };

    let meta = match existing_id {
        Some(id) => {
            let mut updated: Option<DriveFileMeta> = None;
            for attempt in 0..2 {
                let resp = state
                    .google
                    .client
                    .patch(format!(
                        "https://www.googleapis.com/upload/drive/v3/files/{id}"
                    ))
                    .bearer_auth(token.as_str())
                    .query(&[("uploadType", "media"), ("fields", "id,modifiedTime")])
                    .header("Content-Type", "application/json")
                    .body(json_bytes.clone())
                    .send()
                    .await
                    .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

                let status = resp.status().as_u16();
                if status == 401 && attempt == 0 {
                    token = force_refresh_google_token(state, token.as_str()).await?;
                    continue;
                }
                if status == 404 {
                    {
                        let conn = state.db.lock().await;
                        clear_cached_drive_file_id(&conn);
                    }
                    updated = Some(create_drive_file(state, token.as_str(), json_bytes.clone()).await?);
                    break;
                }
                if !resp.status().is_success() {
                    return Err(match status {
                        401 => "GOOGLE_SESSION_EXPIRED".to_string(),
                        _ => format!("Drive frissítés sikertelen ({status})"),
                    });
                }
                let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
                updated = Some(DriveFileMeta {
                    id: body["id"].as_str().unwrap_or(&id).to_string(),
                    modified_time: body["modifiedTime"].as_str().map(str::to_string),
                });
                break;
            }
            updated.ok_or_else(|| "GOOGLE_SESSION_EXPIRED".to_string())?
        }
        None => create_drive_file(state, token.as_str(), json_bytes).await?,
    };

    let saved_at = now_iso_utc();
    {
        let conn = state.db.lock().await;
        set_sync_state(&conn, "drive_file_id", &meta.id);
        set_sync_state(&conn, "last_drive_save_at", &saved_at);
        if let Some(ref mt) = meta.modified_time {
            set_sync_state(&conn, "last_remote_modified_at", mt);
        }
    }

    Ok(meta)
}

async fn create_drive_file(
    state: &AppState,
    token: &str,
    json_bytes: Vec<u8>,
) -> Result<DriveFileMeta, String> {
    use reqwest::multipart;

    let metadata = serde_json::json!({
        "name": DRIVE_FILE_NAME,
        "parents": [DRIVE_SPACES],
        "mimeType": "application/json"
    });

    let form = multipart::Form::new()
        .part(
            "metadata",
            multipart::Part::text(metadata.to_string())
                .mime_str("application/json")
                .map_err(|e| e.to_string())?,
        )
        .part(
            "media",
            multipart::Part::bytes(json_bytes)
                .mime_str("application/json")
                .map_err(|e| e.to_string())?,
        );

    let resp = state
        .google
        .client
        .post("https://www.googleapis.com/upload/drive/v3/files")
        .bearer_auth(token)
        .query(&[("uploadType", "multipart"), ("fields", "id,modifiedTime")])
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(match status {
            401 => "GOOGLE_SESSION_EXPIRED".to_string(),
            _ => format!("Drive fájl létrehozás sikertelen ({status})"),
        });
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let id = body["id"]
        .as_str()
        .ok_or_else(|| "Hiányzó fájl ID a Drive válaszból".to_string())?
        .to_string();
    let modified_time = body["modifiedTime"].as_str().map(str::to_string);

    Ok(DriveFileMeta { id, modified_time })
}

pub(crate) async fn load_and_stage_from_drive(state: &AppState) -> Result<SyncPayload, String> {
    let _sync_guard = state.google.sync_lock.lock().await;
    let mut token = ensure_valid_google_token(state).await?;

    let meta = fetch_remote_file_meta(state)
        .await?
        .ok_or_else(|| "GOOGLE_DRIVE_NO_BACKUP".to_string())?;

    for attempt in 0..2 {
        let resp = state
            .google
            .client
            .get(format!("https://www.googleapis.com/drive/v3/files/{}", meta.id))
            .bearer_auth(token.as_str())
            .query(&[("alt", "media")])
            .send()
            .await
            .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            token = force_refresh_google_token(state, token.as_str()).await?;
            continue;
        }
        if status == 404 {
            let conn = state.db.lock().await;
            clear_cached_drive_file_id(&conn);
            return Err("GOOGLE_DRIVE_NO_BACKUP".to_string());
        }
        if !resp.status().is_success() {
            return Err(format!("Drive letöltés sikertelen ({status})"));
        }

        let payload: SyncPayload = resp.json().await.map_err(|e| e.to_string())?;
        if payload.sync_schema_version > CURRENT_SYNC_SCHEMA_VERSION {
            return Err(format!(
                "A felhős mentés verziója ({}) újabb az alkalmazásnál ({}). Frissítse az alkalmazást.",
                payload.sync_schema_version, CURRENT_SYNC_SCHEMA_VERSION
            ));
        }

        let mut staged = state.google.staged_download.lock().await;
        *staged = Some(StagedDriveDownload {
            file_id: meta.id,
            modified_time: meta.modified_time,
            payload: payload.clone(),
        });

        return Ok(payload);
    }

    Err("GOOGLE_SESSION_EXPIRED".to_string())
}

pub async fn apply_staged_drive_data(state: &AppState) -> Result<(), String> {
    let staged = {
        let mut guard = state.google.staged_download.lock().await;
        guard.take()
    };
    let staged = match staged {
        Some(s) => s,
        None => {
            let _ = load_and_stage_from_drive(state).await?;
            let mut guard = state.google.staged_download.lock().await;
            guard
                .take()
                .ok_or_else(|| "GOOGLE_DRIVE_NO_BACKUP".to_string())?
        }
    };

    let _sync_guard = state.google.sync_lock.lock().await;
    let loaded_at = now_iso_utc();
    let mut conn = state.db.lock().await;
    import_settings_into_conn(&mut conn, staged.payload.backup)?;

    set_sync_state(&conn, "drive_file_id", &staged.file_id);
    set_sync_state(&conn, "last_drive_load_at", &loaded_at);
    if let Some(ref mt) = staged.modified_time {
        set_sync_state(&conn, "last_remote_modified_at", mt);
    }
    Ok(())
}
