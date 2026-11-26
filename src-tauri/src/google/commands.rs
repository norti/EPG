use crate::google;
use crate::google::auth::{GoogleAuthStartData, GoogleProfile};
use crate::google::calendar::{
    CalendarCleanupResult, CalendarEventRequest, CalendarEventResult, CalendarListEntry,
    CalendarSyncResult, TrackedCalendarEvent,
};
use crate::trakt::secret::default_secret_store;
use crate::{set_setting_in_conn, AppState};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAuthStatus {
    pub logged_in: bool,
    pub configured: bool,
    pub profile: Option<GoogleProfile>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSyncStateDto {
    pub last_drive_save_at: Option<String>,
    pub last_drive_load_at: Option<String>,
    pub last_remote_modified_at: Option<String>,
    pub auto_sync_enabled: bool,
    pub target_calendar_id: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DriveSaveResult {
    pub file_id: String,
    pub modified_time: Option<String>,
    pub saved_at: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DriveBackupPreview {
    pub timestamp: String,
    pub client_name: String,
    pub app_version: String,
    pub settings_count: usize,
    pub sources_count: usize,
    pub favorites_count: usize,
    pub presets_count: usize,
}

#[tauri::command]
pub async fn google_start_auth(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<GoogleAuthStartData, String> {
    google::auth::start_google_auth(&state, app).await
}

#[tauri::command]
pub async fn google_cancel_auth(state: tauri::State<'_, AppState>) -> Result<(), String> {
    google::auth::cancel_google_auth(&state).await;
    Ok(())
}

#[tauri::command]
pub async fn google_logout(state: tauri::State<'_, AppState>) -> Result<(), String> {
    google::auth::google_logout(&state).await
}

#[tauri::command]
pub async fn google_get_auth_status(
    state: tauri::State<'_, AppState>,
) -> Result<GoogleAuthStatus, String> {
    let store = default_secret_store();
    let has_mem_token = state.google.token.lock().await.is_some();

    let (configured, profile, loaded_from_db) = {
        let conn = state.db.lock().await;
        let configured =
            google::auth::get_google_client_credentials(&conn, store.as_ref()).is_ok();
        let profile: Option<GoogleProfile> = conn
            .query_row(
                "SELECT value FROM google_sync_state WHERE key = 'profile_json'",
                [],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());

        let loaded = if has_mem_token {
            None
        } else {
            google::auth::load_google_tokens_from_db(&conn, store.as_ref())
                .ok()
                .flatten()
        };
        (configured, profile, loaded)
    };

    let logged_in = if has_mem_token {
        true
    } else if let Some(tok) = loaded_from_db {
        let mut guard = state.google.token.lock().await;
        *guard = Some(tok);
        true
    } else {
        false
    };

    Ok(GoogleAuthStatus {
        logged_in,
        configured,
        profile: if logged_in { profile } else { None },
    })
}

#[tauri::command]
pub async fn google_save_to_drive(
    state: tauri::State<'_, AppState>,
) -> Result<DriveSaveResult, String> {
    let meta = google::drive::save_to_drive(&state).await?;
    Ok(DriveSaveResult {
        file_id: meta.id,
        modified_time: meta.modified_time,
        saved_at: crate::trakt::now_iso_utc(),
    })
}

#[tauri::command]
pub async fn google_load_from_drive(
    state: tauri::State<'_, AppState>,
) -> Result<DriveBackupPreview, String> {
    let payload = google::drive::load_and_stage_from_drive(&state).await?;
    Ok(DriveBackupPreview {
        timestamp: payload.timestamp,
        client_name: payload.client_name,
        app_version: payload.app_version,
        settings_count: payload.backup.settings.len(),
        sources_count: payload.backup.sources.len(),
        favorites_count: payload.backup.favorites.len(),
        presets_count: payload.backup.channel_sort_presets.len(),
    })
}

#[tauri::command]
pub async fn google_apply_drive_data(state: tauri::State<'_, AppState>) -> Result<(), String> {
    google::drive::apply_staged_drive_data(&state).await
}

#[tauri::command]
pub async fn google_set_auto_sync(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let conn = state.db.lock().await;
    set_setting_in_conn(
        &conn,
        "google_auto_sync_enabled",
        if enabled { "1" } else { "0" },
    )
}

#[tauri::command]
pub async fn google_add_calendar_event(
    state: tauri::State<'_, AppState>,
    req: CalendarEventRequest,
) -> Result<CalendarEventResult, String> {
    google::calendar::add_calendar_event(&state, req).await
}

#[tauri::command]
pub async fn google_remove_calendar_event(
    state: tauri::State<'_, AppState>,
    epg_key: String,
) -> Result<bool, String> {
    google::calendar::remove_calendar_event(&state, &epg_key).await
}

#[tauri::command]
pub async fn google_get_calendar_event(
    state: tauri::State<'_, AppState>,
    epg_key: String,
) -> Result<Option<TrackedCalendarEvent>, String> {
    let conn = state.db.lock().await;
    Ok(google::calendar::get_tracked_event_in_conn(&conn, &epg_key))
}

#[tauri::command]
pub async fn google_get_calendar_event_keys(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let conn = state.db.lock().await;
    Ok(google::calendar::get_all_tracked_event_keys_in_conn(&conn))
}

#[tauri::command]
pub async fn google_get_calendar_events_count(
    state: tauri::State<'_, AppState>,
) -> Result<i64, String> {
    let conn = state.db.lock().await;
    Ok(google::calendar::count_tracked_events_in_conn(&conn))
}

#[tauri::command]
pub async fn google_sync_calendar_events(
    state: tauri::State<'_, AppState>,
    force_full: Option<bool>,
) -> Result<CalendarSyncResult, String> {
    google::calendar::sync_calendar_events(&state, force_full.unwrap_or(false)).await
}

#[tauri::command]
pub async fn google_cleanup_calendar_events(
    state: tauri::State<'_, AppState>,
    older_than_days: Option<i64>,
) -> Result<CalendarCleanupResult, String> {
    google::calendar::cleanup_calendar_events(&state, older_than_days).await
}

#[tauri::command]
pub async fn google_list_calendars(
    state: tauri::State<'_, AppState>,
    force_refresh: Option<bool>,
) -> Result<Vec<CalendarListEntry>, String> {
    google::calendar::list_writable_calendars(&state, force_refresh.unwrap_or(false)).await
}

#[tauri::command]
pub async fn google_set_target_calendar(
    state: tauri::State<'_, AppState>,
    calendar_id: String,
) -> Result<(), String> {
    let clean_id = calendar_id.trim();
    if clean_id.is_empty() {
        return Err("Érvénytelen naptár azonosító".to_string());
    }
    if clean_id != "primary" {
        let token = google::auth::ensure_valid_google_token(&state).await?;
        google::calendar::verify_calendar_exists(&state, token.as_str(), clean_id).await?;
    }
    let conn = state.db.lock().await;
    set_setting_in_conn(&conn, "google_calendar_target_id", clean_id)
}

#[tauri::command]
pub async fn google_get_sync_state(
    state: tauri::State<'_, AppState>,
) -> Result<GoogleSyncStateDto, String> {
    let conn = state.db.lock().await;
    let last_drive_save_at: Option<String> = conn
        .query_row(
            "SELECT value FROM google_sync_state WHERE key = 'last_drive_save_at'",
            [],
            |r| r.get(0),
        )
        .ok();
    let last_drive_load_at: Option<String> = conn
        .query_row(
            "SELECT value FROM google_sync_state WHERE key = 'last_drive_load_at'",
            [],
            |r| r.get(0),
        )
        .ok();
    let last_remote_modified_at: Option<String> = conn
        .query_row(
            "SELECT value FROM google_sync_state WHERE key = 'last_remote_modified_at'",
            [],
            |r| r.get(0),
        )
        .ok();
    let auto_sync_enabled: bool = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'google_auto_sync_enabled'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .is_some_and(|v| matches!(v.as_str(), "1" | "true"));
    let target_calendar_id: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'google_calendar_target_id'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "primary".to_string());

    Ok(GoogleSyncStateDto {
        last_drive_save_at,
        last_drive_load_at,
        last_remote_modified_at,
        auto_sync_enabled,
        target_calendar_id,
    })
}
