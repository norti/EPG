use crate::trakt::client::{prune_stale_calendar_keys, trakt_get_cached};
use crate::trakt::now_iso_utc;
use crate::AppState;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEpisodeItem {
    pub episode_trakt_id: i64,
    pub show_trakt_id: i64,
    pub show_tmdb_id: Option<i64>,
    pub show_title: String,
    pub show_title_hu: Option<String>,
    pub season: i64,
    pub number: i64,
    pub episode_title: Option<String>,
    pub first_aired: String,
    pub network: Option<String>,
    pub poster_path: Option<String>,
    pub overview: Option<String>,
    pub watched: bool,
    pub on_watchlist: bool,
}

#[derive(Deserialize, Debug)]
struct TraktCalendarEntry {
    first_aired: String,
    episode: TraktCalendarEpisode,
    show: TraktCalendarShow,
}

#[derive(Deserialize, Debug)]
struct TraktCalendarEpisode {
    season: i64,
    number: i64,
    title: Option<String>,
    ids: TraktIds,
}

#[derive(Deserialize, Debug)]
struct TraktCalendarShow {
    title: String,
    network: Option<String>,
    overview: Option<String>,
    ids: TraktIds,
}

#[derive(Deserialize, Debug)]
struct TraktIds {
    trakt: i64,
    tmdb: Option<i64>,
}

pub async fn fetch_and_cache_my_calendar(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    start_date: &str,
    force_refresh: bool,
) -> Result<(), String> {
    let path = format!("/calendars/my/shows/{}/31?extended=full", start_date);
    let resp = trakt_get_cached(state, app, &path, true, 7200, force_refresh)
        .await
        .map_err(|e| e.to_user_string())?;

    if !resp.from_cache {
        // A new date-keyed body was just stored: the previous days' bodies are dead weight.
        let conn = state.db.lock().await;
        prune_stale_calendar_keys(&conn, "/calendars/my/shows/", &path);
    }

    let has_rows: bool = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM trakt_calendar_cache WHERE scope = 'my')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false)
    };

    if resp.from_cache && has_rows && !force_refresh {
        enrich_missing_my_calendar_posters(state).await;
        return Ok(());
    }

    let entries: Vec<TraktCalendarEntry> = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Naptár JSON hiba: {}", e))?;

    let now = now_iso_utc();
    {
        let mut conn = state.db.lock().await;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM trakt_calendar_cache WHERE scope = 'my'", [])
            .map_err(|e| e.to_string())?;

        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR REPLACE INTO trakt_calendar_cache (
                        scope, episode_trakt_id, show_trakt_id, show_tmdb_id, show_title,
                        season, number, episode_title, first_aired, network, fetched_at
                    ) VALUES ('my', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|e| e.to_string())?;

            let mut show_stmt = tx
                .prepare(
                    "INSERT INTO trakt_cache (
                        media_type, trakt_id, tmdb_id, title, overview, network, not_found, updated_at
                    ) VALUES ('show', ?1, ?2, ?3, ?4, ?5, 0, ?6)
                    ON CONFLICT(media_type, trakt_id) DO UPDATE SET
                        tmdb_id = COALESCE(excluded.tmdb_id, trakt_cache.tmdb_id),
                        overview = COALESCE(excluded.overview, trakt_cache.overview),
                        network = COALESCE(excluded.network, trakt_cache.network)",
                )
                .map_err(|e| e.to_string())?;

            for entry in entries {
                stmt.execute(params![
                    entry.episode.ids.trakt,
                    entry.show.ids.trakt,
                    entry.show.ids.tmdb,
                    entry.show.title,
                    entry.episode.season,
                    entry.episode.number,
                    entry.episode.title,
                    entry.first_aired,
                    entry.show.network,
                    now,
                ])
                .map_err(|e| e.to_string())?;

                let _ = show_stmt.execute(params![
                    entry.show.ids.trakt,
                    entry.show.ids.tmdb,
                    entry.show.title,
                    entry.show.overview,
                    entry.show.network,
                    now,
                ]);
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
    }

    enrich_missing_my_calendar_posters(state).await;
    Ok(())
}

pub async fn enrich_missing_my_calendar_posters(state: &AppState) {
    if crate::get_tmdb_api_key_from_state(state).await.is_none() {
        return;
    }

    let missing_tmdb_ids: Vec<i64> = {
        let conn = state.db.lock().await;
        let mut stmt = match conn.prepare(
            "SELECT DISTINCT c.show_tmdb_id
             FROM trakt_calendar_cache c
             WHERE c.scope = 'my'
               AND c.show_tmdb_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM tmdb_cache tm
                   WHERE tm.tmdb_id = c.show_tmdb_id
                     AND tm.media_type = 'tv'
                     AND tm.poster_path IS NOT NULL
                     AND tm.poster_path != ''
               )",
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
        let _ = crate::get_tmdb_by_id(state, tmdb_id, "tv", "hu-HU", None, false).await;
    }
}

pub async fn fetch_and_cache_global_calendar(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
    start_date: &str,
    force_refresh: bool,
) -> Result<(), String> {
    let path = format!("/calendars/all/shows/{}/14?extended=full", start_date);
    let resp = trakt_get_cached(state, app, &path, false, 10800, force_refresh)
        .await
        .map_err(|e| e.to_user_string())?;

    if !resp.from_cache {
        let conn = state.db.lock().await;
        prune_stale_calendar_keys(&conn, "/calendars/all/shows/", &path);
    }

    let has_rows: bool = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM trakt_calendar_cache WHERE scope = 'all')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false)
    };

    if resp.from_cache && has_rows && !force_refresh {
        return Ok(());
    }

    let entries: Vec<TraktCalendarEntry> = serde_json::from_str(&resp.body)
        .map_err(|e| format!("Globális naptár JSON hiba: {}", e))?;

    let now = now_iso_utc();
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM trakt_calendar_cache WHERE scope = 'all'", [])
        .map_err(|e| e.to_string())?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO trakt_calendar_cache (
                    scope, episode_trakt_id, show_trakt_id, show_tmdb_id, show_title,
                    season, number, episode_title, first_aired, network, fetched_at
                ) VALUES ('all', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(|e| e.to_string())?;

        let mut show_stmt = tx
            .prepare(
                "INSERT INTO trakt_cache (
                    media_type, trakt_id, tmdb_id, title, overview, network, not_found, updated_at
                ) VALUES ('show', ?1, ?2, ?3, ?4, ?5, 0, ?6)
                ON CONFLICT(media_type, trakt_id) DO UPDATE SET
                    tmdb_id = COALESCE(excluded.tmdb_id, trakt_cache.tmdb_id),
                    overview = COALESCE(excluded.overview, trakt_cache.overview),
                    network = COALESCE(excluded.network, trakt_cache.network)",
            )
            .map_err(|e| e.to_string())?;

        for entry in entries {
            let _ = stmt.execute(params![
                entry.episode.ids.trakt,
                entry.show.ids.trakt,
                entry.show.ids.tmdb,
                entry.show.title,
                entry.episode.season,
                entry.episode.number,
                entry.episode.title,
                entry.first_aired,
                entry.show.network,
                now,
            ]);

            let _ = show_stmt.execute(params![
                entry.show.ids.trakt,
                entry.show.ids.tmdb,
                entry.show.title,
                entry.show.overview,
                entry.show.network,
                now,
            ]);
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub fn query_global_calendar_merged(conn: &Connection) -> Result<Vec<CalendarEpisodeItem>, String> {
    let sql = "SELECT
            c.episode_trakt_id,
            c.show_trakt_id,
            COALESCE(c.show_tmdb_id, w.tmdb_id) AS tmdb_id,
            COALESCE(c.show_title, w.title, '') AS show_title,
            COALESCE(w.title_hu, (SELECT tm.hu_title FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(c.show_tmdb_id, w.tmdb_id) AND tm.media_type = 'tv' AND tm.hu_title IS NOT NULL LIMIT 1)) AS show_title_hu,
            c.season,
            c.number,
            c.episode_title,
            c.first_aired,
            COALESCE(c.network, tc.network) AS network,
            COALESCE(w.poster_path, (SELECT tm.poster_path FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(c.show_tmdb_id, w.tmdb_id) AND tm.media_type = 'tv' AND tm.poster_path IS NOT NULL LIMIT 1)) AS poster_path,
            COALESCE(tc.overview, (SELECT tm.overview FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(c.show_tmdb_id, w.tmdb_id) AND tm.media_type = 'tv' AND tm.overview IS NOT NULL LIMIT 1)) AS overview,
            CASE WHEN e.watched_at IS NOT NULL THEN 1 ELSE 0 END AS watched,
            CASE WHEN w.trakt_id IS NOT NULL OR (
                c.show_tmdb_id IS NOT NULL AND EXISTS (
                    SELECT 1 FROM trakt_watchlist_cache w2
                    WHERE w2.media_type = 'show' AND w2.tmdb_id = c.show_tmdb_id
                )
            ) THEN 1 ELSE 0 END AS on_watchlist
         FROM trakt_calendar_cache c
         LEFT JOIN trakt_watchlist_cache w ON w.trakt_id = c.show_trakt_id
         LEFT JOIN trakt_episodes e ON e.show_trakt_id = c.show_trakt_id AND e.season = c.season AND e.episode = c.number
         LEFT JOIN trakt_cache tc ON tc.media_type = 'show' AND tc.trakt_id = c.show_trakt_id
         WHERE c.scope = 'all'
            OR (c.scope = 'my' AND NOT EXISTS (
                SELECT 1 FROM trakt_calendar_cache c2
                WHERE c2.scope = 'all' AND c2.episode_trakt_id = c.episode_trakt_id
            ))
         ORDER BY c.first_aired ASC";

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let watched_int: i32 = r.get(12)?;
            let wl_int: i32 = r.get(13)?;
            Ok(CalendarEpisodeItem {
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
                watched: watched_int == 1,
                on_watchlist: wl_int == 1,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    for row in rows {
        if let Ok(item) = row {
            items.push(item);
        }
    }
    Ok(items)
}

pub fn query_calendar_episodes(
    conn: &Connection,
    scope: &str,
    filter_watchlist: bool,
) -> Result<Vec<CalendarEpisodeItem>, String> {
    let where_wl = if filter_watchlist {
        "AND w.trakt_id IS NOT NULL"
    } else {
        ""
    };

    let sql = format!(
        "SELECT
            c.episode_trakt_id,
            c.show_trakt_id,
            COALESCE(c.show_tmdb_id, w.tmdb_id) AS tmdb_id,
            COALESCE(c.show_title, w.title, '') AS show_title,
            COALESCE(w.title_hu, (SELECT tm.hu_title FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(c.show_tmdb_id, w.tmdb_id) AND tm.media_type = 'tv' AND tm.hu_title IS NOT NULL LIMIT 1)) AS show_title_hu,
            c.season,
            c.number,
            c.episode_title,
            c.first_aired,
            COALESCE(c.network, tc.network) AS network,
            COALESCE(w.poster_path, (SELECT tm.poster_path FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(c.show_tmdb_id, w.tmdb_id) AND tm.media_type = 'tv' AND tm.poster_path IS NOT NULL LIMIT 1)) AS poster_path,
            COALESCE(tc.overview, (SELECT tm.overview FROM tmdb_cache tm WHERE tm.tmdb_id = COALESCE(c.show_tmdb_id, w.tmdb_id) AND tm.media_type = 'tv' AND tm.overview IS NOT NULL LIMIT 1)) AS overview,
            CASE WHEN e.watched_at IS NOT NULL THEN 1 ELSE 0 END AS watched,
            CASE WHEN w.trakt_id IS NOT NULL THEN 1 ELSE 0 END AS on_watchlist
         FROM trakt_calendar_cache c
         LEFT JOIN trakt_watchlist_cache w ON w.trakt_id = c.show_trakt_id
         LEFT JOIN trakt_episodes e ON e.show_trakt_id = c.show_trakt_id AND e.season = c.season AND e.episode = c.number
         LEFT JOIN trakt_cache tc ON tc.media_type = 'show' AND tc.trakt_id = c.show_trakt_id
         WHERE c.scope = ?1 {}
         ORDER BY c.first_aired ASC",
        where_wl
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![scope], |r| {
            let watched_int: i32 = r.get(12)?;
            let wl_int: i32 = r.get(13)?;
            Ok(CalendarEpisodeItem {
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
                watched: watched_int == 1,
                on_watchlist: wl_int == 1,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    for row in rows {
        if let Ok(item) = row {
            items.push(item);
        }
    }
    Ok(items)
}
