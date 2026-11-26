use crate::google::auth::{ensure_valid_google_token, force_refresh_google_token};
use crate::trakt::{iso_utc_to_epoch, now_epoch_secs, now_iso_utc};
use crate::AppState;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const CALENDAR_LIST_CACHE_KEY: &str = "google:calendarList:writer";
const CALENDAR_LIST_TTL_SECS: i64 = 300;
const APP_DISPLAY_NAME: &str = "EPG Viewer";
const APP_REPO_URL: &str = "https://github.com/norti/epg-viewer";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CalendarListEntry {
    pub id: String,
    pub summary: String,
    pub background_color: Option<String>,
    pub primary: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventRequest {
    pub epg_key: String,
    pub source_id: String,
    pub channel_id: String,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub summary: String,
    pub channel_name: String,
    pub source_name: String,
    pub programme_desc: Option<String>,
    pub tmdb_rating: Option<f64>,
    pub tmdb_year: Option<String>,
    pub start_datetime: String,
    pub end_datetime: String,
    pub timezone: String,
    pub reminder_minutes: i32,
    pub calendar_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventResult {
    pub event_id: String,
    pub html_link: String,
    pub calendar_id: String,
    pub calendar_name: Option<String>,
    pub already_exists: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrackedCalendarEvent {
    pub epg_key: String,
    pub source_id: String,
    pub channel_id: String,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub title: String,
    pub calendar_id: String,
    pub calendar_name: Option<String>,
    pub event_id: String,
    pub html_link: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSyncResult {
    pub synced_calendars: usize,
    pub added_or_updated: usize,
    pub removed: usize,
    pub total_tracked: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CalendarCleanupResult {
    pub deleted_count: usize,
    pub failed_count: usize,
    pub remaining_count: i64,
}

pub fn build_calendar_events_url(calendar_id: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse("https://www.googleapis.com/calendar/v3/calendars")
        .map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Érvénytelen Calendar alap URL".to_string())?
        .push(calendar_id)
        .push("events");
    Ok(url)
}

pub fn build_calendar_event_url(calendar_id: &str, event_id: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse("https://www.googleapis.com/calendar/v3/calendars")
        .map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Érvénytelen Calendar alap URL".to_string())?
        .push(calendar_id)
        .push("events")
        .push(event_id);
    Ok(url)
}

pub fn build_calendar_list_entry_url(calendar_id: &str) -> Result<reqwest::Url, String> {
    let mut url =
        reqwest::Url::parse("https://www.googleapis.com/calendar/v3/users/me/calendarList")
            .map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Érvénytelen CalendarList alap URL".to_string())?
        .push(calendar_id);
    Ok(url)
}

pub async fn list_writable_calendars(
    state: &AppState,
    force_refresh: bool,
) -> Result<Vec<CalendarListEntry>, String> {
    if !force_refresh {
        let conn = state.db.lock().await;
        if let Ok((body, fetched_at, ttl)) = conn.query_row(
            "SELECT body, fetched_at, ttl_seconds FROM google_http_cache WHERE key = ?1",
            params![CALENDAR_LIST_CACHE_KEY],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        ) {
            if iso_utc_to_epoch(&fetched_at).is_some_and(|ts| ts + ttl > now_epoch_secs()) {
                if let Ok(cached) = serde_json::from_str::<Vec<CalendarListEntry>>(&body) {
                    return Ok(cached);
                }
            }
        }
    }

    let mut token = ensure_valid_google_token(state).await?;

    for attempt in 0..2 {
        let resp = state
            .google
            .client
            .get("https://www.googleapis.com/calendar/v3/users/me/calendarList")
            .bearer_auth(token.as_str())
            .query(&[
                ("minAccessRole", "writer"),
                ("fields", "items(id,summary,backgroundColor,primary)"),
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
                403 => "GOOGLE_CALENDAR_NO_PERMISSION".to_string(),
                _ => format!("Naptárlista lekérés sikertelen ({status})"),
            });
        }

        let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let items = body["items"].as_array().cloned().unwrap_or_default();

        let mut calendars: Vec<CalendarListEntry> = items
            .iter()
            .filter_map(|item| {
                let id = item["id"].as_str()?.to_string();
                let summary = item["summary"].as_str().unwrap_or("").to_string();
                let background_color = item["backgroundColor"].as_str().map(str::to_string);
                let primary = item["primary"].as_bool().unwrap_or(false);
                Some(CalendarListEntry {
                    id,
                    summary,
                    background_color,
                    primary,
                })
            })
            .collect();

        calendars.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.summary.cmp(&b.summary)));

        if let Ok(serialized) = serde_json::to_string(&calendars) {
            let conn = state.db.lock().await;
            let _ = conn.execute(
                "INSERT INTO google_http_cache (key, etag, body, fetched_at, ttl_seconds)
                 VALUES (?1, NULL, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET body = excluded.body, fetched_at = excluded.fetched_at, ttl_seconds = excluded.ttl_seconds",
                params![CALENDAR_LIST_CACHE_KEY, serialized, now_iso_utc(), CALENDAR_LIST_TTL_SECS],
            );
        }

        return Ok(calendars);
    }

    Err("GOOGLE_SESSION_EXPIRED".to_string())
}

pub async fn get_target_calendar_id(state: &AppState) -> String {
    let conn = state.db.lock().await;
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'google_calendar_target_id'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .unwrap_or_else(|| "primary".to_string())
}

pub async fn verify_calendar_exists(
    state: &AppState,
    token: &str,
    calendar_id: &str,
) -> Result<(String, Option<String>), String> {
    let url = build_calendar_list_entry_url(calendar_id)?;
    let resp = state
        .google
        .client
        .get(url)
        .bearer_auth(token)
        .query(&[("fields", "id,summary,accessRole")])
        .send()
        .await
        .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

    if !resp.status().is_success() {
        return Err("GOOGLE_CALENDAR_NOT_FOUND".to_string());
    }

    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let access_role = body["accessRole"].as_str().unwrap_or("");
    if !matches!(access_role, "owner" | "writer") {
        return Err("GOOGLE_CALENDAR_NO_PERMISSION".to_string());
    }

    let name = body["summary"].as_str().map(str::to_string);
    Ok((calendar_id.to_string(), name))
}

pub fn build_calendar_description(
    channel_name: &str,
    source_name: &str,
    programme_desc: Option<&str>,
    tmdb_rating: Option<f64>,
    tmdb_year: Option<&str>,
) -> String {
    let mut parts = Vec::new();

    if !channel_name.trim().is_empty() {
        parts.push(format!("Csatorna: {}", channel_name.trim()));
    }
    if !source_name.trim().is_empty() {
        parts.push(format!("Forrás: {}", source_name.trim()));
    }

    if let Some(desc) = programme_desc.map(str::trim).filter(|d| !d.is_empty()) {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let truncated = if desc.chars().count() > 500 {
            let prefix: String = desc.chars().take(497).collect();
            format!("{prefix}…")
        } else {
            desc.to_string()
        };
        parts.push(truncated);
    }

    if tmdb_rating.is_some_and(|r| r > 0.0)
        || tmdb_year.is_some_and(|y| !y.trim().is_empty())
    {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        let mut tmdb_line = String::from("TMDB:");
        if let Some(r) = tmdb_rating.filter(|&r| r > 0.0) {
            tmdb_line.push_str(&format!(" {:.1}/10", r));
        }
        if let Some(y) = tmdb_year.map(str::trim).filter(|y| !y.is_empty()) {
            tmdb_line.push_str(&format!(" ({y})"));
        }
        parts.push(tmdb_line);
    }

    parts.join("\n")
}

pub fn get_tracked_event_in_conn(
    conn: &Connection,
    epg_key: &str,
) -> Option<TrackedCalendarEvent> {
    conn.query_row(
        "SELECT epg_key, source_id, channel_id, start_ts, stop_ts, title, calendar_id, calendar_name, event_id, html_link, created_at
         FROM google_calendar_events WHERE epg_key = ?1",
        params![epg_key],
        |r| {
            Ok(TrackedCalendarEvent {
                epg_key: r.get(0)?,
                source_id: r.get(1)?,
                channel_id: r.get(2)?,
                start_ts: r.get(3)?,
                stop_ts: r.get(4)?,
                title: r.get(5)?,
                calendar_id: r.get(6)?,
                calendar_name: r.get(7)?,
                event_id: r.get(8)?,
                html_link: r.get(9)?,
                created_at: r.get(10)?,
            })
        },
    )
    .ok()
}

pub fn upsert_tracked_event_in_conn(
    conn: &Connection,
    ev: &TrackedCalendarEvent,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO google_calendar_events
            (epg_key, source_id, channel_id, start_ts, stop_ts, title, calendar_id, calendar_name, event_id, html_link, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(epg_key) DO UPDATE SET
            source_id = excluded.source_id,
            channel_id = excluded.channel_id,
            start_ts = excluded.start_ts,
            stop_ts = excluded.stop_ts,
            title = excluded.title,
            calendar_id = excluded.calendar_id,
            calendar_name = COALESCE(excluded.calendar_name, google_calendar_events.calendar_name),
            event_id = excluded.event_id,
            html_link = COALESCE(excluded.html_link, google_calendar_events.html_link)",
        params![
            ev.epg_key,
            ev.source_id,
            ev.channel_id,
            ev.start_ts,
            ev.stop_ts,
            ev.title,
            ev.calendar_id,
            ev.calendar_name,
            ev.event_id,
            ev.html_link,
            ev.created_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_all_tracked_event_keys_in_conn(conn: &Connection) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare("SELECT epg_key FROM google_calendar_events") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

pub fn count_tracked_events_in_conn(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM google_calendar_events", [], |r| {
        r.get::<_, i64>(0)
    })
    .unwrap_or(0)
}

pub fn select_cleanup_candidates_in_conn(
    conn: &Connection,
    older_than_days: Option<i64>,
    now_ts: i64,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    if let Some(days) = older_than_days {
        let cutoff = now_ts.saturating_sub(days.max(1) * 86_400);
        if let Ok(mut stmt) = conn.prepare(
            "SELECT epg_key, calendar_id, event_id FROM google_calendar_events WHERE stop_ts < ?1 ORDER BY start_ts ASC",
        ) {
            if let Ok(rows) = stmt.query_map(params![cutoff], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
            }) {
                out.extend(rows.flatten());
            }
        }
    } else if let Ok(mut stmt) = conn.prepare(
        "SELECT epg_key, calendar_id, event_id FROM google_calendar_events ORDER BY start_ts ASC",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        }) {
            out.extend(rows.flatten());
        }
    }
    out
}

pub enum ParsedSyncedEvent {
    Upsert(TrackedCalendarEvent),
    Cancelled { event_id: String },
    Ignored,
}

pub fn parse_synced_event_item(
    calendar_id: &str,
    calendar_name: Option<&str>,
    item: &serde_json::Value,
) -> ParsedSyncedEvent {
    let Some(event_id) = item["id"].as_str().filter(|s| !s.is_empty()) else {
        return ParsedSyncedEvent::Ignored;
    };

    if item["status"].as_str() == Some("cancelled") {
        return ParsedSyncedEvent::Cancelled {
            event_id: event_id.to_string(),
        };
    }

    let priv_props = &item["extendedProperties"]["private"];
    if priv_props["epgViewer"].as_str() != Some("1") {
        return ParsedSyncedEvent::Ignored;
    }

    let Some(epg_key) = priv_props["epgKey"].as_str().filter(|s| !s.is_empty()) else {
        return ParsedSyncedEvent::Ignored;
    };

    let source_id = priv_props["sourceId"].as_str().unwrap_or("").to_string();
    let channel_id = priv_props["channelId"].as_str().unwrap_or("").to_string();
    let start_ts = priv_props["startTs"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| {
            item["start"]["dateTime"]
                .as_str()
                .and_then(iso_utc_to_epoch)
        })
        .unwrap_or(0);
    let stop_ts = priv_props["stopTs"]
        .as_str()
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| item["end"]["dateTime"].as_str().and_then(iso_utc_to_epoch))
        .unwrap_or(start_ts);
    let title = item["summary"].as_str().unwrap_or("").to_string();
    let html_link = item["htmlLink"].as_str().map(str::to_string);

    ParsedSyncedEvent::Upsert(TrackedCalendarEvent {
        epg_key: epg_key.to_string(),
        source_id,
        channel_id,
        start_ts,
        stop_ts,
        title,
        calendar_id: calendar_id.to_string(),
        calendar_name: calendar_name.map(str::to_string),
        event_id: event_id.to_string(),
        html_link,
        created_at: now_iso_utc(),
    })
}

pub async fn add_calendar_event(
    state: &AppState,
    req: CalendarEventRequest,
) -> Result<CalendarEventResult, String> {
    let clean_epg_key = req.epg_key.trim().to_string();
    if !clean_epg_key.is_empty() {
        let conn = state.db.lock().await;
        if let Some(existing) = get_tracked_event_in_conn(&conn, &clean_epg_key) {
            return Ok(CalendarEventResult {
                event_id: existing.event_id,
                html_link: existing.html_link.unwrap_or_default(),
                calendar_id: existing.calendar_id,
                calendar_name: existing.calendar_name,
                already_exists: true,
            });
        }
    }

    let mut token = ensure_valid_google_token(state).await?;

    let calendar_id = match req.calendar_id.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(id) => id.trim().to_string(),
        None => get_target_calendar_id(state).await,
    };

    let (verified_calendar_id, calendar_name) = if calendar_id != "primary" {
        match verify_calendar_exists(state, token.as_str(), &calendar_id).await {
            Ok(res) => res,
            Err(_) => {
                let conn = state.db.lock().await;
                let _ = conn.execute(
                    "INSERT INTO settings (key, value) VALUES ('google_calendar_target_id', 'primary')
                     ON CONFLICT(key) DO UPDATE SET value = 'primary'",
                    [],
                );
                ("primary".to_string(), None)
            }
        }
    } else {
        ("primary".to_string(), None)
    };

    let description = build_calendar_description(
        &req.channel_name,
        &req.source_name,
        req.programme_desc.as_deref(),
        req.tmdb_rating,
        req.tmdb_year.as_deref(),
    );

    let app_version = env!("CARGO_PKG_VERSION");
    let source_title = format!("{APP_DISPLAY_NAME} v{app_version}");

    let event_body = serde_json::json!({
        "summary": req.summary,
        "description": description,
        "start": {
            "dateTime": req.start_datetime,
            "timeZone": req.timezone,
        },
        "end": {
            "dateTime": req.end_datetime,
            "timeZone": req.timezone,
        },
        "source": {
            "title": source_title,
            "url": APP_REPO_URL,
        },
        "extendedProperties": {
            "private": {
                "epgViewer": "1",
                "epgKey": clean_epg_key,
                "sourceId": req.source_id,
                "channelId": req.channel_id,
                "startTs": req.start_ts.to_string(),
                "stopTs": req.stop_ts.to_string(),
                "createdBy": APP_DISPLAY_NAME,
                "appVersion": app_version,
            }
        },
        "reminders": {
            "useDefault": false,
            "overrides": [
                { "method": "popup", "minutes": req.reminder_minutes.clamp(0, 40320) }
            ]
        }
    });

    let url = build_calendar_events_url(&verified_calendar_id)?;

    for attempt in 0..2 {
        let resp = state
            .google
            .client
            .post(url.clone())
            .bearer_auth(token.as_str())
            .json(&event_body)
            .send()
            .await
            .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            token = force_refresh_google_token(state, token.as_str()).await?;
            continue;
        }
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(match status {
                401 => "GOOGLE_SESSION_EXPIRED".to_string(),
                403 => "GOOGLE_CALENDAR_NO_PERMISSION".to_string(),
                404 => "GOOGLE_CALENDAR_NOT_FOUND".to_string(),
                _ => format!("Naptár esemény létrehozás sikertelen ({status}): {body}"),
            });
        }

        let result: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        let event_id = result["id"].as_str().unwrap_or("").to_string();
        let html_link = result["htmlLink"].as_str().unwrap_or("").to_string();

        if !clean_epg_key.is_empty() && !event_id.is_empty() {
            let tracked = TrackedCalendarEvent {
                epg_key: clean_epg_key,
                source_id: req.source_id,
                channel_id: req.channel_id,
                start_ts: req.start_ts,
                stop_ts: req.stop_ts,
                title: req.summary,
                calendar_id: verified_calendar_id.clone(),
                calendar_name: calendar_name.clone(),
                event_id: event_id.clone(),
                html_link: if html_link.is_empty() {
                    None
                } else {
                    Some(html_link.clone())
                },
                created_at: now_iso_utc(),
            };
            let conn = state.db.lock().await;
            let _ = upsert_tracked_event_in_conn(&conn, &tracked);
        }

        return Ok(CalendarEventResult {
            event_id,
            html_link,
            calendar_id: verified_calendar_id,
            calendar_name,
            already_exists: false,
        });
    }

    Err("GOOGLE_SESSION_EXPIRED".to_string())
}

async fn delete_remote_event(
    state: &AppState,
    token: &mut zeroize::Zeroizing<String>,
    calendar_id: &str,
    event_id: &str,
) -> Result<(), String> {
    let url = build_calendar_event_url(calendar_id, event_id)?;

    for attempt in 0..2 {
        let resp = state
            .google
            .client
            .delete(url.clone())
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

        let status = resp.status().as_u16();
        if status == 401 && attempt == 0 {
            *token = force_refresh_google_token(state, token.as_str()).await?;
            continue;
        }
        // 200/204 = deleted; 404/410 = already deleted on Google Calendar -> treat as success
        if resp.status().is_success() || status == 404 || status == 410 {
            return Ok(());
        }
        return Err(match status {
            401 => "GOOGLE_SESSION_EXPIRED".to_string(),
            403 => "GOOGLE_CALENDAR_NO_PERMISSION".to_string(),
            _ => format!("Naptár esemény törlése sikertelen ({status})"),
        });
    }

    Err("GOOGLE_SESSION_EXPIRED".to_string())
}

pub async fn remove_calendar_event(state: &AppState, epg_key: &str) -> Result<bool, String> {
    let clean_key = epg_key.trim();
    if clean_key.is_empty() {
        return Ok(false);
    }

    let tracked = {
        let conn = state.db.lock().await;
        get_tracked_event_in_conn(&conn, clean_key)
    };
    let Some(ev) = tracked else {
        return Ok(false);
    };

    let mut token = ensure_valid_google_token(state).await?;
    delete_remote_event(state, &mut token, &ev.calendar_id, &ev.event_id).await?;

    let conn = state.db.lock().await;
    conn.execute(
        "DELETE FROM google_calendar_events WHERE epg_key = ?1",
        params![clean_key],
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

fn sync_token_key(calendar_id: &str) -> String {
    format!("calendar_sync_token:{calendar_id}")
}

async fn sync_single_calendar(
    state: &AppState,
    token: &mut zeroize::Zeroizing<String>,
    calendar_id: &str,
    force_full: bool,
) -> Result<(usize, usize), String> {
    let key = sync_token_key(calendar_id);
    let mut stored_sync_token: Option<String> = if force_full {
        None
    } else {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT value FROM google_sync_state WHERE key = ?1",
            params![key],
            |r| r.get(0),
        )
        .ok()
        .filter(|s: &String| !s.trim().is_empty())
    };

    let base_url = build_calendar_events_url(calendar_id)?;
    let mut added_or_updated = 0usize;
    let mut removed = 0usize;

    // Up to 2 passes: if incremental syncToken is rejected (410/400), retry once as full sync.
    for pass in 0..2 {
        let is_incremental = stored_sync_token.is_some();
        let mut page_token: Option<String> = None;
        let mut new_sync_token: Option<String> = None;
        let mut live_event_ids: HashSet<String> = HashSet::new();
        let mut need_full_retry = false;

        loop {
            let mut query_params: Vec<(&str, String)> = vec![
                ("privateExtendedProperty", "epgViewer=1".to_string()),
                ("maxResults", "250".to_string()),
                (
                    "fields",
                    "nextPageToken,nextSyncToken,items(id,status,summary,htmlLink,start,end,extendedProperties)"
                        .to_string(),
                ),
            ];
            if let Some(ref st) = stored_sync_token {
                query_params.push(("syncToken", st.clone()));
            } else {
                query_params.push(("showDeleted", "false".to_string()));
            }
            if let Some(ref pt) = page_token {
                query_params.push(("pageToken", pt.clone()));
            }

            let mut resp_opt = None;
            for attempt in 0..2 {
                let resp = state
                    .google
                    .client
                    .get(base_url.clone())
                    .bearer_auth(token.as_str())
                    .query(&query_params)
                    .send()
                    .await
                    .map_err(|e| format!("GOOGLE_OFFLINE: {e}"))?;

                let status = resp.status().as_u16();
                if status == 401 && attempt == 0 {
                    *token = force_refresh_google_token(state, token.as_str()).await?;
                    continue;
                }
                resp_opt = Some(resp);
                break;
            }

            let resp = resp_opt.ok_or_else(|| "GOOGLE_SESSION_EXPIRED".to_string())?;
            let status = resp.status().as_u16();
            if (status == 410 || status == 400) && is_incremental && pass == 0 {
                // Expired or invalid syncToken -> drop token and redo clean full sync
                let conn = state.db.lock().await;
                let _ = conn.execute("DELETE FROM google_sync_state WHERE key = ?1", params![key]);
                stored_sync_token = None;
                need_full_retry = true;
                break;
            }
            if status == 404 {
                // Calendar deleted on Google -> remove local events for this calendar
                let conn = state.db.lock().await;
                let deleted = conn
                    .execute(
                        "DELETE FROM google_calendar_events WHERE calendar_id = ?1",
                        params![calendar_id],
                    )
                    .unwrap_or(0);
                let _ = conn.execute("DELETE FROM google_sync_state WHERE key = ?1", params![key]);
                return Ok((0, deleted));
            }
            if !resp.status().is_success() {
                return Err(match status {
                    401 => "GOOGLE_SESSION_EXPIRED".to_string(),
                    403 => "GOOGLE_CALENDAR_NO_PERMISSION".to_string(),
                    _ => format!("Naptár szinkronizálás sikertelen ({status})"),
                });
            }

            let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
            let items = body["items"].as_array().cloned().unwrap_or_default();

            {
                let conn = state.db.lock().await;
                for item in &items {
                    match parse_synced_event_item(calendar_id, None, item) {
                        ParsedSyncedEvent::Upsert(ev) => {
                            live_event_ids.insert(ev.event_id.clone());
                            if upsert_tracked_event_in_conn(&conn, &ev).is_ok() {
                                added_or_updated += 1;
                            }
                        }
                        ParsedSyncedEvent::Cancelled { event_id } => {
                            let del = conn
                                .execute(
                                    "DELETE FROM google_calendar_events WHERE calendar_id = ?1 AND event_id = ?2",
                                    params![calendar_id, event_id],
                                )
                                .unwrap_or(0);
                            removed += del;
                        }
                        ParsedSyncedEvent::Ignored => {}
                    }
                }
            }

            if let Some(nst) = body["nextSyncToken"].as_str().filter(|s| !s.is_empty()) {
                new_sync_token = Some(nst.to_string());
            }
            if let Some(npt) = body["nextPageToken"].as_str().filter(|s| !s.is_empty()) {
                page_token = Some(npt.to_string());
            } else {
                break;
            }
        }

        if need_full_retry {
            continue;
        }

        // On full sync, reconcile local rows for this calendar that no longer exist on Google
        if !is_incremental {
            let conn = state.db.lock().await;
            let local_ids: Vec<(String, String)> = {
                let mut stmt = conn
                    .prepare(
                        "SELECT epg_key, event_id FROM google_calendar_events WHERE calendar_id = ?1",
                    )
                    .map_err(|e| e.to_string())?;
                let mapped = stmt
                    .query_map(params![calendar_id], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(|e| e.to_string())?;
                mapped.flatten().collect()
            };
            for (epg_key, event_id) in local_ids {
                if !live_event_ids.contains(&event_id) {
                    let del = conn
                        .execute(
                            "DELETE FROM google_calendar_events WHERE epg_key = ?1",
                            params![epg_key],
                        )
                        .unwrap_or(0);
                    removed += del;
                }
            }
        }

        if let Some(nst) = new_sync_token {
            let conn = state.db.lock().await;
            let _ = conn.execute(
                "INSERT INTO google_sync_state (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, nst],
            );
        }

        return Ok((added_or_updated, removed));
    }

    Ok((added_or_updated, removed))
}

pub async fn sync_calendar_events(
    state: &AppState,
    force_full: bool,
) -> Result<CalendarSyncResult, String> {
    let mut token = ensure_valid_google_token(state).await?;

    let mut calendar_ids: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let target_cal = get_target_calendar_id(state).await;
    if seen.insert(target_cal.clone()) {
        calendar_ids.push(target_cal);
    }

    {
        let conn = state.db.lock().await;
        if let Ok(mut stmt) =
            conn.prepare("SELECT DISTINCT calendar_id FROM google_calendar_events")
        {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for cid in rows.flatten() {
                    if !cid.trim().is_empty() && seen.insert(cid.clone()) {
                        calendar_ids.push(cid);
                    }
                }
            }
        };
    }

    let mut total_upserted = 0usize;
    let mut total_removed = 0usize;
    let mut synced_calendars = 0usize;

    for cal_id in &calendar_ids {
        match sync_single_calendar(state, &mut token, cal_id, force_full).await {
            Ok((u, r)) => {
                total_upserted += u;
                total_removed += r;
                synced_calendars += 1;
            }
            Err(e) if e == "GOOGLE_SESSION_EXPIRED" || e.starts_with("GOOGLE_OFFLINE") => {
                return Err(e);
            }
            Err(_) => {
                // Skip individual inaccessible calendar so other calendars still sync
            }
        }
    }

    let total_tracked = {
        let conn = state.db.lock().await;
        count_tracked_events_in_conn(&conn)
    };

    Ok(CalendarSyncResult {
        synced_calendars,
        added_or_updated: total_upserted,
        removed: total_removed,
        total_tracked,
    })
}

pub async fn cleanup_calendar_events(
    state: &AppState,
    older_than_days: Option<i64>,
) -> Result<CalendarCleanupResult, String> {
    // Sync first so events created by EPG Viewer on other devices are included,
    // and only events verified to have `extendedProperties.private.epgViewer == "1"` exist in `google_calendar_events`.
    let _ = sync_calendar_events(state, true).await?;

    let mut token = ensure_valid_google_token(state).await?;
    let candidates = {
        let conn = state.db.lock().await;
        select_cleanup_candidates_in_conn(&conn, older_than_days, now_epoch_secs())
    };

    let mut deleted_count = 0usize;
    let mut failed_count = 0usize;

    for (epg_key, calendar_id, event_id) in candidates {
        match delete_remote_event(state, &mut token, &calendar_id, &event_id).await {
            Ok(()) => {
                let conn = state.db.lock().await;
                let _ = conn.execute(
                    "DELETE FROM google_calendar_events WHERE epg_key = ?1",
                    params![epg_key],
                );
                deleted_count += 1;
            }
            Err(e) if e == "GOOGLE_SESSION_EXPIRED" => return Err(e),
            Err(_) => {
                failed_count += 1;
            }
        }
    }

    let remaining_count = {
        let conn = state.db.lock().await;
        count_tracked_events_in_conn(&conn)
    };

    Ok(CalendarCleanupResult {
        deleted_count,
        failed_count,
        remaining_count,
    })
}
