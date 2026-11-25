use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub type MigFn = fn(&Connection) -> rusqlite::Result<()>;

pub const EMBEDDED_ENGLISH_LNG: &str = include_str!("../../public/languages/english.lng");
pub const EMBEDDED_HUNGARIAN_LNG: &str = include_str!("../../public/languages/hungarian.lng");

pub const MIGRATIONS: &[MigFn] = &[
    migrate_v1,
    migrate_v2,
    migrate_v3,
    migrate_v4,
    migrate_v5,
    migrate_v6,
    migrate_v7,
    migrate_v8,
];

fn column_exists(conn: &Connection, table: &str, col: &str) -> rusqlite::Result<bool> {
    let sql = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name.eq_ignore_ascii_case(col) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    col: &str,
    col_def: &str,
) -> rusqlite::Result<()> {
    if !column_exists(conn, table, col)? {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, col, col_def);
        conn.execute(&sql, [])?;
    }
    Ok(())
}

/// Copies the DB file next to itself with `suffix` appended (e.g. `.bak-v4` = backup of a v4 DB).
fn backup_db_file(db_path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let mut bak_name = db_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "epg.db".to_string());
    bak_name.push_str(suffix);
    let bak_path = db_path.with_file_name(bak_name);
    std::fs::copy(db_path, &bak_path)
        .map_err(|e| format!("Adatbázis biztonsági mentési hiba ({}): {}", bak_path.display(), e))?;
    Ok(bak_path)
}

pub fn run_migrations(conn: &Connection, db_path: Option<&Path>) -> Result<(), String> {
    let current_ver: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);

    for (i, migration) in MIGRATIONS.iter().enumerate() {
        let ver = (i + 1) as i64;
        if ver <= current_ver {
            continue;
        }

        let mut backup_path: Option<PathBuf> = None;
        // v2 rewrites the programmes table; v5 backfills tmdb_cache.is_manual. Both get a pre-migration backup.
        let backup_suffix = match ver {
            2 => Some(".bak-v1"),
            5 => Some(".bak-v4"),
            _ => None,
        };
        if let Some(suffix) = backup_suffix {
            if let Some(path) = db_path {
                if path.exists() {
                    backup_path = Some(backup_db_file(path, suffix)?);
                }
            }
        }

        conn.execute_batch("BEGIN EXCLUSIVE")
            .map_err(|e| format!("Migráció v{} tranzakció indítási hiba: {}", ver, e))?;

        if let Err(e) = migration(conn) {
            let _ = conn.execute_batch("ROLLBACK");
            let bak_hint = backup_path
                .as_ref()
                .map(|p| format!(" (biztonsági mentés: {})", p.display()))
                .unwrap_or_default();
            return Err(format!("Adatbázis migráció v{} sikertelen{}: {}", ver, bak_hint, e));
        }

        if let Err(e) = conn.execute_batch(&format!("PRAGMA user_version = {}; COMMIT;", ver)) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(format!("Migráció v{} lezárási hiba: {}", ver, e));
        }
    }

    sync_embedded_languages(conn)
        .map_err(|e| format!("Nyelvi fájlok szinkronizálási hiba: {}", e))?;

    Ok(())
}

/// Computes a whitespace- and key-order-independent SHA-256 hex digest for a JSON language file.
pub fn canonical_json_sha256(content: &str) -> String {
    use sha2::{Digest, Sha256};

    let canonical = serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| serde_json::to_string(&v).ok());
    let bytes = canonical.as_deref().unwrap_or(content).as_bytes();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Keeps `app_languages` in sync with an embedded `.lng` file using its SHA-256 hash stored in
/// `settings` (`lng_hash_<lang>`):
/// - If the DB row is missing/empty or still matches the previously synced embedded hash, it is
///   replaced with the new embedded content.
/// - If the user customized translations (or upgraded from a legacy DB without a stored hash),
///   only missing keys from the new embedded file are merged in without overwriting existing values.
pub fn sync_embedded_language(
    conn: &Connection,
    lang: &str,
    embedded_content: &str,
) -> rusqlite::Result<()> {
    let hash_key = format!("lng_hash_{}", lang);
    let embedded_hash = canonical_json_sha256(embedded_content);

    let stored_hash: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![hash_key],
            |r| r.get(0),
        )
        .ok();

    let existing_content: Option<String> = conn
        .query_row(
            "SELECT content FROM app_languages WHERE lang = ?1",
            params![lang],
            |r| r.get(0),
        )
        .ok();

    let has_non_empty_row = existing_content
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());

    if stored_hash.as_deref() == Some(embedded_hash.as_str()) && has_non_empty_row {
        return Ok(());
    }

    let updated_content: Option<String> = match existing_content {
        Some(ref existing) if !existing.trim().is_empty() => {
            let existing_hash = canonical_json_sha256(existing);
            if existing_hash == embedded_hash {
                None
            } else if stored_hash.as_deref() == Some(existing_hash.as_str()) {
                Some(embedded_content.to_string())
            } else {
                match (
                    serde_json::from_str::<serde_json::Value>(existing),
                    serde_json::from_str::<serde_json::Value>(embedded_content),
                ) {
                    (
                        Ok(serde_json::Value::Object(mut existing_map)),
                        Ok(serde_json::Value::Object(embedded_map)),
                    ) => {
                        let mut changed = false;
                        for (k, v) in embedded_map {
                            if !existing_map.contains_key(&k) {
                                existing_map.insert(k, v);
                                changed = true;
                            }
                        }
                        if changed {
                            serde_json::to_string_pretty(&serde_json::Value::Object(existing_map))
                                .ok()
                        } else {
                            None
                        }
                    }
                    _ => Some(embedded_content.to_string()),
                }
            }
        }
        _ => Some(embedded_content.to_string()),
    };

    if let Some(content) = updated_content {
        conn.execute(
            "INSERT INTO app_languages (lang, content, updated_at) VALUES (?1, ?2, datetime('now', 'localtime'))
             ON CONFLICT(lang) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
            params![lang, content],
        )?;
    }

    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![hash_key, embedded_hash],
    )?;

    Ok(())
}

pub fn sync_embedded_languages(conn: &Connection) -> rusqlite::Result<()> {
    sync_embedded_language(conn, "english", EMBEDDED_ENGLISH_LNG)?;
    sync_embedded_language(conn, "hungarian", EMBEDDED_HUNGARIAN_LNG)?;
    Ok(())
}

fn migrate_v1(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS channels (
            id TEXT, source_id TEXT, name TEXT, icon TEXT, PRIMARY KEY (id, source_id)
        );
        CREATE TABLE IF NOT EXISTS programmes (
            id INTEGER PRIMARY KEY AUTOINCREMENT, channel_id TEXT, source_id TEXT, start TEXT, stop TEXT, title TEXT, desc TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_prog_time ON programmes (source_id, start, stop);
        CREATE INDEX IF NOT EXISTS idx_prog_src_ch ON programmes (source_id, channel_id);

        CREATE TABLE IF NOT EXISTS sources (
            id TEXT PRIMARY KEY,
            name TEXT,
            url TEXT,
            last_downloaded TEXT,
            country_code TEXT,
            time_offset INTEGER,
            sort_position INTEGER
        );

        CREATE TABLE IF NOT EXISTS favorites (
            title TEXT PRIMARY KEY
        );

        CREATE TABLE IF NOT EXISTS channel_orders (
            source_id TEXT,
            channel_id TEXT,
            position INTEGER,
            PRIMARY KEY (source_id, channel_id)
        );

        CREATE TABLE IF NOT EXISTS channel_sort_presets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            source_id TEXT NOT NULL,
            channel_order TEXT NOT NULL,
            created_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_sort_presets_src ON channel_sort_presets (source_id);

        CREATE TABLE IF NOT EXISTS channel_user_settings (
            source_id TEXT,
            channel_id TEXT,
            is_favorite INTEGER DEFAULT 0,
            is_hidden INTEGER DEFAULT 0,
            custom_icon TEXT,
            custom_name TEXT,
            PRIMARY KEY (source_id, channel_id)
        );
        CREATE INDEX IF NOT EXISTS idx_cus_src_ch ON channel_user_settings (source_id, channel_id);
        CREATE INDEX IF NOT EXISTS idx_cus_fav ON channel_user_settings (is_favorite, is_hidden);

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS app_languages (
            lang TEXT PRIMARY KEY,
            content TEXT,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS external_logo_cache (
            source_key TEXT PRIMARY KEY,
            content TEXT,
            updated_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_ext_logo_key ON external_logo_cache (source_key);

        CREATE TABLE IF NOT EXISTS tmdb_cache (
            query_title TEXT NOT NULL,
            lang TEXT NOT NULL,
            tmdb_id INTEGER,
            media_type TEXT,
            title TEXT,
            hu_title TEXT,
            en_title TEXT,
            original_title TEXT,
            original_language TEXT,
            poster_path TEXT,
            backdrop_path TEXT,
            overview TEXT,
            vote_average REAL,
            release_date TEXT,
            updated_at TEXT,
            PRIMARY KEY (query_title, lang)
        );
        CREATE INDEX IF NOT EXISTS idx_tmdb_cache ON tmdb_cache (query_title, lang);",
    )?;

    add_column_if_missing(conn, "sources", "country_code", "TEXT")?;
    add_column_if_missing(conn, "sources", "time_offset", "INTEGER")?;
    add_column_if_missing(conn, "sources", "sort_position", "INTEGER")?;
    add_column_if_missing(conn, "channel_user_settings", "custom_name", "TEXT")?;
    add_column_if_missing(conn, "tmdb_cache", "hu_title", "TEXT")?;
    add_column_if_missing(conn, "tmdb_cache", "en_title", "TEXT")?;
    add_column_if_missing(conn, "tmdb_cache", "original_language", "TEXT")?;

    Ok(())
}

fn migrate_v2(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS trakt_auth (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            access_blob TEXT NOT NULL,
            refresh_blob TEXT NOT NULL,
            client_secret_blob TEXT,
            expires_at INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS trakt_sync_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS trakt_cache (
            media_type TEXT NOT NULL,
            trakt_id INTEGER NOT NULL,
            tmdb_id INTEGER,
            imdb_id TEXT,
            slug TEXT,
            title TEXT,
            year INTEGER,
            overview TEXT,
            rating REAL,
            votes INTEGER,
            status TEXT,
            network TEXT,
            data_json TEXT,
            not_found INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (media_type, trakt_id)
        );
        CREATE INDEX IF NOT EXISTS idx_trakt_cache_tmdb ON trakt_cache (media_type, tmdb_id);

        CREATE TABLE IF NOT EXISTS trakt_watchlist_cache (
            trakt_id INTEGER PRIMARY KEY,
            media_type TEXT NOT NULL,
            title TEXT NOT NULL,
            title_hu TEXT,
            aliases_json TEXT,
            year INTEGER,
            tmdb_id INTEGER,
            poster_path TEXT,
            listed_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_trakt_wl_tmdb ON trakt_watchlist_cache (tmdb_id);

        CREATE TABLE IF NOT EXISTS trakt_history_cache (
            history_id INTEGER PRIMARY KEY,
            trakt_id INTEGER NOT NULL,
            media_type TEXT NOT NULL,
            show_trakt_id INTEGER,
            show_tmdb_id INTEGER,
            show_title TEXT,
            episode_title TEXT,
            season INTEGER,
            episode INTEGER,
            watched_at TEXT NOT NULL,
            poster_path TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_trakt_hist_watched ON trakt_history_cache (watched_at DESC);

        CREATE TABLE IF NOT EXISTS trakt_calendar_cache (
            scope TEXT NOT NULL,
            episode_trakt_id INTEGER NOT NULL,
            show_trakt_id INTEGER NOT NULL,
            show_tmdb_id INTEGER,
            show_title TEXT,
            season INTEGER,
            number INTEGER,
            episode_title TEXT,
            first_aired TEXT NOT NULL,
            network TEXT,
            fetched_at TEXT NOT NULL,
            PRIMARY KEY (scope, episode_trakt_id)
        );
        CREATE INDEX IF NOT EXISTS idx_trakt_cal_aired ON trakt_calendar_cache (scope, first_aired);

        CREATE TABLE IF NOT EXISTS trakt_episodes (
            show_trakt_id INTEGER NOT NULL,
            season INTEGER NOT NULL,
            episode INTEGER NOT NULL,
            episode_trakt_id INTEGER,
            title TEXT,
            watched_at TEXT,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (show_trakt_id, season, episode)
        );

        CREATE TABLE IF NOT EXISTS trakt_show_progress (
            show_trakt_id INTEGER PRIMARY KEY,
            aired INTEGER,
            completed INTEGER,
            last_watched_at TEXT,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS trakt_pending_actions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            last_error TEXT
        );

        CREATE TABLE IF NOT EXISTS trakt_http_cache (
            key TEXT PRIMARY KEY,
            etag TEXT,
            body TEXT NOT NULL,
            fetched_at TEXT NOT NULL,
            ttl_seconds INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_tmdb_cache_tmdb_id ON tmdb_cache (tmdb_id);",
    )?;

    add_column_if_missing(conn, "programmes", "title_norm", "TEXT")?;
    add_column_if_missing(conn, "programmes", "start_ts", "INTEGER")?;
    add_column_if_missing(conn, "programmes", "stop_ts", "INTEGER")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_prog_norm ON programmes (title_norm, start_ts)",
        [],
    )?;

    Ok(())
}

fn migrate_v3(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "trakt_show_progress", "next_episode_trakt_id", "INTEGER")?;
    add_column_if_missing(conn, "trakt_show_progress", "next_season", "INTEGER")?;
    add_column_if_missing(conn, "trakt_show_progress", "next_episode", "INTEGER")?;
    add_column_if_missing(conn, "trakt_show_progress", "next_episode_title", "TEXT")?;
    add_column_if_missing(
        conn,
        "trakt_show_progress",
        "hidden_from_progress",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

fn migrate_v4(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "trakt_show_progress", "next_episode_first_aired", "TEXT")?;
    Ok(())
}

/// v5 is purely additive (older binaries keep working on a v5 database):
///  * `trakt_http_cache.page_count`  - keeps pagination info for cached/304 answers
///  * `tmdb_cache.is_manual`         - separates the user's manual matches / "not this title" markers
///    from automatic cache rows (only automatic negatives may expire)
///  * `tmdb_cache.hu_overview/en_overview` - both language overviews in one row (used by the TMDB merge phase)
///  * `trakt_show_fp`                - per-show watched fingerprints (used by the progress-refresh phase)
fn migrate_v5(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "trakt_http_cache", "page_count", "INTEGER")?;

    // The backfill must run exactly once, right when the column appears: from then on
    // is_manual = 0 + 'none' means "automatic negative result" and may expire.
    if !column_exists(conn, "tmdb_cache", "is_manual")? {
        conn.execute(
            "ALTER TABLE tmdb_cache ADD COLUMN is_manual INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        // Legacy 'none' rows cannot be attributed (automatic miss vs. the user's clear_tmdb_match),
        // so they are protected as manual. Positive rows stay automatic until set_tmdb_match rewrites them.
        conn.execute("UPDATE tmdb_cache SET is_manual = 1 WHERE media_type = 'none'", [])?;
    }
    add_column_if_missing(conn, "tmdb_cache", "hu_overview", "TEXT")?;
    add_column_if_missing(conn, "tmdb_cache", "en_overview", "TEXT")?;

    // Separate table on purpose: a trakt_show_progress row means "progress was fetched at least once".
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS trakt_show_fp (
            show_trakt_id INTEGER PRIMARY KEY,
            seen_fp       TEXT,
            synced_fp     TEXT,
            updated_at    TEXT NOT NULL
        );",
    )?;
    Ok(())
}

/// v6 removes duplicate rows from `programmes` (same source, channel, start time, and normalized title),
/// keeping the entry with a valid duration (`stop_ts > start_ts`), longest description, and lowest `id`.
fn migrate_v6(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DELETE FROM programmes
         WHERE id IN (
             SELECT id FROM (
                 SELECT id,
                        ROW_NUMBER() OVER (
                            PARTITION BY source_id,
                                         channel_id,
                                         COALESCE(CAST(start_ts AS TEXT), start),
                                         COALESCE(NULLIF(title_norm, ''), LOWER(TRIM(title)))
                            ORDER BY
                                CASE WHEN stop_ts IS NOT NULL AND start_ts IS NOT NULL AND stop_ts > start_ts THEN 0 ELSE 1 END,
                                LENGTH(COALESCE(desc, '')) DESC,
                                id ASC
                        ) AS rn
                 FROM programmes
             )
             WHERE rn > 1
         );",
    )?;
    Ok(())
}

struct DbProgrammeRow {
    id: i64,
    channel_id: String,
    start: String,
    stop: String,
    start_ts: Option<i64>,
    stop_ts: Option<i64>,
    changed: bool,
}

impl crate::RepairableProgramme for DbProgrammeRow {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }
    fn start_str(&self) -> &str {
        &self.start
    }
    fn start_ts(&self) -> Option<i64> {
        self.start_ts
    }
    fn stop_ts(&self) -> Option<i64> {
        self.stop_ts
    }
    fn set_stop(&mut self, stop: String, stop_ts: i64) {
        self.stop = stop;
        self.stop_ts = Some(stop_ts);
        self.changed = true;
    }
}

/// v7 repairs existing `programmes` rows that have missing/zero-duration `stop` times or
/// 30-minute scraper placeholder `stop` times at day boundaries.
fn migrate_v7(conn: &Connection) -> rusqlite::Result<()> {
    let mut by_source: std::collections::HashMap<String, Vec<DbProgrammeRow>> =
        std::collections::HashMap::new();

    {
        let mut stmt = conn.prepare(
            "SELECT id, source_id, channel_id, start, stop, start_ts, stop_ts
             FROM programmes
             ORDER BY source_id, channel_id, start_ts, id",
        )?;
        let rows = stmt.query_map([], |r| {
            let id: i64 = r.get(0)?;
            let source_id: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
            let channel_id: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
            let start: String = r.get::<_, Option<String>>(3)?.unwrap_or_default();
            let stop: String = r.get::<_, Option<String>>(4)?.unwrap_or_default();
            let start_ts: Option<i64> = r.get(5)?;
            let stop_ts: Option<i64> = r.get(6)?;
            Ok((source_id, id, channel_id, start, stop, start_ts, stop_ts))
        })?;

        for row in rows {
            let (source_id, id, channel_id, start, stop, start_ts_db, stop_ts_db) = row?;
            let start_ts = start_ts_db.or_else(|| crate::xmltv_to_epoch(&start));
            let stop_ts = stop_ts_db.or_else(|| crate::xmltv_to_epoch(&stop));
            by_source.entry(source_id).or_default().push(DbProgrammeRow {
                id,
                channel_id,
                start,
                stop,
                start_ts,
                stop_ts,
                changed: false,
            });
        }
    }

    let mut upd = conn.prepare("UPDATE programmes SET stop = ?1, stop_ts = ?2 WHERE id = ?3")?;
    for (_source_id, mut src_rows) in by_source {
        crate::repair_source_programmes(&mut src_rows);
        for row in src_rows {
            if row.changed {
                upd.execute(params![row.stop, row.stop_ts, row.id])?;
            }
        }
    }

    Ok(())
}

/// v8 resets previously protected `media_type = 'none'` rows (`is_manual = 1`) back to automatic
/// (`is_manual = 0`) while keeping their original `updated_at`, so legacy pre-v5 negative rows
/// expire under the normal 7-day TTL and can be cleared by `clear_tmdb_cache`. Positive manual
/// matches (`movie`/`tv`) stay untouched, and future `clear_tmdb_match` clicks remain permanent.
fn migrate_v8(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE tmdb_cache SET is_manual = 0 WHERE media_type = 'none' AND is_manual = 1",
        [],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_from_fresh_db_and_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, None).unwrap();

        let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver, MIGRATIONS.len() as i64);

        assert!(column_exists(&conn, "programmes", "title_norm").unwrap());
        assert!(column_exists(&conn, "programmes", "start_ts").unwrap());
        assert!(column_exists(&conn, "programmes", "stop_ts").unwrap());
        assert!(column_exists(&conn, "sources", "sort_position").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_episode_trakt_id").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_season").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_episode").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_episode_title").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "hidden_from_progress").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_episode_first_aired").unwrap());
        assert!(column_exists(&conn, "trakt_http_cache", "page_count").unwrap());
        assert!(column_exists(&conn, "tmdb_cache", "is_manual").unwrap());
        assert!(column_exists(&conn, "tmdb_cache", "hu_overview").unwrap());
        assert!(column_exists(&conn, "tmdb_cache", "en_overview").unwrap());

        let trakt_tables = [
            "trakt_auth",
            "trakt_sync_state",
            "trakt_cache",
            "trakt_watchlist_cache",
            "trakt_history_cache",
            "trakt_calendar_cache",
            "trakt_episodes",
            "trakt_show_progress",
            "trakt_pending_actions",
            "trakt_http_cache",
            "trakt_show_fp",
        ];
        for tbl in trakt_tables {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    params![tbl],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "Missing table {}", tbl);
        }

        // Idempotent second run
        run_migrations(&conn, None).unwrap();
        let ver2: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver2, MIGRATIONS.len() as i64);
    }

    #[test]
    fn test_migrations_from_legacy_v0_db_preserves_data() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sources (id TEXT PRIMARY KEY, name TEXT, url TEXT, last_downloaded TEXT);
             INSERT INTO sources VALUES ('s1', 'Test', 'https://example.com/epg.xml', NULL);
             CREATE TABLE programmes (id INTEGER PRIMARY KEY AUTOINCREMENT, channel_id TEXT, source_id TEXT, start TEXT, stop TEXT, title TEXT, desc TEXT);
             INSERT INTO programmes (channel_id, source_id, start, stop, title, desc) VALUES ('c1', 's1', '20261118200000 +0100', '20261118210000 +0100', 'Stranger Things', 'Desc');",
        )
        .unwrap();

        run_migrations(&conn, None).unwrap();

        let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver, MIGRATIONS.len() as i64);

        let src_name: String = conn
            .query_row("SELECT name FROM sources WHERE id = 's1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(src_name, "Test");

        let prog_title: String = conn
            .query_row("SELECT title FROM programmes WHERE channel_id = 'c1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(prog_title, "Stranger Things");
        assert!(column_exists(&conn, "programmes", "title_norm").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_episode_trakt_id").unwrap());
        assert!(column_exists(&conn, "trakt_show_progress", "next_episode_first_aired").unwrap());
    }

    #[test]
    fn test_migrations_creates_backup_before_v2() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("epg_test_mig_backup.db");
        let bak_path = temp_dir.join("epg_test_mig_backup.db.bak-v1");
        let bak_v4_path = temp_dir.join("epg_test_mig_backup.db.bak-v4");
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&bak_path);
        let _ = std::fs::remove_file(&bak_v4_path);

        {
            let conn = Connection::open(&db_path).unwrap();
            migrate_v1(&conn).unwrap();
            conn.execute_batch("PRAGMA user_version = 1;").unwrap();
            run_migrations(&conn, Some(&db_path)).unwrap();
            let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(ver, MIGRATIONS.len() as i64);
        }

        assert!(bak_path.exists(), "Backup file should exist before v2 migration");
        assert!(bak_v4_path.exists(), "Backup file should exist before v5 migration");
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&bak_path);
        let _ = std::fs::remove_file(&bak_v4_path);
    }

    /// Builds a database that is exactly at schema v4 (what existing installs have).
    fn make_v4_db(conn: &Connection) {
        migrate_v1(conn).unwrap();
        migrate_v2(conn).unwrap();
        migrate_v3(conn).unwrap();
        migrate_v4(conn).unwrap();
        conn.execute_batch("PRAGMA user_version = 4;").unwrap();
    }

    #[test]
    fn test_v5_backfills_none_rows_as_manual_and_preserves_data() {
        let conn = Connection::open_in_memory().unwrap();
        make_v4_db(&conn);
        assert!(!column_exists(&conn, "tmdb_cache", "is_manual").unwrap());

        conn.execute_batch(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, updated_at)
                 VALUES ('legacy none', 'hu-HU', 'none', '2020-01-01 00:00:00');
             INSERT INTO tmdb_cache (query_title, lang, tmdb_id, media_type, title, updated_at)
                 VALUES ('dune', 'hu-HU', 438631, 'movie', 'Dune', '2020-01-01 00:00:00');",
        )
        .unwrap();

        migrate_v5(&conn).unwrap();

        let none_manual: i64 = conn
            .query_row("SELECT is_manual FROM tmdb_cache WHERE query_title = 'legacy none'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(none_manual, 1, "v5 itself marks legacy 'none' rows as manual");

        let (hit_manual, hit_title): (i64, String) = conn
            .query_row("SELECT is_manual, title FROM tmdb_cache WHERE query_title = 'dune'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(hit_manual, 0);
        assert_eq!(hit_title, "Dune");
    }

    #[test]
    fn test_v8_converts_legacy_none_rows_to_expiring_once() {
        let conn = Connection::open_in_memory().unwrap();
        make_v6_db(&conn);
        migrate_v7(&conn).unwrap();
        conn.execute_batch("PRAGMA user_version = 7;").unwrap();

        conn.execute_batch(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at)
                 VALUES ('legacy none', 'hu-HU', 'none', 1, '2020-01-01 00:00:00');
             INSERT INTO tmdb_cache (query_title, lang, tmdb_id, media_type, title, is_manual, updated_at)
                 VALUES ('manual dune', 'hu-HU', 438631, 'movie', 'Dune', 1, '2020-01-01 00:00:00');",
        )
        .unwrap();

        run_migrations(&conn, None).unwrap();

        let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver, MIGRATIONS.len() as i64);

        // Legacy 'none' row becomes expiring (is_manual = 0) while preserving its old updated_at
        let (none_manual, none_updated): (i64, String) = conn
            .query_row(
                "SELECT is_manual, updated_at FROM tmdb_cache WHERE query_title = 'legacy none'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(none_manual, 0);
        assert_eq!(none_updated, "2020-01-01 00:00:00");

        // Positive manual matches must remain protected
        let movie_manual: i64 = conn
            .query_row(
                "SELECT is_manual FROM tmdb_cache WHERE query_title = 'manual dune'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(movie_manual, 1);

        // Any NEW manual 'none' row created after v8 must stay permanent across restarts
        conn.execute(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at)
             VALUES ('new manual none', 'hu-HU', 'none', 1, datetime('now', 'localtime'))",
            [],
        )
        .unwrap();
        run_migrations(&conn, None).unwrap();
        let new_none_manual: i64 = conn
            .query_row(
                "SELECT is_manual FROM tmdb_cache WHERE query_title = 'new manual none'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(new_none_manual, 1, "Post-v8 manual 'none' markers must remain permanent");
    }

    #[test]
    fn test_v5_backfill_runs_only_once() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, None).unwrap();

        // A new automatic negative written after v5 must NOT be turned manual by a re-run of the migration.
        conn.execute(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at)
             VALUES ('auto none', 'hu-HU', 'none', 0, datetime('now','localtime'))",
            [],
        )
        .unwrap();

        migrate_v5(&conn).unwrap();
        migrate_v5(&conn).unwrap();

        let m: i64 = conn
            .query_row("SELECT is_manual FROM tmdb_cache WHERE query_title = 'auto none'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(m, 0);
    }

    #[test]
    fn test_v5_creates_backup_of_v4_database() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("epg_test_mig_v5_backup.db");
        let bak_v4_path = temp_dir.join("epg_test_mig_v5_backup.db.bak-v4");
        let bak_v1_path = temp_dir.join("epg_test_mig_v5_backup.db.bak-v1");
        for p in [&db_path, &bak_v4_path, &bak_v1_path] {
            let _ = std::fs::remove_file(p);
        }

        {
            let conn = Connection::open(&db_path).unwrap();
            make_v4_db(&conn);
            conn.execute(
                "INSERT INTO tmdb_cache (query_title, lang, media_type, updated_at) VALUES ('x', 'hu-HU', 'none', '2020-01-01 00:00:00')",
                [],
            )
            .unwrap();
            run_migrations(&conn, Some(&db_path)).unwrap();
        }

        assert!(bak_v4_path.exists(), "v4 backup must exist before the v5 migration");
        assert!(!bak_v1_path.exists(), "v2 backup must not be created for a v4 database");

        // The backup is the pre-migration state: it has no is_manual column yet.
        let bak = Connection::open(&bak_v4_path).unwrap();
        assert!(!column_exists(&bak, "tmdb_cache", "is_manual").unwrap());
        drop(bak);

        for p in [&db_path, &bak_v4_path, &bak_v1_path] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn test_v5_is_additive_old_binary_compatible() {
        // An "old binary" only knows MIGRATIONS[0..4]; opening a v5 DB must be a no-op for it.
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, None).unwrap();
        let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert!(ver as usize >= 4);
        // The pre-v5 insert shape (no is_manual column mentioned) still works.
        conn.execute(
            "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, updated_at) VALUES ('t', 'hu-HU', 'none', datetime('now','localtime'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO trakt_http_cache (key, body, etag, fetched_at, ttl_seconds) VALUES ('/k', '{}', NULL, '2026-01-01T00:00:00Z', 60)",
            [],
        )
        .unwrap();
    }

    fn make_v5_db(conn: &Connection) {
        migrate_v1(conn).unwrap();
        migrate_v2(conn).unwrap();
        migrate_v3(conn).unwrap();
        migrate_v4(conn).unwrap();
        migrate_v5(conn).unwrap();
        conn.execute_batch("PRAGMA user_version = 5;").unwrap();
    }

    #[test]
    fn test_v6_deduplicates_programmes_keeping_best_row() {
        let conn = Connection::open_in_memory().unwrap();
        make_v5_db(&conn);

        conn.execute_batch(
            "INSERT INTO programmes (channel_id, source_id, start, stop, title, desc, title_norm, start_ts, stop_ts) VALUES
                ('398', 'porthu', '20261124200000 +0000', '20261124205000 +0000', 'Tulsa királya', 'IV / 5. rész', 'tulsa kiralya', 1795550400, 1795553400),
                ('398', 'porthu', '20261125232000 +0000', '20261126001000 +0000', 'Tulsa királya', 'IV / 5. rész', 'tulsa kiralya', 1795648800, 1795651800),
                ('398', 'porthu', '20261125232000 +0000', '20261126001000 +0000', 'Tulsa királya', 'IV / 5. rész', 'tulsa kiralya', 1795648800, 1795651800),
                ('398', 'porthu', '20261128234500 +0000', '20261129003500 +0000', 'Tulsa királya', '', 'tulsa kiralya', 1795909500, 1795909500),
                ('398', 'porthu', '20261128234500 +0000', '20261129003500 +0000', 'Tulsa királya', 'Részletes leírás', 'tulsa kiralya', 1795909500, 1795912500),
                ('399', 'porthu', '20261125232000 +0000', '20261126001000 +0000', 'Tulsa királya', 'Másik csatorna', 'tulsa kiralya', 1795648800, 1795651800),
                ('398', 'musor',  '20261125232000 +0000', '20261126001000 +0000', 'Tulsa királya', 'Másik forrás', 'tulsa kiralya', 1795648800, 1795651800),
                ('100', 'porthu', '20261125100000 +0000', '20261125110000 +0000', 'Legacy Show', '', NULL, NULL, NULL),
                ('100', 'porthu', '20261125100000 +0000', '20261125110000 +0000', 'Legacy Show', 'Hosszabb leírás', NULL, NULL, NULL);",
        )
        .unwrap();

        run_migrations(&conn, None).unwrap();

        let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver, MIGRATIONS.len() as i64);

        let porthu_398_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM programmes WHERE source_id = 'porthu' AND channel_id = '398'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(porthu_398_count, 3, "5 rows (with 2 duplicates) must collapse to 3 distinct airings");

        // On Nov 29, the valid-duration + richer-desc row must be kept over the zero-duration row
        let (kept_stop, kept_desc): (i64, String) = conn
            .query_row(
                "SELECT stop_ts, desc FROM programmes WHERE source_id = 'porthu' AND channel_id = '398' AND start_ts = 1795909500",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kept_stop, 1795912500);
        assert_eq!(kept_desc, "Részletes leírás");

        // Different channel and different source at the same timestamp must remain intact
        let other_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM programmes WHERE (source_id = 'porthu' AND channel_id = '399') OR (source_id = 'musor' AND channel_id = '398')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(other_count, 2);

        // Legacy rows with NULL title_norm/start_ts must also deduplicate keeping the richer desc
        let legacy_desc: String = conn
            .query_row(
                "SELECT desc FROM programmes WHERE source_id = 'porthu' AND channel_id = '100'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let legacy_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM programmes WHERE source_id = 'porthu' AND channel_id = '100'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(legacy_count, 1);
        assert_eq!(legacy_desc, "Hosszabb leírás");

        // Idempotent re-run
        migrate_v6(&conn).unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM programmes", [], |r| r.get(0)).unwrap();
        assert_eq!(total, 6);
    }

    fn make_v6_db(conn: &Connection) {
        make_v5_db(conn);
        migrate_v6(conn).unwrap();
        conn.execute_batch("PRAGMA user_version = 6;").unwrap();
    }

    #[test]
    fn test_v7_repairs_30m_dummy_stops_and_zero_durations_without_touching_clean_sources() {
        let conn = Connection::open_in_memory().unwrap();
        make_v6_db(&conn);

        conn.execute_batch(
            "INSERT INTO programmes (channel_id, source_id, start, stop, title, desc, title_norm, start_ts, stop_ts) VALUES
                -- Source with 30m dummy stops (gap + overlap + legacy NULL timestamps)
                ('5-star-max', 'src_dummy', '20261126030800 +0000', '20261126033800 +0000', 'The Hangover Part II', '', 'the hangover part ii', 1795662480, 1795664280),
                ('5-star-max', 'src_dummy', '20261126045000 +0000', '20261126064700 +0000', 'Time Bandits', '', 'time bandits', 1795668600, 1795675620),
                ('bbc-news',   'src_dummy', '20261126034500 +0000', '20261126041500 +0000', 'BBC Sport', '', 'bbc sport', NULL, NULL),
                ('bbc-news',   'src_dummy', '20261126040000 +0000', '20261126043000 +0000', 'BBC News', '', 'bbc news', NULL, NULL),
                -- Clean source with real gaps and a zero-duration live event
                ('kika',  'src_clean', '20261125193500 +0000', '20261125200000 +0000', 'My Move', '', 'my move', 1795635300, 1795636800),
                ('kika',  'src_clean', '20261126050000 +0000', '20261126053000 +0000', 'Kikaninchen', '', 'kikaninchen', 1795669200, 1795671000),
                ('kika',  'src_clean', '20261126193000 +0000', '20261126200000 +0000', 'Cave Painter', '', 'cave painter', 1795721400, 1795723200),
                ('kika',  'src_clean', '20261127050000 +0000', '20261127053000 +0000', 'Logo', '', 'logo', 1795755600, 1795757400),
                ('swr',   'src_clean', '20261126163000 +0000', '20261126170000 +0000', 'Sportarena', '', 'sportarena', 1795710600, 1795712400),
                ('swr',   'src_clean', '20261126171500 +0000', '20261126174500 +0000', 'Flohmarkt', '', 'flohmarkt', 1795713300, 1795715100),
                ('swr',   'src_clean', '20261126174500 +0000', '20261126181500 +0000', 'Mit Herz', '', 'mit herz', 1795715100, 1795716900),
                ('swr',   'src_clean', '20261126190000 +0000', '20261126191500 +0000', 'Tagesschau', '', 'tagesschau', 1795719600, 1795720500),
                ('swr',   'src_clean', '20261126191500 +0000', '20261126193000 +0000', 'Short', '', 'short', 1795720500, 1795721400),
                ('swr',   'src_clean', '20261126200000 +0000', '20261126203000 +0000', 'Late', '', 'late', 1795723200, 1795725000),
                ('swr',   'src_clean', '20261126203000 +0000', '20261126204500 +0000', 'News', '', 'news', 1795725000, 1795725900),
                ('swr',   'src_clean', '20261126210000 +0000', '20261126213000 +0000', 'Night', '', 'night', 1795726800, 1795728600),
                ('dazn1', 'src_clean', '20261126172000 +0000', '20261126172000 +0000', 'UCL', '', 'ucl', 1795713600, 1795713600),
                ('dazn1', 'src_clean', '20261126191000 +0000', '20261126210000 +0000', 'UCL 2', '', 'ucl 2', 1795720200, 1795726800);",
        )
        .unwrap();

        run_migrations(&conn, None).unwrap();

        let (hangover_stop, hangover_stop_ts): (String, i64) = conn
            .query_row(
                "SELECT stop, stop_ts FROM programmes WHERE source_id = 'src_dummy' AND channel_id = '5-star-max' AND title = 'The Hangover Part II'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(hangover_stop, "20261126045000 +0000");
        assert_eq!(hangover_stop_ts, 1795668600);

        let (bbc_stop, bbc_stop_ts): (String, i64) = conn
            .query_row(
                "SELECT stop, stop_ts FROM programmes WHERE source_id = 'src_dummy' AND channel_id = 'bbc-news' AND title = 'BBC Sport'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(bbc_stop, "20261126040000 +0000");
        assert_eq!(bbc_stop_ts, crate::xmltv_to_epoch("20261126040000 +0000").unwrap());

        // Clean source: zero-duration DAZN1 is repaired, while legitimate 30m Sportarena is untouched
        let dazn_stop: String = conn
            .query_row(
                "SELECT stop FROM programmes WHERE source_id = 'src_clean' AND channel_id = 'dazn1' AND title = 'UCL'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dazn_stop, "20261126191000 +0000");

        let swr_stop: String = conn
            .query_row(
                "SELECT stop FROM programmes WHERE source_id = 'src_clean' AND channel_id = 'swr' AND title = 'Sportarena'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(swr_stop, "20261126170000 +0000");

        // Idempotent re-run
        migrate_v7(&conn).unwrap();
        let hangover_stop_2: String = conn
            .query_row(
                "SELECT stop FROM programmes WHERE source_id = 'src_dummy' AND channel_id = '5-star-max' AND title = 'The Hangover Part II'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(hangover_stop_2, "20261126045000 +0000");
    }

    #[test]
    fn test_sync_embedded_language_updates_unmodified_and_merges_custom_translations() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, None).unwrap();

        // Both embedded languages must store their hash and contain confirm_clear_tmdb_cache
        for (lang, embedded) in [("english", EMBEDDED_ENGLISH_LNG), ("hungarian", EMBEDDED_HUNGARIAN_LNG)] {
            let stored_hash: String = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = ?1",
                    params![format!("lng_hash_{}", lang)],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(stored_hash, canonical_json_sha256(embedded));

            let db_content: String = conn
                .query_row("SELECT content FROM app_languages WHERE lang = ?1", params![lang], |r| r.get(0))
                .unwrap();
            let parsed: serde_json::Value = serde_json::from_str(&db_content).unwrap();
            assert!(
                parsed.get("confirm_clear_tmdb_cache").and_then(|v| v.as_str()).is_some(),
                "{} must contain confirm_clear_tmdb_cache",
                lang
            );
        }

        // 1. Unmodified language (even if minified by i18n.js save_language_content) gets cleanly replaced on update
        let v1 = r#"{ "greeting": "Hello", "old": "Old" }"#;
        let v1_minified = r#"{"old":"Old","greeting":"Hello"}"#;
        let v2 = r#"{ "greeting": "Hello v2", "old": "Old", "new_key": "New" }"#;
        sync_embedded_language(&conn, "testlang", v1).unwrap();
        conn.execute(
            "UPDATE app_languages SET content = ?1 WHERE lang = 'testlang'",
            params![v1_minified],
        )
        .unwrap();
        sync_embedded_language(&conn, "testlang", v2).unwrap();
        let after_replace: serde_json::Value = serde_json::from_str(
            &conn
                .query_row("SELECT content FROM app_languages WHERE lang = 'testlang'", [], |r| r.get::<_, String>(0))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(after_replace["greeting"], "Hello v2");
        assert_eq!(after_replace["new_key"], "New");

        // 2. User-customized language preserves custom values while receiving new keys
        let customized = r#"{"greeting":"Custom Hello","old":"Old","new_key":"New"}"#;
        conn.execute(
            "UPDATE app_languages SET content = ?1 WHERE lang = 'testlang'",
            params![customized],
        )
        .unwrap();
        let v3 = r#"{ "greeting": "Hello v3", "old": "Old v3", "new_key": "New", "brand_new": "Fresh" }"#;
        sync_embedded_language(&conn, "testlang", v3).unwrap();
        let after_merge: serde_json::Value = serde_json::from_str(
            &conn
                .query_row("SELECT content FROM app_languages WHERE lang = 'testlang'", [], |r| r.get::<_, String>(0))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(after_merge["greeting"], "Custom Hello", "User customization must not be overwritten");
        assert_eq!(after_merge["old"], "Old", "Existing value must stay intact on merge");
        assert_eq!(after_merge["brand_new"], "Fresh", "Newly added embedded key must be merged in");
    }
}
