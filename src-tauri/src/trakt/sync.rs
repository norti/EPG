use crate::trakt::auth::{ensure_valid_token, fetch_and_save_profile};
use crate::trakt::calendar::fetch_and_cache_my_calendar;
use crate::trakt::client::{trakt_get_cached, trakt_get_raw, trakt_post, TraktApiError};
use crate::trakt::matcher::rebuild_title_index;
use crate::trakt::{is_trakt_enabled, iso_utc_to_epoch, now_epoch_secs, now_iso_utc};
use crate::{get_tmdb_api_key_from_state, get_tmdb_by_id, AppState};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use tauri::Emitter;

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct LastActivities {
    #[serde(default)]
    all: Option<String>,
    #[serde(default)]
    movies: Option<ActivitySection>,
    #[serde(default)]
    episodes: Option<ActivitySection>,
    #[serde(default)]
    shows: Option<ActivitySection>,
    #[serde(default)]
    watchlist: Option<WatchlistActivity>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct ActivitySection {
    #[serde(default)]
    watched_at: Option<String>,
    #[serde(default)]
    watchlisted_at: Option<String>,
    #[serde(default)]
    rated_at: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct WatchlistActivity {
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub watchlist_updated: bool,
    pub history_updated: bool,
    pub calendar_updated: bool,
    pub pending_flushed: usize,
    pub last_sync_ok_at: String,
}

#[derive(Deserialize, Debug)]
pub struct TraktMediaIds {
    pub trakt: i64,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub slug: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TraktMediaObj {
    title: String,
    year: Option<i64>,
    overview: Option<String>,
    rating: Option<f64>,
    votes: Option<i64>,
    status: Option<String>,
    network: Option<String>,
    ids: TraktMediaIds,
}

#[derive(Deserialize, Debug)]
struct TraktWatchlistEntry {
    listed_at: String,
    #[serde(rename = "type")]
    item_type: String,
    movie: Option<TraktMediaObj>,
    show: Option<TraktMediaObj>,
}

#[derive(Deserialize, Debug)]
struct TraktHistoryEntry {
    id: i64,
    watched_at: String,
    #[serde(rename = "type")]
    item_type: String,
    movie: Option<TraktMediaObj>,
    show: Option<TraktMediaObj>,
    episode: Option<TraktHistoryEpisode>,
}

#[derive(Deserialize, Debug)]
struct TraktHistoryEpisode {
    season: i64,
    number: i64,
    title: Option<String>,
    ids: TraktMediaIds,
}

#[derive(Deserialize, Debug)]
struct TraktAlias {
    title: String,
    #[serde(default)]
    country: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct TraktProgressNextEpisode {
    pub season: i64,
    pub number: i64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub first_aired: Option<String>,
    pub ids: TraktMediaIds,
}

#[derive(Deserialize, Debug)]
struct TraktEpisodeExtended {
    #[serde(default)]
    first_aired: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct TraktProgressResponse {
    pub aired: Option<i64>,
    pub completed: Option<i64>,
    pub last_watched_at: Option<String>,
    #[serde(default)]
    pub next_episode: Option<TraktProgressNextEpisode>,
    #[serde(default)]
    pub seasons: Vec<TraktProgressSeason>,
}

#[derive(Deserialize, Debug)]
pub struct TraktProgressSeason {
    pub number: i64,
    #[serde(default)]
    pub episodes: Vec<TraktProgressEpisode>,
}

#[derive(Deserialize, Debug)]
pub struct TraktProgressEpisode {
    pub number: i64,
    pub completed: bool,
    pub last_watched_at: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TraktWatchedShowEntry {
    #[serde(default)]
    reset_at: Option<String>,
    show: TraktWatchedShowRef,
    #[serde(default)]
    seasons: Vec<TraktWatchedSeason>,
}

#[derive(Deserialize, Debug)]
struct TraktWatchedShowRef {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    year: Option<i64>,
    ids: TraktMediaIds,
}

#[derive(Deserialize, Debug)]
struct TraktHiddenEntry {
    #[serde(default)]
    show: Option<TraktWatchedShowRef>,
}

#[derive(Deserialize, Debug)]
struct TraktWatchedSeason {
    number: i64,
    #[serde(default)]
    episodes: Vec<TraktWatchedEpisode>,
}

#[derive(Deserialize, Debug)]
struct TraktWatchedEpisode {
    number: i64,
    #[serde(default)]
    last_watched_at: Option<String>,
}

fn emit_sync_state(
    app: Option<&tauri::AppHandle>,
    status: &str,
    step: &str,
    last_sync_ok_at: Option<&str>,
    error: Option<&str>,
) {
    if let Some(handle) = app {
        let _ = handle.emit(
            "trakt-sync-state",
            json!({
                "status": status,
                "step": step,
                "lastSyncOkAt": last_sync_ok_at,
                "error": error,
            }),
        );
    }
}

pub fn get_sync_state_val(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM trakt_sync_state WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .ok()
}

pub fn set_sync_state_val(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO trakt_sync_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn local_yesterday_ymd() -> String {
    // Conservative UTC-1 day fallback so frontend/backend always covers local yesterday
    let ts = now_epoch_secs() - 86400;
    crate::trakt::epoch_to_iso_utc(ts)[..10].to_string()
}

const CALENDAR_START_KEY: &str = "calendar_start_ymd";

fn is_ymd(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

/// Seconds since the epoch of `ymd` 00:00 UTC.
fn ymd_to_epoch(ymd: &str) -> Option<i64> {
    crate::trakt::iso_utc_to_epoch(&format!("{}T00:00:00Z", ymd))
}

/// The UI asks for the calendar with *its* local "yesterday"; the background sync only knows a UTC based
/// guess. When they differ the two requests have different cache keys and the same 31-day calendar is
/// downloaded twice. The UI's date is therefore remembered and reused by the sync.
pub fn remember_calendar_start(conn: &Connection, ymd: &str) {
    if !is_ymd(ymd) {
        return;
    }
    if get_sync_state_val(conn, CALENDAR_START_KEY).as_deref() != Some(ymd) {
        let _ = set_sync_state_val(conn, CALENDAR_START_KEY, ymd);
    }
}

/// Start date for a calendar download that has no explicit UI request (background sync).
/// The remembered UI date is used only while it is plausible (at most 2 days older / 1 day newer than the
/// UTC based guess); otherwise (app was closed for days, clock jump) the guess is used.
pub fn calendar_start_for_sync(conn: &Connection) -> String {
    let guess = local_yesterday_ymd();
    let Some(stored) = get_sync_state_val(conn, CALENDAR_START_KEY) else {
        return guess;
    };
    if !is_ymd(&stored) {
        return guess;
    }
    match (ymd_to_epoch(&stored), ymd_to_epoch(&guess)) {
        (Some(s), Some(g)) if s - g >= -2 * 86_400 && s - g <= 86_400 => stored,
        _ => guess,
    }
}

/// Resolves the start date of a calendar request: the UI's explicit date (and remembers it),
/// otherwise the same date the background sync uses.
pub async fn resolve_calendar_start(state: &AppState, requested: Option<String>) -> String {
    let conn = state.db.lock().await;
    match requested.map(|s| s.trim().to_string()).filter(|s| s.len() >= 10) {
        Some(s) => {
            remember_calendar_start(&conn, &s);
            s
        }
        None => calendar_start_for_sync(&conn),
    }
}

pub async fn flush_pending_actions(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
) -> Result<usize, String> {
    let rows: Vec<(i64, String, String, i64)> = {
        let conn = state.db.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT id, action, payload_json, attempts FROM trakt_pending_actions ORDER BY id ASC",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .map_err(|e| e.to_string())?;
        mapped.flatten().collect()
    };

    let mut flushed = 0usize;
    for (id, action, payload_json, attempts) in rows {
        let payload: serde_json::Value = match serde_json::from_str(&payload_json) {
            Ok(v) => v,
            Err(_) => {
                let conn = state.db.lock().await;
                let _ = conn.execute("DELETE FROM trakt_pending_actions WHERE id = ?1", params![id]);
                continue;
            }
        };

        let path = match action.as_str() {
            "history_add" => "/sync/history",
            "watchlist_add" => "/sync/watchlist",
            "watchlist_remove" => "/sync/watchlist/remove",
            _ => {
                let conn = state.db.lock().await;
                let _ = conn.execute("DELETE FROM trakt_pending_actions WHERE id = ?1", params![id]);
                continue;
            }
        };

        match trakt_post(state, app, path, &payload).await {
            Ok(_) => {
                let conn = state.db.lock().await;
                let _ = conn.execute("DELETE FROM trakt_pending_actions WHERE id = ?1", params![id]);
                flushed += 1;
            }
            Err(err) if err.is_transient() => {
                let conn = state.db.lock().await;
                let _ = conn.execute(
                    "UPDATE trakt_pending_actions SET attempts = ?1, last_error = ?2 WHERE id = ?3",
                    params![attempts + 1, err.to_user_string(), id],
                );
                return Err(err.to_user_string());
            }
            Err(err) => {
                let conn = state.db.lock().await;
                if matches!(
                    err,
                    TraktApiError::AccountLimitExceeded(_)
                        | TraktApiError::VipRequired(_)
                        | TraktApiError::LockedAccount(_)
                        | TraktApiError::Http(400..=499, _)
                ) || attempts >= 3
                {
                    let _ = conn.execute("DELETE FROM trakt_pending_actions WHERE id = ?1", params![id]);
                } else {
                    let _ = conn.execute(
                        "UPDATE trakt_pending_actions SET attempts = ?1, last_error = ?2 WHERE id = ?3",
                        params![attempts + 1, err.to_user_string(), id],
                    );
                }
            }
        }
    }

    Ok(flushed)
}

pub async fn run_sync(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    force: bool,
) -> Result<SyncSummary, String> {
    let _sync_guard = state.trakt.sync_lock.lock().await;

    let (enabled, last_ok_prev) = {
        let conn = state.db.lock().await;
        (
            is_trakt_enabled(&conn),
            get_sync_state_val(&conn, "last_sync_ok_at"),
        )
    };

    if !enabled {
        return Err("A Trakt integráció ki van kapcsolva.".to_string());
    }

    emit_sync_state(app, "syncing", "pending", last_ok_prev.as_deref(), None);

    // 1. Flush offline pending queue
    let pending_flushed = match flush_pending_actions(state, app).await {
        Ok(n) => n,
        Err(e) => {
            if e.contains("TRAKT_OFFLINE") || e.contains("TRAKT_ERR_429") {
                emit_sync_state(app, "offline", "", last_ok_prev.as_deref(), Some(&e));
            } else {
                emit_sync_state(app, "error", "", last_ok_prev.as_deref(), Some(&e));
            }
            return Err(e);
        }
    };

    // 2. Ensure valid token
    if let Err(e) = ensure_valid_token(state, app).await {
        emit_sync_state(app, "error", "", last_ok_prev.as_deref(), Some(&e));
        return Err(e);
    }

    // 3. Fetch /sync/last_activities
    emit_sync_state(app, "syncing", "activities", last_ok_prev.as_deref(), None);
    let act_resp = match trakt_get_raw(state, app, "/sync/last_activities", true).await {
        Ok(r) => r,
        Err(e) => {
            let status = if e.is_transient() { "offline" } else { "error" };
            let msg = e.to_user_string();
            emit_sync_state(app, status, "", last_ok_prev.as_deref(), Some(&msg));
            return Err(msg);
        }
    };

    let activities: LastActivities = serde_json::from_str(&act_resp.body)
        .map_err(|e| format!("last_activities JSON hiba: {}", e))?;

    let (stored_act, wl_empty, hist_empty, has_profile, history_dirty, watched_shows_synced, cw_synced) = {
        let conn = state.db.lock().await;
        let stored: Option<LastActivities> = get_sync_state_val(&conn, "last_activities")
            .and_then(|s| serde_json::from_str(&s).ok());
        let wl_empty: bool = conn
            .query_row("SELECT COUNT(*) FROM trakt_watchlist_cache", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            == 0;
        let hist_empty: bool = conn
            .query_row("SELECT COUNT(*) FROM trakt_history_cache", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            == 0;
        let has_prof = get_sync_state_val(&conn, "profile_json").is_some();
        let dirty = get_sync_state_val(&conn, "history_dirty").as_deref() == Some("1");
        let ws_synced =
            get_sync_state_val(&conn, "watched_shows_synced_v1").as_deref() == Some("1");
        let cw_done =
            get_sync_state_val(&conn, "continue_watching_premiered_v1").as_deref() == Some("1");
        (stored, wl_empty, hist_empty, has_prof, dirty, ws_synced, cw_done)
    };

    if !has_profile {
        let _ = fetch_and_save_profile(state).await;
    }

    let wl_changed = force
        || wl_empty
        || stored_act.as_ref().map_or(true, |s| {
            s.watchlist.as_ref().and_then(|w| w.updated_at.as_ref())
                != activities.watchlist.as_ref().and_then(|w| w.updated_at.as_ref())
        });

    let hist_changed = force
        || hist_empty
        || history_dirty
        || !watched_shows_synced
        || stored_act.as_ref().map_or(true, |s| {
            s.episodes.as_ref().and_then(|e| e.watched_at.as_ref())
                != activities.episodes.as_ref().and_then(|e| e.watched_at.as_ref())
                || s.movies.as_ref().and_then(|m| m.watched_at.as_ref())
                    != activities.movies.as_ref().and_then(|m| m.watched_at.as_ref())
        });

    if wl_changed {
        emit_sync_state(app, "syncing", "watchlist", last_ok_prev.as_deref(), None);
        if let Err(e) = sync_watchlist(state, app).await {
            emit_sync_state(app, "error", "", last_ok_prev.as_deref(), Some(&e));
            return Err(e);
        }
    }

    if hist_changed {
        emit_sync_state(app, "syncing", "history", last_ok_prev.as_deref(), None);
        if let Err(e) = sync_history(state).await {
            emit_sync_state(app, "error", "", last_ok_prev.as_deref(), Some(&e));
            return Err(e);
        }
        if let Err(e) = sync_watched_shows(state, app).await {
            emit_sync_state(app, "error", "", last_ok_prev.as_deref(), Some(&e));
            return Err(e);
        }
        let conn = state.db.lock().await;
        let _ = set_sync_state_val(&conn, "history_dirty", "0");
        let _ = set_sync_state_val(&conn, "watched_shows_synced_v1", "1");
    }

    emit_sync_state(app, "syncing", "calendar", last_ok_prev.as_deref(), None);
    let start_date = {
        let conn = state.db.lock().await;
        calendar_start_for_sync(&conn)
    };
    let calendar_updated = match fetch_and_cache_my_calendar(state, app, &start_date, force).await {
        Ok(_) => true,
        Err(e) => {
            emit_sync_state(app, "error", "", last_ok_prev.as_deref(), Some(&e));
            return Err(e);
        }
    };

    if hist_changed || !cw_synced {
        let hidden_ids = fetch_hidden_progress_show_ids(state, app).await;
        let conn = state.db.lock().await;
        let _ = conn.execute(
            "UPDATE trakt_show_progress SET hidden_from_progress = 0 WHERE hidden_from_progress != 0",
            [],
        );
        for hid in hidden_ids {
            let _ = conn.execute(
                "INSERT INTO trakt_show_progress (show_trakt_id, hidden_from_progress, updated_at)
                 VALUES (?1, 1, ?2)
                 ON CONFLICT(show_trakt_id) DO UPDATE SET hidden_from_progress = 1",
                params![hid, now_iso_utc()],
            );
        }
    }

    // Refresh progress & next_episode for calendar shows and started (continue watching) shows
    let progress_show_ids: Vec<i64> = {
        let conn = state.db.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT show_trakt_id FROM (
                    SELECT c.show_trakt_id AS show_trakt_id, 0 AS prio, MAX(c.first_aired) AS sort_ts
                    FROM trakt_calendar_cache c
                    WHERE c.scope = 'my'
                    GROUP BY c.show_trakt_id
                    UNION ALL
                    SELECT e.show_trakt_id AS show_trakt_id,
                           CASE
                               WHEN p.show_trakt_id IS NULL THEN 1
                               WHEN p.next_episode_trakt_id IS NOT NULL OR COALESCE(p.completed, 0) < COALESCE(p.aired, 1) THEN 2
                               ELSE 3
                           END AS prio,
                           MAX(e.watched_at) AS sort_ts
                    FROM trakt_episodes e
                    LEFT JOIN trakt_show_progress p ON p.show_trakt_id = e.show_trakt_id
                    WHERE e.watched_at IS NOT NULL
                      AND COALESCE(p.hidden_from_progress, 0) = 0
                    GROUP BY e.show_trakt_id
                )
                GROUP BY show_trakt_id
                ORDER BY MIN(prio) ASC, MAX(sort_ts) DESC
                LIMIT 35",
            )
            .map_err(|e| e.to_string())?;
        let mapped = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?;
        mapped.flatten().collect()
    };

    for show_id in progress_show_ids {
        let _ = sync_show_progress(state, app, show_id, hist_changed || !cw_synced).await;
    }

    if hist_changed || !cw_synced {
        enrich_missing_continue_watching_tmdb(state).await;
        let conn = state.db.lock().await;
        let _ = set_sync_state_val(&conn, "continue_watching_premiered_v1", "1");
    }

    let now_ok = now_iso_utc();
    {
        let conn = state.db.lock().await;
        set_sync_state_val(&conn, "last_sync_ok_at", &now_ok)?;
        if let Ok(act_json) = serde_json::to_string(&activities) {
            let _ = set_sync_state_val(&conn, "last_activities", &act_json);
        }
        // Housekeeping: drop HTTP cache rows that expired long ago (daily calendar keys, old progress bodies ...)
        let _ = crate::trakt::client::prune_http_cache(&conn);
    }

    emit_sync_state(app, "idle", "", Some(&now_ok), None);

    Ok(SyncSummary {
        watchlist_updated: wl_changed,
        history_updated: hist_changed,
        calendar_updated,
        pending_flushed,
        last_sync_ok_at: now_ok,
    })
}

pub async fn sync_watchlist(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
) -> Result<(), String> {
    let mut page = 1u32;
    let mut seen_ids: HashSet<i64> = HashSet::new();
    let mut to_enrich_tmdb: Vec<(i64, String, i64)> = Vec::new();
    let mut to_fetch_aliases: Vec<(i64, String)> = Vec::new();
    let now = now_iso_utc();

    loop {
        let path = format!(
            "/sync/watchlist/all/added/desc?extended=full&page={}&limit=200",
            page
        );
        let resp = trakt_get_raw(state, app, &path, true)
            .await
            .map_err(|e| e.to_user_string())?;

        let entries: Vec<TraktWatchlistEntry> = serde_json::from_str(&resp.body)
            .map_err(|e| format!("Watchlist JSON hiba: {}", e))?;

        if entries.is_empty() {
            break;
        }

        {
            let mut conn = state.db.lock().await;
            let tx = conn.transaction().map_err(|e| e.to_string())?;

            for entry in &entries {
                let (media_type, media_opt) = match entry.item_type.as_str() {
                    "movie" => ("movie", entry.movie.as_ref()),
                    "show" => ("show", entry.show.as_ref()),
                    "season" | "episode" => {
                        // Handle season/episode watchlist entries gracefully by indexing parent show if present
                        if entry.show.is_some() {
                            ("show", entry.show.as_ref())
                        } else {
                            continue;
                        }
                    }
                    _ => continue,
                };

                let Some(media) = media_opt else {
                    continue;
                };

                let trakt_id = media.ids.trakt;
                seen_ids.insert(trakt_id);

                // Check existing tmdb_cache for hu_title & poster_path (0 HTTP calls)
                let tmdb_media = if media_type == "show" { "tv" } else { "movie" };
                let (cached_hu, cached_poster): (Option<String>, Option<String>) = media
                    .ids
                    .tmdb
                    .and_then(|tid| {
                        tx.query_row(
                            "SELECT hu_title, poster_path FROM tmdb_cache
                             WHERE tmdb_id = ?1 AND media_type = ?2 AND media_type != 'none'
                             ORDER BY CASE WHEN lang = 'hu-HU' THEN 0 ELSE 1 END
                             LIMIT 1",
                            params![tid, tmdb_media],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .ok()
                    })
                    .unwrap_or((None, None));

                tx.execute(
                    "INSERT INTO trakt_watchlist_cache (
                        trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id, poster_path, listed_at, updated_at
                    ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9)
                    ON CONFLICT(trakt_id) DO UPDATE SET
                        media_type = excluded.media_type,
                        title = excluded.title,
                        title_hu = COALESCE(excluded.title_hu, trakt_watchlist_cache.title_hu),
                        year = excluded.year,
                        tmdb_id = excluded.tmdb_id,
                        poster_path = COALESCE(excluded.poster_path, trakt_watchlist_cache.poster_path),
                        listed_at = excluded.listed_at,
                        updated_at = excluded.updated_at",
                    params![
                        trakt_id,
                        media_type,
                        media.title,
                        cached_hu,
                        media.year,
                        media.ids.tmdb,
                        cached_poster,
                        entry.listed_at,
                        now,
                    ],
                )
                .map_err(|e| e.to_string())?;

                tx.execute(
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
                        media_type,
                        trakt_id,
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
                )
                .map_err(|e| e.to_string())?;

                if let Some(tid) = media.ids.tmdb {
                    if cached_poster.is_none() || cached_hu.is_none() {
                        to_enrich_tmdb.push((trakt_id, tmdb_media.to_string(), tid));
                    }
                }
                if media_type == "show" {
                    let has_aliases: bool = tx
                        .query_row(
                            "SELECT aliases_json IS NOT NULL FROM trakt_watchlist_cache WHERE trakt_id = ?1",
                            params![trakt_id],
                            |r| r.get(0),
                        )
                        .unwrap_or(false);
                    if !has_aliases {
                        to_fetch_aliases.push((trakt_id, media_type.to_string()));
                    }
                }
            }

            tx.commit().map_err(|e| e.to_string())?;
        }

        let total_pages = resp.page_count.unwrap_or(1);
        if page >= total_pages {
            break;
        }
        page += 1;
    }

    // Remove items deleted from Trakt Watchlist
    {
        let mut conn = state.db.lock().await;
        let existing_ids: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT trakt_id FROM trakt_watchlist_cache")
                .map_err(|e| e.to_string())?;
            let mapped = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?;
            mapped.flatten().collect()
        };
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for eid in existing_ids {
            if !seen_ids.contains(&eid) {
                tx.execute(
                    "DELETE FROM trakt_watchlist_cache WHERE trakt_id = ?1",
                    params![eid],
                )
                .ok();
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
    }

    // TMDB enrichment for all missing watchlist items (only if TMDB API key is configured)
    if get_tmdb_api_key_from_state(state).await.is_some() {
        for (trakt_id, tmdb_media, tmdb_id) in to_enrich_tmdb {
            if let Ok(details) = get_tmdb_by_id(state, tmdb_id, &tmdb_media, "hu-HU", None, false).await {
                let conn = state.db.lock().await;
                let _ = conn.execute(
                    "UPDATE trakt_watchlist_cache
                     SET title_hu = COALESCE(?1, title_hu),
                         poster_path = COALESCE(?2, poster_path)
                     WHERE trakt_id = ?3",
                    params![details.hu_title, details.poster_path, trakt_id],
                );
            }
        }
    }

    // Fetch aliases for up to 10 shows per sync (cached 30 days)
    for (trakt_id, _) in to_fetch_aliases.into_iter().take(10) {
        let alias_path = format!("/shows/{}/aliases", trakt_id);
        if let Ok(resp) = trakt_get_cached(state, app, &alias_path, false, 2_592_000, false).await {
            if let Ok(aliases) = serde_json::from_str::<Vec<TraktAlias>>(&resp.body) {
                let filtered: Vec<String> = aliases
                    .into_iter()
                    .filter(|a| {
                        matches!(
                            a.country.as_deref(),
                            Some("hu") | Some("us") | Some("gb") | None
                        )
                    })
                    .map(|a| a.title)
                    .collect();
                if let Ok(json_str) = serde_json::to_string(&filtered) {
                    let conn = state.db.lock().await;
                    let _ = conn.execute(
                        "UPDATE trakt_watchlist_cache SET aliases_json = ?1 WHERE trakt_id = ?2",
                        params![json_str, trakt_id],
                    );
                }
            }
        }
    }

    let new_ver = rebuild_title_index(state).await;
    if let Some(handle) = app {
        let _ = handle.emit("trakt-watchlist-changed", new_ver);
    }

    Ok(())
}

pub async fn sync_history(state: &AppState) -> Result<(), String> {
    let resp = trakt_get_raw(state, None, "/users/me/history/?extended=full&page=1&limit=30", true)
        .await
        .map_err(|e| e.to_user_string())?;

    let entries: Vec<TraktHistoryEntry> = serde_json::from_str(&resp.body)
        .map_err(|e| format!("History JSON hiba: {}", e))?;

    {
        let mut conn = state.db.lock().await;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM trakt_history_cache", [])
            .map_err(|e| e.to_string())?;

        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO trakt_history_cache (
                        history_id, trakt_id, media_type, show_trakt_id, show_tmdb_id,
                        show_title, episode_title, season, episode, watched_at, poster_path
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                )
                .map_err(|e| e.to_string())?;

            for entry in entries {
                match entry.item_type.as_str() {
                    "episode" => {
                        let (Some(ep), Some(show)) = (entry.episode.as_ref(), entry.show.as_ref()) else {
                            continue;
                        };
                        let poster: Option<String> = show.ids.tmdb.and_then(|tid| {
                            tx.query_row(
                                "SELECT poster_path FROM trakt_watchlist_cache WHERE trakt_id = ?1
                                 UNION ALL
                                 SELECT poster_path FROM tmdb_cache WHERE tmdb_id = ?2 AND media_type = 'tv' AND poster_path IS NOT NULL
                                 LIMIT 1",
                                params![show.ids.trakt, tid],
                                |r| r.get(0),
                            )
                            .ok()
                            .flatten()
                        });
                        let _ = stmt.execute(params![
                            entry.id,
                            ep.ids.trakt,
                            "episode",
                            show.ids.trakt,
                            show.ids.tmdb,
                            show.title,
                            ep.title,
                            ep.season,
                            ep.number,
                            entry.watched_at,
                            poster,
                        ]);
                    }
                    "movie" => {
                        let Some(movie) = entry.movie.as_ref() else {
                            continue;
                        };
                        let poster: Option<String> = movie.ids.tmdb.and_then(|tid| {
                            tx.query_row(
                                "SELECT poster_path FROM trakt_watchlist_cache WHERE trakt_id = ?1
                                 UNION ALL
                                 SELECT poster_path FROM tmdb_cache WHERE tmdb_id = ?2 AND media_type = 'movie' AND poster_path IS NOT NULL
                                 LIMIT 1",
                                params![movie.ids.trakt, tid],
                                |r| r.get(0),
                            )
                            .ok()
                            .flatten()
                        });
                        let _ = stmt.execute(params![
                            entry.id,
                            movie.ids.trakt,
                            "movie",
                            Option::<i64>::None,
                            movie.ids.tmdb,
                            movie.title,
                            Option::<String>::None,
                            Option::<i64>::None,
                            Option::<i64>::None,
                            entry.watched_at,
                            poster,
                        ]);
                    }
                    _ => {}
                }
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
    }

    enrich_missing_history_posters(state).await;
    Ok(())
}

pub async fn enrich_missing_history_posters(state: &AppState) {
    if get_tmdb_api_key_from_state(state).await.is_none() {
        return;
    }

    let missing: Vec<(String, i64)> = {
        let conn = state.db.lock().await;
        let mut stmt = match conn.prepare(
            "SELECT DISTINCT CASE WHEN media_type = 'movie' THEN 'movie' ELSE 'tv' END AS tmdb_mt,
                             show_tmdb_id
             FROM trakt_history_cache
             WHERE show_tmdb_id IS NOT NULL
               AND (poster_path IS NULL OR poster_path = '')",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mapped = match stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            Ok(m) => m,
            Err(_) => return,
        };
        mapped.flatten().collect()
    };

    for (tmdb_mt, tmdb_id) in missing {
        if let Ok(details) = get_tmdb_by_id(state, tmdb_id, &tmdb_mt, "hu-HU", None, false).await {
            if let Some(ref poster) = details.poster_path {
                let conn = state.db.lock().await;
                let _ = conn.execute(
                    "UPDATE trakt_history_cache SET poster_path = ?1
                     WHERE show_tmdb_id = ?2 AND (poster_path IS NULL OR poster_path = '')",
                    params![poster, tmdb_id],
                );
            }
        }
    }
}

pub async fn sync_watched_shows(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
) -> Result<(), String> {
    let resp = trakt_get_raw(state, app, "/sync/watched/shows", true)
        .await
        .map_err(|e| e.to_user_string())?;

    let entries: Vec<TraktWatchedShowEntry> = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Watched shows JSON hiba: {}", e))?;

    let now = now_iso_utc();
    let mut conn = state.db.lock().await;

    // Preserve any locally queued offline history_add episode IDs
    let mut pending_ep_ids: HashSet<i64> = HashSet::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT payload_json FROM trakt_pending_actions WHERE action = 'history_add'",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for payload_str in rows.flatten() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&payload_str) {
                    if let Some(eps) = val.get("episodes").and_then(|v| v.as_array()) {
                        for ep in eps {
                            if let Some(id) = ep
                                .get("ids")
                                .and_then(|i| i.get("trakt"))
                                .and_then(|t| t.as_i64())
                            {
                                pending_ep_ids.insert(id);
                            }
                        }
                    }
                }
            }
        }
    }

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    if pending_ep_ids.is_empty() {
        tx.execute(
            "UPDATE trakt_episodes SET watched_at = NULL, updated_at = ?1 WHERE watched_at IS NOT NULL",
            params![now],
        )
        .map_err(|e| e.to_string())?;
    } else {
        let mut clear_stmt = tx
            .prepare(
                "SELECT show_trakt_id, season, episode, episode_trakt_id
                 FROM trakt_episodes WHERE watched_at IS NOT NULL",
            )
            .map_err(|e| e.to_string())?;
        let to_clear: Vec<(i64, i64, i64)> = clear_stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|(_, _, _, ep_id)| {
                ep_id.map_or(true, |id| !pending_ep_ids.contains(&id))
            })
            .map(|(s, se, ep, _)| (s, se, ep))
            .collect();
        drop(clear_stmt);

        let mut upd = tx
            .prepare(
                "UPDATE trakt_episodes SET watched_at = NULL, updated_at = ?1
                 WHERE show_trakt_id = ?2 AND season = ?3 AND episode = ?4",
            )
            .map_err(|e| e.to_string())?;
        for (sid, s_num, e_num) in to_clear {
            let _ = upd.execute(params![now, sid, s_num, e_num]);
        }
    }

    {
        let mut ep_stmt = tx
            .prepare(
                "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, title, watched_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5)
                 ON CONFLICT(show_trakt_id, season, episode) DO UPDATE SET
                    watched_at = excluded.watched_at,
                    updated_at = excluded.updated_at",
            )
            .map_err(|e| e.to_string())?;

        let mut show_stmt = tx
            .prepare(
                "INSERT INTO trakt_cache (
                    media_type, trakt_id, tmdb_id, imdb_id, slug, title, year, not_found, updated_at
                ) VALUES ('show', ?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
                ON CONFLICT(media_type, trakt_id) DO UPDATE SET
                    tmdb_id = COALESCE(excluded.tmdb_id, trakt_cache.tmdb_id),
                    imdb_id = COALESCE(excluded.imdb_id, trakt_cache.imdb_id),
                    slug = COALESCE(excluded.slug, trakt_cache.slug),
                    title = COALESCE(NULLIF(excluded.title, ''), trakt_cache.title),
                    year = COALESCE(excluded.year, trakt_cache.year),
                    not_found = 0",
            )
            .map_err(|e| e.to_string())?;

        let mut watched_show_ids: HashSet<i64> = HashSet::new();

        for entry in &entries {
            let show_id = entry.show.ids.trakt;
            let mut has_active_ep = false;
            for season in &entry.seasons {
                for ep in &season.episodes {
                    let after_reset = match (&entry.reset_at, &ep.last_watched_at) {
                        (Some(r), Some(w)) => w.as_str() >= r.as_str(),
                        _ => true,
                    };
                    if !after_reset {
                        continue;
                    }
                    has_active_ep = true;
                    let watched_at = ep.last_watched_at.as_deref().unwrap_or(&now);
                    let _ = ep_stmt.execute(params![
                        show_id,
                        season.number,
                        ep.number,
                        watched_at,
                        now,
                    ]);
                }
            }
            if has_active_ep {
                watched_show_ids.insert(show_id);
                if let Some(ref title) = entry.show.title {
                    let _ = show_stmt.execute(params![
                        show_id,
                        entry.show.ids.tmdb,
                        entry.show.ids.imdb,
                        entry.show.ids.slug,
                        title,
                        entry.show.year,
                        now,
                    ]);
                }
            }
        }

        // Clear next_episode for shows that no longer have any watched episodes
        let existing_prog_ids: Vec<i64> = {
            let mut p_stmt = tx
                .prepare("SELECT show_trakt_id FROM trakt_show_progress WHERE next_episode_trakt_id IS NOT NULL")
                .map_err(|e| e.to_string())?;
            let mapped = p_stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?;
            mapped.flatten().collect()
        };
        for pid in existing_prog_ids {
            if !watched_show_ids.contains(&pid) {
                let _ = tx.execute(
                    "UPDATE trakt_show_progress
                     SET completed = 0, next_episode_trakt_id = NULL, next_season = NULL, next_episode = NULL, next_episode_title = NULL, next_episode_first_aired = NULL, updated_at = ?1
                     WHERE show_trakt_id = ?2",
                    params![now, pid],
                );
            }
        }
    }

    tx.execute(
        "DELETE FROM trakt_http_cache WHERE key LIKE '/shows/%/progress/watched%' OR key LIKE '/users/%/stats'",
        [],
    )
    .ok();

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

async fn fetch_hidden_progress_show_ids(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
) -> HashSet<i64> {
    let mut hidden = HashSet::new();
    let mut page = 1u32;
    loop {
        let path = format!(
            "/users/hidden/progress_watched?type=show&page={}&limit=100",
            page
        );
        let Ok(resp) = trakt_get_cached(state, app, &path, true, 3600, false).await else {
            break;
        };
        let Ok(entries) = serde_json::from_str::<Vec<TraktHiddenEntry>>(&resp.body) else {
            break;
        };
        if entries.is_empty() {
            break;
        }
        for entry in entries {
            if let Some(show) = entry.show {
                hidden.insert(show.ids.trakt);
            }
        }
        let total_pages = resp.page_count.unwrap_or(1);
        if page >= total_pages {
            break;
        }
        page += 1;
    }
    hidden
}

async fn enrich_missing_continue_watching_tmdb(state: &AppState) {
    if get_tmdb_api_key_from_state(state).await.is_none() {
        return;
    }

    let missing_tmdb_ids: Vec<i64> = {
        let conn = state.db.lock().await;
        let mut stmt = match conn.prepare(
            "SELECT DISTINCT tc.tmdb_id
             FROM trakt_show_progress p
             JOIN trakt_cache tc ON tc.media_type = 'show' AND tc.trakt_id = p.show_trakt_id
             WHERE p.hidden_from_progress = 0
               AND p.completed > 0
               AND p.next_episode_trakt_id IS NOT NULL
               AND tc.tmdb_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM tmdb_cache tm
                   WHERE tm.tmdb_id = tc.tmdb_id
                     AND tm.media_type = 'tv'
                     AND tm.poster_path IS NOT NULL
                     AND tm.poster_path != ''
               )
             LIMIT 20",
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        let mapped = match stmt.query_map([], |r| r.get::<_, i64>(0)) {
            Ok(m) => m,
            Err(_) => return,
        };
        mapped.flatten().collect()
    };

    for tmdb_id in missing_tmdb_ids {
        let _ = get_tmdb_by_id(state, tmdb_id, "tv", "hu-HU", None, false).await;
    }
}

pub async fn sync_show_progress(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    show_trakt_id: i64,
    force_refresh: bool,
) -> Result<TraktProgressResponse, String> {
    let path = format!(
        "/shows/{}/progress/watched?hidden=false&specials=false&count_specials=false&extended=full",
        show_trakt_id
    );
    let resp = trakt_get_cached(state, app, &path, true, 600, force_refresh)
        .await
        .map_err(|e| e.to_user_string())?;

    let progress: TraktProgressResponse = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Progress JSON hiba: {}", e))?;

    let mut next_first_aired: Option<String> = None;
    if progress.completed.unwrap_or(0) > 0 {
        if let Some(ref ne) = progress.next_episode {
            next_first_aired = ne.first_aired.clone();
            if next_first_aired.is_none() {
                let cal_fa: Option<String> = {
                    let conn = state.db.lock().await;
                    conn.query_row(
                        "SELECT first_aired FROM trakt_calendar_cache WHERE episode_trakt_id = ?1 LIMIT 1",
                        params![ne.ids.trakt],
                        |r| r.get(0),
                    )
                    .ok()
                };
                next_first_aired = cal_fa;
            }
            if next_first_aired.is_none() {
                let ep_path = format!(
                    "/shows/{}/seasons/{}/episodes/{}?extended=full",
                    show_trakt_id, ne.season, ne.number
                );
                if let Ok(ep_resp) =
                    trakt_get_cached(state, app, &ep_path, false, 86400, false).await
                {
                    if let Ok(ep_ext) = serde_json::from_str::<TraktEpisodeExtended>(&ep_resp.body) {
                        next_first_aired = ep_ext.first_aired;
                    }
                }
            }
        }
    }

    let now = now_iso_utc();
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let (next_ep_id, next_season, next_ep_num, next_ep_title, next_ep_aired) =
        if progress.completed.unwrap_or(0) > 0 {
            match progress.next_episode.as_ref() {
                Some(ne) => (
                    Some(ne.ids.trakt),
                    Some(ne.season),
                    Some(ne.number),
                    ne.title.clone(),
                    next_first_aired,
                ),
                None => (None, None, None, None, None),
            }
        } else {
            (None, None, None, None, None)
        };

    tx.execute(
        "INSERT INTO trakt_show_progress (
            show_trakt_id, aired, completed, last_watched_at,
            next_episode_trakt_id, next_season, next_episode, next_episode_title, next_episode_first_aired, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(show_trakt_id) DO UPDATE SET
            aired = excluded.aired,
            completed = excluded.completed,
            last_watched_at = excluded.last_watched_at,
            next_episode_trakt_id = excluded.next_episode_trakt_id,
            next_season = excluded.next_season,
            next_episode = excluded.next_episode,
            next_episode_title = excluded.next_episode_title,
            next_episode_first_aired = excluded.next_episode_first_aired,
            updated_at = excluded.updated_at",
        params![
            show_trakt_id,
            progress.aired,
            progress.completed,
            progress.last_watched_at,
            next_ep_id,
            next_season,
            next_ep_num,
            next_ep_title,
            next_ep_aired,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;

    {
        let mut ep_stmt = tx
            .prepare(
                "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, title, watched_at, updated_at)
                 VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5)
                 ON CONFLICT(show_trakt_id, season, episode) DO UPDATE SET
                    watched_at = excluded.watched_at,
                    updated_at = excluded.updated_at",
            )
            .map_err(|e| e.to_string())?;

        for season in &progress.seasons {
            for ep in &season.episodes {
                let watched_at = if ep.completed {
                    Some(
                        ep.last_watched_at
                            .clone()
                            .unwrap_or_else(|| now.clone()),
                    )
                } else {
                    None
                };
                let _ = ep_stmt.execute(params![
                    show_trakt_id,
                    season.number,
                    ep.number,
                    watched_at,
                    now,
                ]);
            }
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(progress)
}

pub fn is_sync_overdue(last_sync_ok_at: Option<&str>) -> bool {
    let Some(ts) = last_sync_ok_at else {
        return true;
    };
    let Some(epoch) = iso_utc_to_epoch(ts) else {
        return true;
    };
    now_epoch_secs() - epoch > 86400
}
