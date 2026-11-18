use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub type MigFn = fn(&Connection) -> rusqlite::Result<()>;

pub const EMBEDDED_ENGLISH_LNG: &str = include_str!("../../public/languages/english.lng");
pub const EMBEDDED_HUNGARIAN_LNG: &str = include_str!("../../public/languages/hungarian.lng");

pub const MIGRATIONS: &[MigFn] = &[migrate_v1, migrate_v2];

pub fn column_exists(conn: &Connection, table: &str, col: &str) -> rusqlite::Result<bool> {
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

fn backup_db_file(db_path: &Path) -> Result<PathBuf, String> {
    let mut bak_name = db_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "epg.db".to_string());
    bak_name.push_str(".bak-v1");
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
        if ver == 2 {
            if let Some(path) = db_path {
                if path.exists() {
                    backup_path = Some(backup_db_file(path)?);
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
            time_offset INTEGER
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

    let has_migrated_legacy_cleanup: bool = conn
        .query_row(
            "SELECT 1 FROM settings WHERE key = 'migration_cleanup_v1'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !has_migrated_legacy_cleanup {
        conn.execute(
            "DELETE FROM tmdb_cache WHERE query_title = 'Jurassic World' AND (title LIKE '%gefallene%' OR title LIKE '%Fallen Kingdom%')",
            [],
        )?;
        conn.execute(
            "DELETE FROM tmdb_cache WHERE query_title = 'G.I. Joe' AND (title LIKE '%Megtorlás%' OR title LIKE '%Retaliation%')",
            [],
        )?;

        conn.execute(
            "DELETE FROM sources WHERE id IN ('src_porthu', 'src_magenta', 'src_sky')",
            [],
        )?;
        conn.execute(
            "DELETE FROM channels WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')",
            [],
        )?;
        conn.execute(
            "DELETE FROM programmes WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')",
            [],
        )?;
        conn.execute(
            "DELETE FROM channel_orders WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')",
            [],
        )?;

        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('migration_cleanup_v1', '1')",
            [],
        )?;
    }

    let lang_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM app_languages", [], |r| r.get(0))
        .unwrap_or(0);
    if lang_count == 0 {
        conn.execute(
            "INSERT OR IGNORE INTO app_languages (lang, content, updated_at) VALUES ('english', ?1, datetime('now', 'localtime'))",
            params![EMBEDDED_ENGLISH_LNG],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO app_languages (lang, content, updated_at) VALUES ('hungarian', ?1, datetime('now', 'localtime'))",
            params![EMBEDDED_HUNGARIAN_LNG],
        )?;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrations_from_fresh_db_and_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn, None).unwrap();

        let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(ver, 2);

        assert!(column_exists(&conn, "programmes", "title_norm").unwrap());
        assert!(column_exists(&conn, "programmes", "start_ts").unwrap());
        assert!(column_exists(&conn, "programmes", "stop_ts").unwrap());
        assert!(column_exists(&conn, "sources", "sort_position").unwrap());

        let trakt_tables = [
            "trakt_auth",
            "trakt_sync_state",
            "trakt_http_cache",
            "trakt_watchlist_cache",
            "trakt_history_cache",
            "trakt_calendar_cache",
            "trakt_episodes",
            "trakt_show_progress",
            "trakt_pending_actions",
            "trakt_http_cache",
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
        assert_eq!(ver2, 2);
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
        assert_eq!(ver, 2);

        let src_name: String = conn
            .query_row("SELECT name FROM sources WHERE id = 's1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(src_name, "Test");

        let prog_title: String = conn
            .query_row("SELECT title FROM programmes WHERE channel_id = 'c1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(prog_title, "Stranger Things");
        assert!(column_exists(&conn, "programmes", "title_norm").unwrap());
    }

    #[test]
    fn test_migrations_creates_backup_before_v2() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("epg_test_mig_backup.db");
        let bak_path = temp_dir.join("epg_test_mig_backup.db.bak-v1");
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&bak_path);

        {
            let conn = Connection::open(&db_path).unwrap();
            migrate_v1(&conn).unwrap();
            conn.execute_batch("PRAGMA user_version = 1;").unwrap();
            run_migrations(&conn, Some(&db_path)).unwrap();
            let ver: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
            assert_eq!(ver, 2);
        }

        assert!(bak_path.exists(), "Backup file should exist before v2 migration");
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&bak_path);
    }
}
