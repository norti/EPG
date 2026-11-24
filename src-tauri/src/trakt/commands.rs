use crate::trakt::auth::{
    cancel_device_flow, get_auth_status, logout, save_custom_client_secret, start_device_flow,
    DeviceCodeResponse, TraktAuthStatus,
};
use crate::trakt::calendar::{
    fetch_and_cache_global_calendar, fetch_and_cache_my_calendar, query_calendar_episodes,
    query_global_calendar_merged, CalendarEpisodeItem,
};
use crate::trakt::client::{trakt_get_cached, trakt_post};
use crate::trakt::matcher::{match_titles, rebuild_title_index};
use crate::trakt::secret::default_secret_store;
use crate::trakt::sync::{
    enrich_missing_history_posters, get_sync_state_val, is_sync_overdue, local_yesterday_ymd,
    run_sync, sync_show_progress, SyncSummary,
};
use crate::trakt::{is_trakt_enabled, iso_utc_to_epoch, now_epoch_millis, now_epoch_secs, now_iso_utc};
use crate::{
    apply_tmdb_auth, cache_logo_inner, clean_title, extract_year_hint,
    get_tmdb_api_key_from_state, get_tmdb_by_id, normalise_for_match, tmdb_title_score, AppState,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use tauri::Emitter;

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktSyncStatus {
    pub last_sync_ok_at: Option<String>,
    pub overdue: bool,
    pub pending_count: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktHistoryItem {
    pub history_id: i64,
    pub trakt_id: i64,
    pub media_type: String,
    pub show_trakt_id: Option<i64>,
    pub show_tmdb_id: Option<i64>,
    pub show_title: String,
    pub episode_title: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub watched_at: String,
    pub poster_path: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ContinueWatchingItem {
    pub episode_trakt_id: i64,
    pub show_trakt_id: i64,
    pub show_tmdb_id: Option<i64>,
    pub show_title: String,
    pub show_title_hu: Option<String>,
    pub season: i64,
    pub number: i64,
    pub episode_title: Option<String>,
    pub first_aired: Option<String>,
    pub network: Option<String>,
    pub poster_path: Option<String>,
    pub overview: Option<String>,
    pub aired: i64,
    pub completed: i64,
    pub last_watched_at: Option<String>,
    pub on_watchlist: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktDashboardData {
    pub calendar_episodes: Vec<CalendarEpisodeItem>,
    pub continue_watching: Vec<ContinueWatchingItem>,
    pub history: Vec<TraktHistoryItem>,
    pub last_sync_ok_at: Option<String>,
    pub overdue: bool,
    pub pending_count: i64,
    pub offline: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktWatchlistItem {
    pub trakt_id: i64,
    pub media_type: String,
    pub title: String,
    pub title_hu: Option<String>,
    pub aliases: Vec<String>,
    pub year: Option<i64>,
    pub tmdb_id: Option<i64>,
    pub poster_path: Option<String>,
    pub listed_at: String,
    pub rating: Option<f64>,
    pub votes: Option<i64>,
    pub status: Option<String>,
    pub network: Option<String>,
    pub overview: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktModalData {
    pub trakt_id: i64,
    pub tmdb_id: Option<i64>,
    pub media_type: String,
    pub title: String,
    pub year: Option<i64>,
    pub rating: Option<f64>,
    pub votes: Option<i64>,
    pub status: Option<String>,
    pub network: Option<String>,
    pub overview: Option<String>,
    pub on_watchlist: bool,
    pub logged_in: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktSeasonEpisodeItem {
    pub season: i64,
    pub episode: i64,
    pub episode_trakt_id: Option<i64>,
    pub title: String,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
    pub watched: bool,
    pub watched_at: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktSeasonGroup {
    pub season_number: i64,
    pub name: String,
    pub aired: i64,
    pub completed: i64,
    pub episodes: Vec<TraktSeasonEpisodeItem>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktShowSeasonsProgress {
    pub show_trakt_id: i64,
    pub tmdb_id: Option<i64>,
    pub aired: i64,
    pub completed: i64,
    pub last_watched_at: Option<String>,
    pub seasons: Vec<TraktSeasonGroup>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EpgAiring {
    pub channel_id: String,
    pub channel_name: String,
    pub source_id: String,
    pub source_name: String,
    pub start: String,
    pub stop: String,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub time_offset: i32,
    pub title: String,
    pub desc: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TraktStatsResponse {
    pub watchlist_count: i64,
    pub episodes_watched: i64,
    pub movies_watched: i64,
    pub total_minutes: i64,
    pub cache_items_count: i64,
    pub cache_calendar_count: i64,
    pub cache_history_count: i64,
    pub cache_episodes_count: i64,
    pub pending_actions_count: i64,
}

#[derive(Deserialize, Debug)]
struct TraktSearchResult {
    #[serde(rename = "type")]
    item_type: String,
    movie: Option<TraktSearchMedia>,
    show: Option<TraktSearchMedia>,
}

#[derive(Deserialize, Debug)]
struct TraktSearchMedia {
    title: String,
    year: Option<i64>,
    overview: Option<String>,
    rating: Option<f64>,
    votes: Option<i64>,
    status: Option<String>,
    network: Option<String>,
    ids: TraktSearchIds,
}

#[derive(Deserialize, Debug)]
struct TraktSearchIds {
    trakt: i64,
    tmdb: Option<i64>,
    imdb: Option<String>,
    slug: Option<String>,
}

#[tauri::command]
pub async fn trakt_get_auth_status(
    state: tauri::State<'_, AppState>,
) -> Result<TraktAuthStatus, String> {
    get_auth_status(state.inner()).await
}

#[tauri::command]
pub async fn trakt_start_login(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<DeviceCodeResponse, String> {
    start_device_flow(state.inner(), Some(app)).await
}

#[tauri::command]
pub async fn trakt_cancel_login(state: tauri::State<'_, AppState>) -> Result<(), String> {
    cancel_device_flow(state.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn trakt_logout(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    logout(state.inner(), Some(&app)).await
}

#[tauri::command]
pub async fn trakt_set_enabled(
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('trakt_enabled', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![if enabled { "true" } else { "false" }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn trakt_set_custom_keys(
    state: tauri::State<'_, AppState>,
    client_id: String,
    client_secret: Option<String>,
) -> Result<(), String> {
    let store = default_secret_store();
    let conn = state.db.lock().await;
    let clean_id = client_id.trim();
    if clean_id.is_empty() {
        conn.execute(
            "DELETE FROM settings WHERE key = 'trakt_client_id_override'",
            [],
        )
        .map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('trakt_client_id_override', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![clean_id],
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(sec) = client_secret {
        save_custom_client_secret(&conn, store.as_ref(), Some(&sec))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn trakt_sync(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<SyncSummary, String> {
    run_sync(state.inner(), Some(&app), force.unwrap_or(true)).await
}

#[tauri::command]
pub async fn trakt_get_sync_status(
    state: tauri::State<'_, AppState>,
) -> Result<TraktSyncStatus, String> {
    let conn = state.db.lock().await;
    let last_sync_ok_at = get_sync_state_val(&conn, "last_sync_ok_at");
    let overdue = is_sync_overdue(last_sync_ok_at.as_deref());
    let pending_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_pending_actions", [], |r| r.get(0))
        .unwrap_or(0);
    Ok(TraktSyncStatus {
        last_sync_ok_at,
        overdue,
        pending_count,
    })
}

pub fn query_continue_watching(
    conn: &rusqlite::Connection,
    now_secs: i64,
) -> Result<Vec<ContinueWatchingItem>, String> {
    let mut cw_stmt = conn
        .prepare(
            "SELECT p.next_episode_trakt_id,
                    p.show_trakt_id,
                    COALESCE(w.tmdb_id, tc.tmdb_id) AS tmdb_id,
                    COALESCE(tc.title, w.title, '') AS show_title,
                    COALESCE(w.title_hu, (SELECT tm.hu_title FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(w.tmdb_id, tc.tmdb_id) AND tm.media_type = 'tv' AND tm.hu_title IS NOT NULL LIMIT 1)) AS show_title_hu,
                    p.next_season,
                    p.next_episode,
                    p.next_episode_title,
                    COALESCE(p.next_episode_first_aired, (SELECT c.first_aired FROM trakt_calendar_cache c WHERE c.episode_trakt_id = p.next_episode_trakt_id LIMIT 1)) AS first_aired,
                    tc.network,
                    COALESCE(w.poster_path, (SELECT tm.poster_path FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(w.tmdb_id, tc.tmdb_id) AND tm.media_type = 'tv' AND tm.poster_path IS NOT NULL LIMIT 1)) AS poster_path,
                    COALESCE(tc.overview, (SELECT tm.overview FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(w.tmdb_id, tc.tmdb_id) AND tm.media_type = 'tv' AND tm.overview IS NOT NULL LIMIT 1)) AS overview,
                    COALESCE(p.aired, 0),
                    COALESCE(p.completed, 0),
                    p.last_watched_at,
                    CASE WHEN w.trakt_id IS NOT NULL THEN 1 ELSE 0 END AS on_watchlist
             FROM trakt_show_progress p
             LEFT JOIN trakt_cache tc ON tc.media_type = 'show' AND tc.trakt_id = p.show_trakt_id
             LEFT JOIN trakt_watchlist_cache w ON w.trakt_id = p.show_trakt_id
             LEFT JOIN trakt_episodes e ON e.show_trakt_id = p.show_trakt_id AND e.season = p.next_season AND e.episode = p.next_episode
             WHERE p.hidden_from_progress = 0
               AND COALESCE(p.completed, 0) > 0
               AND p.next_episode_trakt_id IS NOT NULL
               AND p.next_season IS NOT NULL
               AND p.next_episode IS NOT NULL
               AND e.watched_at IS NULL
             ORDER BY p.last_watched_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let items: Vec<ContinueWatchingItem> = cw_stmt
        .query_map([], |r| {
            let wl_int: i32 = r.get(15)?;
            Ok(ContinueWatchingItem {
                episode_trakt_id: r.get(0)?,
                show_trakt_id: r.get(1)?,
                show_tmdb_id: r.get(2)?,
                show_title: r.get(3)?,
                show_title_hu: r.get(4)?,
                season: r.get(5)?,
                number: r.get(6)?,
                episode_title: r.get(7)?,
                first_aired: r.get(8)?,
                network: r.get(9)?,
                poster_path: r.get(10)?,
                overview: r.get(11)?,
                aired: r.get(12)?,
                completed: r.get(13)?,
                last_watched_at: r.get(14)?,
                on_watchlist: wl_int == 1,
            })
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|item| {
            item.first_aired
                .as_deref()
                .and_then(iso_utc_to_epoch)
                .map_or(false, |aired_ts| aired_ts <= now_secs)
        })
        .collect();

    Ok(items)
}

#[tauri::command]
pub async fn trakt_get_dashboard_data(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    start_date: Option<String>,
    force_refresh: Option<bool>,
) -> Result<TraktDashboardData, String> {
    let start = start_date
        .filter(|s| s.trim().len() >= 10)
        .unwrap_or_else(local_yesterday_ymd);

    let mut offline = false;
    if let Err(e) =
        fetch_and_cache_my_calendar(state.inner(), Some(&app), &start, force_refresh.unwrap_or(false))
            .await
    {
        if e.contains("TRAKT_OFFLINE") || e.contains("TRAKT_ERR_429") {
            offline = true;
        } else if e.contains("TRAKT_NOT_LOGGED_IN") || e.contains("TRAKT_DISABLED") {
            return Err(e);
        }
    }

    enrich_missing_history_posters(state.inner()).await;

    let conn = state.db.lock().await;
    let calendar_episodes = query_calendar_episodes(&conn, "my", false)?;
    let continue_watching = query_continue_watching(&conn, now_epoch_secs())?;

    let mut hist_stmt = conn
        .prepare(
            "SELECT h.history_id, h.trakt_id, h.media_type, h.show_trakt_id, h.show_tmdb_id,
                    COALESCE(h.show_title, ''), h.episode_title, h.season, h.episode, h.watched_at,
                    COALESCE(h.poster_path, w.poster_path, (SELECT tm.poster_path FROM tmdb_cache tm WHERE tm.tmdb_id = h.show_tmdb_id AND tm.poster_path IS NOT NULL LIMIT 1))
             FROM trakt_history_cache h
             LEFT JOIN trakt_watchlist_cache w ON w.trakt_id = COALESCE(h.show_trakt_id, h.trakt_id)
             ORDER BY h.watched_at DESC
             LIMIT 30",
        )
        .map_err(|e| e.to_string())?;

    let history: Vec<TraktHistoryItem> = hist_stmt
        .query_map([], |r| {
            Ok(TraktHistoryItem {
                history_id: r.get(0)?,
                trakt_id: r.get(1)?,
                media_type: r.get(2)?,
                show_trakt_id: r.get(3)?,
                show_tmdb_id: r.get(4)?,
                show_title: r.get(5)?,
                episode_title: r.get(6)?,
                season: r.get(7)?,
                episode: r.get(8)?,
                watched_at: r.get(9)?,
                poster_path: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let last_sync_ok_at = get_sync_state_val(&conn, "last_sync_ok_at");
    let overdue = is_sync_overdue(last_sync_ok_at.as_deref());
    let pending_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_pending_actions", [], |r| r.get(0))
        .unwrap_or(0);

    Ok(TraktDashboardData {
        calendar_episodes,
        continue_watching,
        history,
        last_sync_ok_at,
        overdue,
        pending_count,
        offline,
    })
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MarkWatchedResult {
    pub queued: bool,
    pub watched_at: String,
}

#[tauri::command]
pub async fn trakt_mark_watched(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    episode_trakt_id: Option<i64>,
    movie_trakt_id: Option<i64>,
    show_trakt_id: Option<i64>,
    show_tmdb_id: Option<i64>,
    show_title: Option<String>,
    episode_title: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    poster_path: Option<String>,
    watched_at: Option<String>,
) -> Result<MarkWatchedResult, String> {
    let ts = watched_at.unwrap_or_else(now_iso_utc);

    let (payload, media_type, trakt_id) = if let Some(ep_id) = episode_trakt_id {
        (
            json!({
                "episodes": [{
                    "watched_at": ts,
                    "ids": { "trakt": ep_id }
                }]
            }),
            "episode",
            ep_id,
        )
    } else if let Some(mov_id) = movie_trakt_id {
        (
            json!({
                "movies": [{
                    "watched_at": ts,
                    "ids": { "trakt": mov_id }
                }]
            }),
            "movie",
            mov_id,
        )
    } else {
        return Err("Hiányzó Trakt azonosító".to_string());
    };

    let mut queued = false;
    match trakt_post(state.inner(), Some(&app), "/sync/history", &payload).await {
        Ok(_) => {}
        Err(err) if err.is_transient() => {
            queued = true;
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO trakt_pending_actions (action, payload_json, created_at, attempts, last_error)
                 VALUES ('history_add', ?1, ?2, 0, ?3)",
                params![payload.to_string(), now_iso_utc(), err.to_user_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        Err(err) => return Err(err.to_user_string()),
    }

    // Write-through to local trakt_episodes and trakt_history_cache
    {
        let mut conn = state.db.lock().await;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        if media_type == "episode" {
            if let (Some(sid), Some(s_num), Some(e_num)) = (show_trakt_id, season, episode) {
                tx.execute(
                    "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, title, watched_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(show_trakt_id, season, episode) DO UPDATE SET
                        episode_trakt_id = COALESCE(excluded.episode_trakt_id, trakt_episodes.episode_trakt_id),
                        watched_at = excluded.watched_at,
                        updated_at = excluded.updated_at",
                    params![sid, s_num, e_num, trakt_id, episode_title, ts, now_iso_utc()],
                )
                .ok();
            }
        }

        let temp_hist_id = -now_epoch_millis();
        tx.execute(
            "INSERT OR REPLACE INTO trakt_history_cache (
                history_id, trakt_id, media_type, show_trakt_id, show_tmdb_id,
                show_title, episode_title, season, episode, watched_at, poster_path
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                temp_hist_id,
                trakt_id,
                media_type,
                show_trakt_id,
                show_tmdb_id,
                show_title,
                episode_title,
                season,
                episode,
                ts,
                poster_path,
            ],
        )
        .ok();

        tx.execute(
            "INSERT INTO trakt_sync_state (key, value) VALUES ('history_dirty', '1')
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            [],
        )
        .ok();

        if let Some(sid) = show_trakt_id {
            let prefix = format!("/shows/{}/progress/watched%", sid);
            tx.execute(
                "DELETE FROM trakt_http_cache WHERE key LIKE ?1",
                params![prefix],
            )
            .ok();
            if queued {
                if let (Some(s_num), Some(e_num)) = (season, episode) {
                    tx.execute(
                        "UPDATE trakt_show_progress
                         SET next_episode_trakt_id = NULL, next_season = NULL, next_episode = NULL, next_episode_title = NULL, next_episode_first_aired = NULL
                         WHERE show_trakt_id = ?1 AND next_season = ?2 AND next_episode = ?3",
                        params![sid, s_num, e_num],
                    )
                    .ok();
                }
            }
        }

        tx.execute(
            "DELETE FROM trakt_http_cache WHERE key LIKE '/users/%/stats'",
            [],
        )
        .ok();

        tx.commit().map_err(|e| e.to_string())?;
    }

    if !queued && media_type == "episode" {
        if let Some(sid) = show_trakt_id {
            let _ = sync_show_progress(state.inner(), Some(&app), sid, true).await;
        }
    }

    Ok(MarkWatchedResult {
        queued,
        watched_at: ts,
    })
}

#[tauri::command]
pub async fn trakt_watchlist_add(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    trakt_id: i64,
    media_type: String,
    title: String,
    year: Option<i64>,
    tmdb_id: Option<i64>,
    poster_path: Option<String>,
    title_hu: Option<String>,
) -> Result<bool, String> {
    let norm_type = if media_type == "tv" || media_type == "show" {
        "show"
    } else {
        "movie"
    };
    let payload = if norm_type == "show" {
        json!({ "shows": [{ "ids": { "trakt": trakt_id } }] })
    } else {
        json!({ "movies": [{ "ids": { "trakt": trakt_id } }] })
    };

    let mut queued = false;
    match trakt_post(state.inner(), Some(&app), "/sync/watchlist", &payload).await {
        Ok(_) => {}
        Err(err) if err.is_transient() => {
            queued = true;
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO trakt_pending_actions (action, payload_json, created_at, attempts, last_error)
                 VALUES ('watchlist_add', ?1, ?2, 0, ?3)",
                params![payload.to_string(), now_iso_utc(), err.to_user_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        Err(err) => return Err(err.to_user_string()),
    }

    let now = now_iso_utc();
    {
        let conn = state.db.lock().await;
        let (cached_hu, cached_poster): (Option<String>, Option<String>) = tmdb_id
            .and_then(|tid| {
                let tmdb_mt = if norm_type == "show" { "tv" } else { "movie" };
                conn.query_row(
                    "SELECT hu_title, poster_path FROM tmdb_cache WHERE tmdb_id = ?1 AND media_type = ?2 AND media_type != 'none' LIMIT 1",
                    params![tid, tmdb_mt],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok()
            })
            .unwrap_or((None, None));

        conn.execute(
            "INSERT INTO trakt_watchlist_cache (
                trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id, poster_path, listed_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?8)
            ON CONFLICT(trakt_id) DO UPDATE SET
                title = excluded.title,
                title_hu = COALESCE(excluded.title_hu, trakt_watchlist_cache.title_hu),
                year = COALESCE(excluded.year, trakt_watchlist_cache.year),
                tmdb_id = COALESCE(excluded.tmdb_id, trakt_watchlist_cache.tmdb_id),
                poster_path = COALESCE(excluded.poster_path, trakt_watchlist_cache.poster_path),
                updated_at = excluded.updated_at",
            params![
                trakt_id,
                norm_type,
                title,
                title_hu.or(cached_hu),
                year,
                tmdb_id,
                poster_path.or(cached_poster),
                now,
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    let new_ver = rebuild_title_index(state.inner()).await;
    let _ = app.emit("trakt-watchlist-changed", new_ver);
    Ok(queued)
}

#[tauri::command]
pub async fn trakt_watchlist_remove(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    trakt_id: i64,
    media_type: String,
) -> Result<bool, String> {
    let norm_type = if media_type == "tv" || media_type == "show" {
        "show"
    } else {
        "movie"
    };
    let payload = if norm_type == "show" {
        json!({ "shows": [{ "ids": { "trakt": trakt_id } }] })
    } else {
        json!({ "movies": [{ "ids": { "trakt": trakt_id } }] })
    };

    let mut queued = false;
    match trakt_post(state.inner(), Some(&app), "/sync/watchlist/remove", &payload).await {
        Ok(_) => {}
        Err(err) if err.is_transient() => {
            queued = true;
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO trakt_pending_actions (action, payload_json, created_at, attempts, last_error)
                 VALUES ('watchlist_remove', ?1, ?2, 0, ?3)",
                params![payload.to_string(), now_iso_utc(), err.to_user_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        Err(err) => return Err(err.to_user_string()),
    }

    {
        let conn = state.db.lock().await;
        conn.execute(
            "DELETE FROM trakt_watchlist_cache WHERE trakt_id = ?1",
            params![trakt_id],
        )
        .map_err(|e| e.to_string())?;
    }

    let new_ver = rebuild_title_index(state.inner()).await;
    let _ = app.emit("trakt-watchlist-changed", new_ver);
    Ok(queued)
}

#[tauri::command]
pub async fn trakt_get_watchlist(
    state: tauri::State<'_, AppState>,
    filter: Option<String>,
) -> Result<Vec<TraktWatchlistItem>, String> {
    let conn = state.db.lock().await;
    let filter_clause = match filter.as_deref() {
        Some("shows") | Some("show") => "WHERE w.media_type = 'show'",
        Some("movies") | Some("movie") => "WHERE w.media_type = 'movie'",
        _ => "",
    };

    let sql = format!(
        "SELECT w.trakt_id, w.media_type, w.title, w.title_hu, w.aliases_json, w.year, w.tmdb_id,
                COALESCE(w.poster_path, (SELECT tm.poster_path FROM tmdb_cache tm WHERE tm.tmdb_id = w.tmdb_id AND tm.poster_path IS NOT NULL LIMIT 1)),
                w.listed_at, tc.rating, tc.votes, tc.status, tc.network, tc.overview
         FROM trakt_watchlist_cache w
         LEFT JOIN trakt_cache tc ON tc.media_type = w.media_type AND tc.trakt_id = w.trakt_id
         {}
         ORDER BY w.listed_at DESC",
        filter_clause
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let aliases_str: Option<String> = r.get(4)?;
            let aliases: Vec<String> = aliases_str
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();
            Ok(TraktWatchlistItem {
                trakt_id: r.get(0)?,
                media_type: r.get(1)?,
                title: r.get(2)?,
                title_hu: r.get(3)?,
                aliases,
                year: r.get(5)?,
                tmdb_id: r.get(6)?,
                poster_path: r.get(7)?,
                listed_at: r.get(8)?,
                rating: r.get(9)?,
                votes: r.get(10)?,
                status: r.get(11)?,
                network: r.get(12)?,
                overview: r.get(13)?,
            })
        })
        .map_err(|e| e.to_string())?;

    Ok(rows.flatten().collect())
}

#[derive(Deserialize, Debug)]
struct TraktSeasonApi {
    number: i64,
    title: Option<String>,
    #[serde(default)]
    episodes: Vec<TraktSeasonEpisodeApi>,
}

#[derive(Deserialize, Debug)]
struct TraktSeasonEpisodeApi {
    season: i64,
    number: i64,
    title: Option<String>,
    overview: Option<String>,
    first_aired: Option<String>,
    ids: TraktSearchIds,
}

#[derive(Deserialize, Debug)]
struct TmdbSeasonApi {
    name: Option<String>,
    #[serde(default)]
    episodes: Vec<TmdbSeasonEpisodeApi>,
}

#[derive(Deserialize, Debug)]
struct TmdbSeasonEpisodeApi {
    episode_number: i64,
    name: Option<String>,
    overview: Option<String>,
    still_path: Option<String>,
    air_date: Option<String>,
}

#[tauri::command]
pub async fn trakt_get_show_seasons_progress(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    show_trakt_id: i64,
    tmdb_id: Option<i64>,
    lang: Option<String>,
) -> Result<TraktShowSeasonsProgress, String> {
    let tmdb_lang = lang
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "hu-HU".to_string());

    // 1. Check if we have any episode data in DB for this show
    let has_local_episodes: bool = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM trakt_episodes WHERE show_trakt_id = ?1)",
            params![show_trakt_id],
            |r| r.get(0),
        )
        .unwrap_or(false)
    };

    // 2. Fetch watched progress (10-min TTL if data exists, force-refresh if first time)
    let progress_opt = sync_show_progress(state.inner(), Some(&app), show_trakt_id, !has_local_episodes)
        .await
        .ok();

    // 3. Fetch Trakt seasons + episodes (24h TTL) for episode Trakt IDs and fallback titles
    let seasons_path = format!("/shows/{}/seasons?extended=episodes", show_trakt_id);
    let trakt_seasons: Vec<TraktSeasonApi> =
        match trakt_get_cached(state.inner(), Some(&app), &seasons_path, false, 86400, false).await
        {
            Ok(resp) => serde_json::from_str(&resp.body).unwrap_or_default(),
            Err(_) => Vec::new(),
        };

    // Resolve tmdb_id from DB if not passed
    let resolved_tmdb_id = match tmdb_id {
        Some(id) if id > 0 => Some(id),
        _ => {
            let conn = state.db.lock().await;
            conn.query_row(
                "SELECT tmdb_id FROM trakt_watchlist_cache WHERE trakt_id = ?1
                 UNION ALL
                 SELECT tmdb_id FROM trakt_cache WHERE media_type = 'show' AND trakt_id = ?1
                 LIMIT 1",
                params![show_trakt_id],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
        }
    };

    // Build watched lookup from DB
    let watched_map: HashMap<(i64, i64), Option<String>> = {
        let conn = state.db.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT season, episode, watched_at FROM trakt_episodes WHERE show_trakt_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map(params![show_trakt_id], |r| {
                Ok(((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?), r.get::<_, Option<String>>(2)?))
            })
            .map_err(|e| e.to_string())?;
        mapped.flatten().collect()
    };

    let tmdb_key = get_tmdb_api_key_from_state(state.inner()).await;
    let mut season_groups: Vec<TraktSeasonGroup> = Vec::new();

    for ts in trakt_seasons {
        if ts.episodes.is_empty() {
            continue;
        }

        // Fetch TMDB season details (1 call per season, cached 24h in trakt_http_cache) if TMDB key & tmdb_id exist
        let mut tmdb_ep_map: HashMap<i64, TmdbSeasonEpisodeApi> = HashMap::new();
        let mut tmdb_season_name: Option<String> = None;
        if let (Some(tid), Some(ref key)) = (resolved_tmdb_id, &tmdb_key) {
            let cache_key = format!("tmdb_season:{}:{}:{}", tid, ts.number, tmdb_lang);
            let cached_body: Option<String> = {
                let conn = state.db.lock().await;
                conn.query_row(
                    "SELECT body, fetched_at, ttl_seconds FROM trakt_http_cache WHERE key = ?1",
                    params![cache_key],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
                )
                .ok()
                .and_then(|(body, fetched_at, ttl)| {
                    let ep = crate::trakt::iso_utc_to_epoch(&fetched_at)?;
                    if ep + ttl > now_epoch_secs() {
                        Some(body)
                    } else {
                        None
                    }
                })
            };

            let season_json_opt = if let Some(body) = cached_body {
                serde_json::from_str::<TmdbSeasonApi>(&body).ok()
            } else {
                let url = format!(
                    "{}/tv/{}/season/{}",
                    state.tmdb.base, tid, ts.number
                );
                let req = apply_tmdb_auth(
                    state
                        .http
                        .get(&url)
                        .header("Accept", "application/json")
                        .query(&[("language", tmdb_lang.as_str())]),
                    key,
                );
                if let Ok(resp) = req.send().await {
                    if resp.status().is_success() {
                        if let Ok(text) = resp.text().await {
                            let conn = state.db.lock().await;
                            let _ = conn.execute(
                                "INSERT OR REPLACE INTO trakt_http_cache (key, etag, body, fetched_at, ttl_seconds)
                                 VALUES (?1, NULL, ?2, ?3, 86400)",
                                params![cache_key, text, now_iso_utc()],
                            );
                            serde_json::from_str::<TmdbSeasonApi>(&text).ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(tmdb_season) = season_json_opt {
                tmdb_season_name = tmdb_season.name;
                for tep in tmdb_season.episodes {
                    tmdb_ep_map.insert(tep.episode_number, tep);
                }
            }
        }

        let mut episodes_out: Vec<TraktSeasonEpisodeItem> = Vec::new();
        let mut completed_count = 0i64;

        for ep in ts.episodes {
            let watched_at = watched_map
                .get(&(ep.season, ep.number))
                .cloned()
                .flatten();
            let watched = watched_at.is_some();
            if watched {
                completed_count += 1;
            }

            let tmdb_ep = tmdb_ep_map.get(&ep.number);
            let title = tmdb_ep
                .and_then(|te| te.name.clone())
                .filter(|s| !s.trim().is_empty())
                .or(ep.title)
                .unwrap_or_else(|| format!("Episode {}", ep.number));
            let overview = tmdb_ep
                .and_then(|te| te.overview.clone())
                .filter(|s| !s.trim().is_empty())
                .or(ep.overview);
            let still_path = tmdb_ep.and_then(|te| te.still_path.clone());
            let air_date = tmdb_ep
                .and_then(|te| te.air_date.clone())
                .or(ep.first_aired);

            episodes_out.push(TraktSeasonEpisodeItem {
                season: ep.season,
                episode: ep.number,
                episode_trakt_id: Some(ep.ids.trakt),
                title,
                overview,
                still_path,
                air_date,
                watched,
                watched_at,
            });
        }

        let aired_count = episodes_out.len() as i64;
        let name = tmdb_season_name
            .or(ts.title)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                if ts.number == 0 {
                    "Specials".to_string()
                } else {
                    format!("Season {}", ts.number)
                }
            });

        season_groups.push(TraktSeasonGroup {
            season_number: ts.number,
            name,
            aired: aired_count,
            completed: completed_count,
            episodes: episodes_out,
        });
    }

    let (aired, completed, last_watched_at) = match progress_opt {
        Some(p) => (
            p.aired.unwrap_or(0),
            p.completed.unwrap_or(0),
            p.last_watched_at,
        ),
        None => {
            let a: i64 = season_groups.iter().map(|s| s.aired).sum();
            let c: i64 = season_groups.iter().map(|s| s.completed).sum();
            (a, c, None)
        }
    };

    Ok(TraktShowSeasonsProgress {
        show_trakt_id,
        tmdb_id: resolved_tmdb_id,
        aired,
        completed,
        last_watched_at,
        seasons: season_groups,
    })
}

#[tauri::command]
pub async fn trakt_get_calendar_my(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    start_date: Option<String>,
    force_refresh: Option<bool>,
) -> Result<Vec<CalendarEpisodeItem>, String> {
    let start = start_date
        .filter(|s| s.trim().len() >= 10)
        .unwrap_or_else(local_yesterday_ymd);
    let _ = fetch_and_cache_my_calendar(
        state.inner(),
        Some(&app),
        &start,
        force_refresh.unwrap_or(false),
    )
    .await;
    let conn = state.db.lock().await;
    query_calendar_episodes(&conn, "my", false)
}

#[tauri::command]
pub async fn trakt_get_calendar_global(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    start_date: Option<String>,
    force_refresh: Option<bool>,
) -> Result<Vec<CalendarEpisodeItem>, String> {
    let start = start_date
        .filter(|s| s.trim().len() >= 10)
        .unwrap_or_else(local_yesterday_ymd);
    let force = force_refresh.unwrap_or(false);

    let should_sync_wl = {
        let conn = state.db.lock().await;
        let logged_in: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM trakt_auth WHERE id = 1 AND access_blob != '')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        let never_synced = get_sync_state_val(&conn, "last_sync_ok_at").is_none();
        let wl_empty: bool = conn
            .query_row("SELECT COUNT(*) FROM trakt_watchlist_cache", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            == 0;
        logged_in && never_synced && wl_empty
    };
    if should_sync_wl {
        let _ = crate::trakt::sync::sync_watchlist(state.inner(), Some(&app)).await;
    }

    fetch_and_cache_global_calendar(state.inner(), Some(&app), &start, force).await?;
    // Best-effort refresh of personal calendar so user's own shows are always included in Global Calendar
    let _ = fetch_and_cache_my_calendar(state.inner(), Some(&app), &start, force).await;
    let conn = state.db.lock().await;
    query_global_calendar_merged(&conn)
}

#[tauri::command]
pub async fn trakt_match_titles(
    state: tauri::State<'_, AppState>,
    raw_titles: Vec<String>,
) -> Result<Vec<String>, String> {
    match_titles(state.inner(), raw_titles).await
}

#[tauri::command]
pub async fn trakt_find_epg_airings(
    state: tauri::State<'_, AppState>,
    titles: Vec<String>,
) -> Result<Vec<EpgAiring>, String> {
    let mut norms: Vec<String> = Vec::new();
    for t in titles {
        let n = normalise_for_match(&clean_title(&t));
        if !n.is_empty() && !norms.contains(&n) {
            norms.push(n);
        }
        let n_raw = normalise_for_match(&t);
        if !n_raw.is_empty() && !norms.contains(&n_raw) {
            norms.push(n_raw);
        }
    }

    if norms.is_empty() {
        return Ok(Vec::new());
    }

    let now_ts = now_epoch_secs();
    let placeholders = vec!["?"; norms.len()].join(", ");
    let sql = format!(
        "SELECT p.channel_id,
                COALESCE(NULLIF(cus.custom_name, ''), c.name, p.channel_id) AS channel_name,
                p.source_id,
                COALESCE(s.name, p.source_id) AS source_name,
                p.start,
                p.stop,
                p.start_ts,
                p.stop_ts,
                COALESCE(s.time_offset, 0) AS time_offset,
                p.title,
                COALESCE(p.desc, '')
         FROM programmes p
         JOIN sources s ON s.id = p.source_id
         LEFT JOIN channels c ON c.id = p.channel_id AND c.source_id = p.source_id
         LEFT JOIN channel_user_settings cus ON cus.source_id = p.source_id AND cus.channel_id = p.channel_id
         WHERE p.title_norm != ''
           AND p.title_norm IN ({})
           AND COALESCE(cus.is_hidden, 0) = 0
           AND (p.stop_ts + COALESCE(s.time_offset, 0) * 3600) >= ?
         ORDER BY (p.start_ts + COALESCE(s.time_offset, 0) * 3600) ASC
         LIMIT 100",
        placeholders
    );

    let conn = state.db.lock().await;
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    let mut param_values: Vec<rusqlite::types::Value> = norms
        .into_iter()
        .map(rusqlite::types::Value::Text)
        .collect();
    param_values.push(rusqlite::types::Value::Integer(now_ts));

    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_values), |r| {
            Ok(EpgAiring {
                channel_id: r.get(0)?,
                channel_name: r.get(1)?,
                source_id: r.get(2)?,
                source_name: r.get(3)?,
                start: r.get(4)?,
                stop: r.get(5)?,
                start_ts: r.get(6)?,
                stop_ts: r.get(7)?,
                time_offset: r.get(8)?,
                title: r.get(9)?,
                desc: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;

    Ok(rows.flatten().collect())
}

#[tauri::command]
pub async fn trakt_get_stats(
    state: tauri::State<'_, AppState>,
) -> Result<TraktStatsResponse, String> {
    let conn = state.db.lock().await;
    let watchlist_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_watchlist_cache", [], |r| r.get(0))
        .unwrap_or(0);
    let cache_items_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_cache", [], |r| r.get(0))
        .unwrap_or(0);
    let cache_calendar_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_calendar_cache", [], |r| r.get(0))
        .unwrap_or(0);
    let cache_history_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_history_cache", [], |r| r.get(0))
        .unwrap_or(0);
    let cache_episodes_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_episodes", [], |r| r.get(0))
        .unwrap_or(0);
    let pending_actions_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_pending_actions", [], |r| r.get(0))
        .unwrap_or(0);

    Ok(TraktStatsResponse {
        watchlist_count,
        episodes_watched: 0,
        movies_watched: 0,
        total_minutes: 0,
        cache_items_count,
        cache_calendar_count,
        cache_history_count,
        cache_episodes_count,
        pending_actions_count,
    })
}

#[tauri::command]
pub async fn trakt_clear_cache(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    {
        let mut conn = state.db.lock().await;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM trakt_cache", []).ok();
        tx.execute("DELETE FROM trakt_watchlist_cache", []).ok();
        tx.execute("DELETE FROM trakt_history_cache", []).ok();
        tx.execute("DELETE FROM trakt_calendar_cache", []).ok();
        tx.execute("DELETE FROM trakt_episodes", []).ok();
        tx.execute("DELETE FROM trakt_show_progress", []).ok();
        tx.execute("DELETE FROM trakt_http_cache", []).ok();
        tx.execute(
            "DELETE FROM trakt_sync_state WHERE key IN ('last_activities', 'last_sync_ok_at')",
            [],
        )
        .ok();
        tx.commit().map_err(|e| e.to_string())?;
    }

    let new_ver = rebuild_title_index(state.inner()).await;
    let _ = app.emit("trakt-watchlist-changed", new_ver);
    Ok(())
}

#[tauri::command]
pub async fn trakt_rebuild_index(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<u64, String> {
    let new_ver = rebuild_title_index(state.inner()).await;
    let _ = app.emit("trakt-watchlist-changed", new_ver);
    Ok(new_ver)
}

#[tauri::command]
pub async fn trakt_resolve_for_title(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    title: String,
    tmdb_id: Option<i64>,
    media_type: Option<String>,
) -> Result<Option<TraktModalData>, String> {
    let (enabled, logged_in) = {
        let conn = state.db.lock().await;
        let en = is_trakt_enabled(&conn);
        let li: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM trakt_auth WHERE id = 1 AND access_blob != '')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        (en, li)
    };

    if !enabled {
        return Ok(None);
    }

    let year_hint = extract_year_hint(&title);
    let clean = clean_title(&title);
    let norm = normalise_for_match(&clean);

    // 1. Local Watchlist match (0 HTTP calls)
    let wl_trakt_id: Option<(i64, String)> = {
        let idx = state.trakt.title_index.read().await;
        let conn = state.db.lock().await;
        if let Some(refs) = idx.lookup(&norm) {
            if let Some(tid) = tmdb_id {
                refs.iter()
                    .find(|r| r.tmdb_id == Some(tid))
                    .or_else(|| refs.iter().find(|r| r.tmdb_id.is_none()))
                    .map(|r| (r.trakt_id, r.media_type.clone()))
            } else if crate::trakt::matcher::check_title_against_index(&conn, &idx, &title) {
                refs.first().map(|r| (r.trakt_id, r.media_type.clone()))
            } else {
                None
            }
        } else {
            None
        }
    };

    if let Some((trakt_id, mt)) = wl_trakt_id {
        if let Some(data) = load_modal_data_from_cache(state.inner(), trakt_id, &mt, logged_in).await {
            return Ok(Some(data));
        }
    }

    // 2. Check trakt_cache by (media_type, tmdb_id)
    let trakt_mt_opt = media_type
        .as_deref()
        .map(|m| if m == "tv" || m == "show" { "show" } else { "movie" });
    if let (Some(tid), Some(tmt)) = (tmdb_id, trakt_mt_opt) {
        let cached_row: Option<(i64, bool)> = {
            let conn = state.db.lock().await;
            conn.query_row(
                "SELECT trakt_id, not_found = 1 FROM trakt_cache
                 WHERE media_type = ?1 AND tmdb_id = ?2
                   AND updated_at > datetime('now', '-14 days')
                 LIMIT 1",
                params![tmt, tid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
        };

        if let Some((trakt_id, is_not_found)) = cached_row {
            if is_not_found {
                return Ok(None);
            }
            if let Some(data) =
                load_modal_data_from_cache(state.inner(), trakt_id, tmt, logged_in).await
            {
                return Ok(Some(data));
            }
        }

        // 3. Trakt API: GET /search/tmdb/{id}?type=show|movie&extended=full
        let path = format!("/search/tmdb/{}?type={}&extended=full", tid, tmt);
        if let Ok(resp) = trakt_get_cached(state.inner(), Some(&app), &path, false, 86400, false).await {
            if let Ok(results) = serde_json::from_str::<Vec<TraktSearchResult>>(&resp.body) {
                if let Some(first) = results.into_iter().next() {
                    if let Some(data) =
                        cache_and_build_modal_data(state.inner(), first, logged_in).await
                    {
                        return Ok(Some(data));
                    }
                } else {
                    // Negative cache for this tmdb_id
                    let conn = state.db.lock().await;
                    let _ = conn.execute(
                        "INSERT OR REPLACE INTO trakt_cache (media_type, trakt_id, tmdb_id, not_found, updated_at)
                         VALUES (?1, ?2, ?3, 1, ?4)",
                        params![tmt, -tid, tid, now_iso_utc()],
                    );
                    return Ok(None);
                }
            }
        }
    }

    // 4. Fallback: title search if TMDB failed or no TMDB key
    if clean.is_empty() {
        return Ok(None);
    }

    // Check negative cache by title in trakt_http_cache / trakt_cache
    let neg_key = format!("neg_title:{}", norm);
    {
        let conn = state.db.lock().await;
        let neg_hit: Option<String> = conn
            .query_row(
                "SELECT fetched_at FROM trakt_http_cache WHERE key = ?1",
                params![neg_key],
                |r| r.get(0),
            )
            .ok();
        if let Some(ts) = neg_hit {
            if let Some(ep) = crate::trakt::iso_utc_to_epoch(&ts) {
                if now_epoch_secs() - ep < 7 * 86400 {
                    return Ok(None);
                }
            }
        }
    }

    let encoded_q = url_encode_query(&clean);
    let search_path = format!("/search/show,movie?query={}&extended=full&limit=8", encoded_q);
    if let Ok(resp) =
        trakt_get_cached(state.inner(), Some(&app), &search_path, false, 86400, false).await
    {
        if let Ok(results) = serde_json::from_str::<Vec<TraktSearchResult>>(&resp.body) {
            let best = results
                .into_iter()
                .filter_map(|r| {
                    let media = match r.item_type.as_str() {
                        "show" => r.show.as_ref(),
                        "movie" => r.movie.as_ref(),
                        _ => None,
                    }?;
                    let score = tmdb_title_score(
                        &clean,
                        &media.title,
                        None,
                        year_hint,
                        media.year.map(|y| y as u32),
                    );
                    Some((score, r))
                })
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            if let Some((score, best_res)) = best {
                if score >= 0.55 {
                    if let Some(data) =
                        cache_and_build_modal_data(state.inner(), best_res, logged_in).await
                    {
                        return Ok(Some(data));
                    }
                }
            }

            // Store negative cache for 7 days
            let conn = state.db.lock().await;
            let _ = conn.execute(
                "INSERT OR REPLACE INTO trakt_http_cache (key, etag, body, fetched_at, ttl_seconds)
                 VALUES (?1, NULL, 'none', ?2, 604800)",
                params![neg_key, now_iso_utc()],
            );
        }
    }

    Ok(None)
}

async fn load_modal_data_from_cache(
    state: &AppState,
    trakt_id: i64,
    media_type: &str,
    logged_in: bool,
) -> Option<TraktModalData> {
    let conn = state.db.lock().await;
    let on_watchlist: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM trakt_watchlist_cache WHERE trakt_id = ?1)",
            params![trakt_id],
            |r| r.get(0),
        )
        .unwrap_or(false);

    conn.query_row(
        "SELECT tc.trakt_id, tc.tmdb_id, tc.media_type, COALESCE(tc.title, w.title, ''),
                COALESCE(tc.year, w.year), tc.rating, tc.votes, tc.status, tc.network, tc.overview
         FROM trakt_cache tc
         LEFT JOIN trakt_watchlist_cache w ON w.trakt_id = tc.trakt_id
         WHERE tc.media_type = ?1 AND tc.trakt_id = ?2 AND tc.not_found = 0",
        params![media_type, trakt_id],
        |r| {
            Ok(TraktModalData {
                trakt_id: r.get(0)?,
                tmdb_id: r.get(1)?,
                media_type: r.get(2)?,
                title: r.get(3)?,
                year: r.get(4)?,
                rating: r.get(5)?,
                votes: r.get(6)?,
                status: r.get(7)?,
                network: r.get(8)?,
                overview: r.get(9)?,
                on_watchlist,
                logged_in,
            })
        },
    )
    .ok()
    .or_else(|| {
        conn.query_row(
            "SELECT trakt_id, tmdb_id, media_type, title, year FROM trakt_watchlist_cache WHERE trakt_id = ?1",
            params![trakt_id],
            |r| {
                Ok(TraktModalData {
                    trakt_id: r.get(0)?,
                    tmdb_id: r.get(1)?,
                    media_type: r.get(2)?,
                    title: r.get(3)?,
                    year: r.get(4)?,
                    rating: None,
                    votes: None,
                    status: None,
                    network: None,
                    overview: None,
                    on_watchlist: true,
                    logged_in,
                })
            },
        )
        .ok()
    })
}

async fn cache_and_build_modal_data(
    state: &AppState,
    res: TraktSearchResult,
    logged_in: bool,
) -> Option<TraktModalData> {
    let (mt, media) = match res.item_type.as_str() {
        "show" => ("show", res.show?),
        "movie" => ("movie", res.movie?),
        _ => return None,
    };

    let now = now_iso_utc();
    let conn = state.db.lock().await;
    let _ = conn.execute(
        "INSERT INTO trakt_cache (
            media_type, trakt_id, tmdb_id, imdb_id, slug, title, year, overview,
            rating, votes, status, network, data_json, not_found, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, 0, ?13)
        ON CONFLICT(media_type, trakt_id) DO UPDATE SET
            tmdb_id = COALESCE(excluded.tmdb_id, trakt_cache.tmdb_id),
            imdb_id = COALESCE(excluded.imdb_id, trakt_cache.imdb_id),
            slug = COALESCE(excluded.slug, trakt_cache.slug),
            title = excluded.title,
            year = excluded.year,
            overview = COALESCE(excluded.overview, trakt_cache.overview),
            rating = COALESCE(excluded.rating, trakt_cache.rating),
            votes = COALESCE(excluded.votes, trakt_cache.votes),
            status = COALESCE(excluded.status, trakt_cache.status),
            network = COALESCE(excluded.network, trakt_cache.network),
            not_found = 0,
            updated_at = excluded.updated_at",
        params![
            mt,
            media.ids.trakt,
            media.ids.tmdb,
            media.ids.imdb,
            media.ids.slug,
            media.title,
            media.year,
            media.overview,
            media.rating,
            media.votes,
            media.status,
            media.network,
            now,
        ],
    );

    let on_watchlist: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM trakt_watchlist_cache WHERE trakt_id = ?1)",
            params![media.ids.trakt],
            |r| r.get(0),
        )
        .unwrap_or(false);

    Some(TraktModalData {
        trakt_id: media.ids.trakt,
        tmdb_id: media.ids.tmdb,
        media_type: mt.to_string(),
        title: media.title,
        year: media.year,
        rating: media.rating,
        votes: media.votes,
        status: media.status,
        network: media.network,
        overview: media.overview,
        on_watchlist,
        logged_in,
    })
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPosterResult {
    pub poster_path: Option<String>,
    pub data_uri: Option<String>,
}

#[tauri::command]
pub async fn trakt_resolve_tmdb_poster(
    state: tauri::State<'_, AppState>,
    tmdb_id: i64,
    media_type: String,
    size: Option<String>,
) -> Result<ResolvedPosterResult, String> {
    let tmdb_mt = if media_type == "show" || media_type == "tv" {
        "tv"
    } else {
        "movie"
    };
    let details = match get_tmdb_by_id(state.inner(), tmdb_id, tmdb_mt, "hu-HU", None, false).await {
        Ok(d) => d,
        Err(_) => {
            return Ok(ResolvedPosterResult {
                poster_path: None,
                data_uri: None,
            });
        }
    };

    let Some(poster_path) = details.poster_path.filter(|p| !p.trim().is_empty()) else {
        return Ok(ResolvedPosterResult {
            poster_path: None,
            data_uri: None,
        });
    };

    let sz = size.unwrap_or_else(|| "w185".to_string());
    let url = format!("https://image.tmdb.org/t/p/{}{}", sz, poster_path);
    let data_uri = cache_logo_inner(state.inner(), url).await.ok();

    Ok(ResolvedPosterResult {
        poster_path: Some(poster_path),
        data_uri,
    })
}

#[tauri::command]
pub async fn get_tmdb_details_by_id(
    state: tauri::State<'_, AppState>,
    tmdb_id: i64,
    media_type: String,
    lang: Option<String>,
) -> Result<Option<crate::TmdbDetails>, String> {
    let tmdb_mt = if media_type == "show" || media_type == "tv" {
        "tv"
    } else {
        "movie"
    };
    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());
    match get_tmdb_by_id(state.inner(), tmdb_id, tmdb_mt, &target_lang, None, false).await {
        Ok(d) => Ok(Some(d)),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
pub async fn trakt_on_window_focus(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<bool, String> {
    let should_sync = {
        let conn = state.db.lock().await;
        if !is_trakt_enabled(&conn) {
            return Ok(false);
        }
        let logged_in: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM trakt_auth WHERE id = 1 AND access_blob != '')",
                [],
                |r| r.get(0),
            )
            .unwrap_or(false);
        logged_in
    };

    if !should_sync {
        return Ok(false);
    }

    {
        let mut last_guard = state.trakt.last_focus_sync.lock().await;
        let now = std::time::Instant::now();
        if let Some(prev) = *last_guard {
            if now.duration_since(prev) < std::time::Duration::from_secs(900) {
                return Ok(false);
            }
        }
        *last_guard = Some(now);
    }

    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Manager;
        let st = app_clone.state::<AppState>();
        let _ = run_sync(st.inner(), Some(&app_clone), false).await;
    });

    Ok(true)
}

fn url_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
