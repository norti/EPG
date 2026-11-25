use crate::{clean_title, extract_year_hint, normalise_for_match, AppState};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

#[derive(Debug, Clone)]
pub struct WatchlistRef {
    pub trakt_id: i64,
    pub tmdb_id: Option<i64>,
    pub media_type: String,
    pub year: Option<i64>,
}

#[derive(Debug, Default)]
pub struct TitleIndex {
    index: HashMap<String, Vec<WatchlistRef>>,
}

impl TitleIndex {
    pub fn build_from_db(conn: &Connection) -> rusqlite::Result<Self> {
        let mut index: HashMap<String, Vec<WatchlistRef>> = HashMap::new();

        let mut stmt = conn.prepare(
            "SELECT trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id
             FROM trakt_watchlist_cache",
        )?;

        type WatchlistRow = (
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        );
        let rows: Vec<WatchlistRow> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .flatten()
            .collect();

        let mut tmdb_stmt = conn.prepare(
            "SELECT title, hu_title, en_title, original_title
             FROM tmdb_cache
             WHERE tmdb_id = ?1 AND media_type != 'none'",
        )?;

        for (trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id) in rows {
            let wl_ref = WatchlistRef {
                trakt_id,
                tmdb_id,
                media_type,
                year,
            };

            let mut variants: Vec<String> = Vec::new();
            variants.push(title);
            if let Some(hu) = title_hu {
                variants.push(hu);
            }
            if let Some(aj) = aliases_json {
                if let Ok(aliases) = serde_json::from_str::<Vec<String>>(&aj) {
                    variants.extend(aliases);
                }
            }

            if let Some(tid) = tmdb_id {
                if let Ok(mapped) = tmdb_stmt.query_map(params![tid], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                }) {
                    for (t, hu, en, orig) in mapped.flatten() {
                        if let Some(s) = t {
                            variants.push(s);
                        }
                        if let Some(s) = hu {
                            variants.push(s);
                        }
                        if let Some(s) = en {
                            variants.push(s);
                        }
                        if let Some(s) = orig {
                            variants.push(s);
                        }
                    }
                }
            }

            let mut seen_norms = HashSet::new();
            for v in variants {
                let cleaned = clean_title(&v);
                let norm = normalise_for_match(&cleaned);
                if !norm.is_empty() && seen_norms.insert(norm.clone()) {
                    let entry = index.entry(norm).or_default();
                    if !entry
                        .iter()
                        .any(|r| r.trakt_id == wl_ref.trakt_id && r.media_type == wl_ref.media_type)
                    {
                        entry.push(wl_ref.clone());
                    }
                }
                let raw_norm = normalise_for_match(&v);
                if !raw_norm.is_empty() && seen_norms.insert(raw_norm.clone()) {
                    let entry = index.entry(raw_norm).or_default();
                    if !entry
                        .iter()
                        .any(|r| r.trakt_id == wl_ref.trakt_id && r.media_type == wl_ref.media_type)
                    {
                        entry.push(wl_ref.clone());
                    }
                }
            }
        }

        Ok(Self { index })
    }

    pub fn lookup(&self, norm_title: &str) -> Option<&Vec<WatchlistRef>> {
        self.index.get(norm_title)
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

pub async fn rebuild_title_index(state: &AppState) -> u64 {
    let new_index = {
        let conn = state.db.lock().await;
        TitleIndex::build_from_db(&conn).unwrap_or_default()
    };
    {
        let mut idx_guard = state.trakt.title_index.write().await;
        *idx_guard = new_index;
    }
    let new_ver = state.trakt.watchlist_version.fetch_add(1, Ordering::SeqCst) + 1;
    {
        let conn = state.db.lock().await;
        let _ = conn.execute(
            "INSERT INTO trakt_sync_state (key, value) VALUES ('watchlist_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![new_ver.to_string()],
        );
    }
    {
        let mut mc = state.trakt.match_cache.write().await;
        mc.0 = new_ver;
        mc.1.clear();
    }
    new_ver
}

pub async fn invalidate_match_cache(state: &AppState) {
    let new_ver = state.trakt.watchlist_version.fetch_add(1, Ordering::SeqCst) + 1;
    let mut mc = state.trakt.match_cache.write().await;
    mc.0 = new_ver;
    mc.1.clear();
}

pub fn check_title_against_index(
    conn: &Connection,
    index: &TitleIndex,
    raw_title: &str,
) -> bool {
    if index.is_empty() {
        return false;
    }
    let year_hint = extract_year_hint(raw_title);
    let clean = clean_title(raw_title);
    let norm = normalise_for_match(&clean);
    if norm.is_empty() {
        return false;
    }

    let Some(refs) = index.lookup(&norm) else {
        return false;
    };

    // Check tmdb_cache for manual Fix overrides or non-expired 'none' negative markers
    let tmdb_row: Option<(Option<i64>, Option<String>)> = conn
        .query_row(
            "SELECT tmdb_id, media_type FROM tmdb_cache
             WHERE query_title = ?1
               AND NOT (media_type = 'none' AND is_manual = 0 AND updated_at < datetime('now', 'localtime', '-7 days'))
             ORDER BY is_manual DESC, CASE WHEN lang = 'hu-HU' THEN 0 ELSE 1 END, updated_at DESC
             LIMIT 1",
            params![clean],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();

    if let Some((tmdb_id_opt, media_type_opt)) = tmdb_row {
        if media_type_opt.as_deref() == Some("none") || tmdb_id_opt.is_none() {
            return false;
        }
        if let Some(cache_tmdb_id) = tmdb_id_opt {
            return refs.iter().any(|r| r.tmdb_id == Some(cache_tmdb_id));
        }
    }

    // No tmdb_cache row: accept unless year hint clearly conflicts with watchlist item year
    refs.iter().any(|r| match (year_hint, r.year) {
        (Some(yh), Some(wy)) => (yh as i64 - wy).abs() <= 1,
        _ => true,
    })
}

pub async fn match_titles(state: &AppState, raw_titles: Vec<String>) -> Result<Vec<String>, String> {
    let current_ver = state.trakt.watchlist_version.load(Ordering::SeqCst);

    let mut uncached: Vec<String> = Vec::new();
    let mut matched: Vec<String> = Vec::new();

    {
        let mc = state.trakt.match_cache.read().await;
        if mc.0 == current_ver {
            for t in &raw_titles {
                match mc.1.get(t) {
                    Some(true) => matched.push(t.clone()),
                    Some(false) => {}
                    None => uncached.push(t.clone()),
                }
            }
        } else {
            uncached = raw_titles.clone();
        }
    }

    if uncached.is_empty() {
        return Ok(matched);
    }

    let mut computed: Vec<(String, bool)> = Vec::with_capacity(uncached.len());
    {
        let idx = state.trakt.title_index.read().await;
        let conn = state.db.lock().await;
        if !crate::trakt::is_trakt_enabled(&conn) {
            return Ok(Vec::new());
        }
        for t in uncached {
            let is_match = check_title_against_index(&conn, &idx, &t);
            if is_match {
                matched.push(t.clone());
            }
            computed.push((t, is_match));
        }
    }

    {
        let mut mc = state.trakt.match_cache.write().await;
        if mc.0 != current_ver {
            mc.0 = current_ver;
            mc.1.clear();
        }
        for (t, is_match) in computed {
            mc.1.insert(t, is_match);
        }
    }

    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run_migrations(&conn, None).unwrap();
        conn
    }

    #[test]
    fn test_matcher_cleans_epg_titles_and_matches_variants() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO trakt_watchlist_cache (trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id, poster_path, listed_at, updated_at)
             VALUES (104439, 'show', 'Stranger Things', 'Stranger Things', '[\"Különös dolgok\"]', 2016, 66732, '/poster.jpg', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_watchlist_cache (trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id, poster_path, listed_at, updated_at)
             VALUES (999, 'show', 'A mi kis falunk', 'A mi kis falunk', NULL, 2017, 70000, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        ).unwrap();

        let idx = TitleIndex::build_from_db(&conn).unwrap();
        assert!(check_title_against_index(&conn, &idx, "Stranger Things S03E04"));
        assert!(check_title_against_index(&conn, &idx, "Különös dolgok (16) (HD)"));
        assert!(check_title_against_index(&conn, &idx, "A mi kis falunk 6. évad 1. rész"));
        assert!(!check_title_against_index(&conn, &idx, "Híradó"));
    }

    #[test]
    fn test_matcher_respects_tmdb_cache_fix_and_none() {
        let conn = setup_test_db();
        conn.execute(
            "INSERT INTO trakt_watchlist_cache (trakt_id, media_type, title, title_hu, aliases_json, year, tmdb_id, poster_path, listed_at, updated_at)
             VALUES (100, 'movie', 'Dune', 'Dűne', NULL, 2021, 438631, NULL, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        ).unwrap();

        let idx = TitleIndex::build_from_db(&conn).unwrap();

        // Year hint mismatch (1984 vs 2021) without tmdb_cache should reject
        assert!(!check_title_against_index(&conn, &idx, "Dűne (1984)"));
        assert!(check_title_against_index(&conn, &idx, "Dűne (2021)"));

        // Expired automatic negative ('none', is_manual=0) is ignored -> falls back to title match
        conn.execute(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at) VALUES ('Dűne', 'hu-HU', 'none', 0, '2020-01-01 00:00:00')",
            [],
        ).unwrap();
        assert!(check_title_against_index(&conn, &idx, "Dűne"));

        // User cleared match via Fix ('none', is_manual=1) -> must NOT match regardless of age
        conn.execute(
            "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at) VALUES ('Dűne', 'hu-HU', 'none', 1, '2020-01-01 00:00:00')",
            [],
        ).unwrap();
        assert!(!check_title_against_index(&conn, &idx, "Dűne"));

        // User fixed match to a different tmdb_id -> must NOT match
        conn.execute(
            "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, tmdb_id, media_type, is_manual, updated_at) VALUES ('Dűne', 'hu-HU', 841, 'movie', 1, '2026-01-01')",
            [],
        ).unwrap();
        assert!(!check_title_against_index(&conn, &idx, "Dűne"));

        // User fixed match to the matching tmdb_id -> strong match
        conn.execute(
            "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, tmdb_id, media_type, is_manual, updated_at) VALUES ('Dűne', 'hu-HU', 438631, 'movie', 1, '2026-01-01')",
            [],
        ).unwrap();
        assert!(check_title_against_index(&conn, &idx, "Dűne"));

        // Manual en-US fix takes precedence over automatic hu-HU wrong match
        conn.execute("DELETE FROM tmdb_cache WHERE query_title = 'Dűne'", []).unwrap();
        conn.execute(
            "INSERT INTO tmdb_cache (query_title, lang, tmdb_id, media_type, is_manual, updated_at) VALUES
             ('Dűne', 'hu-HU', 841, 'movie', 0, '2026-11-25 10:00:00'),
             ('Dűne', 'en-US', 438631, 'movie', 1, '2026-11-25 09:00:00')",
            [],
        ).unwrap();
        assert!(check_title_against_index(&conn, &idx, "Dűne"));
    }
}
