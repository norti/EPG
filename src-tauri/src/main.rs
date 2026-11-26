#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod migrations;
mod trakt;
mod updater;
#[cfg(test)]
mod tmdb_tests;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, RwLock};
use tokio::sync::Mutex;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use flate2::read::GzDecoder;
use regex::Regex;

#[derive(Default)]
pub struct RequestStats {
    pub trakt_net: AtomicU64,
    pub trakt_304: AtomicU64,
    pub trakt_cache_hit: AtomicU64,
    pub trakt_post: AtomicU64,
    pub trakt_429: AtomicU64,
    pub tmdb_net: AtomicU64,
    pub tmdb_429: AtomicU64,
    pub tmdb_cache_hit: AtomicU64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestStatsSnapshot {
    pub trakt_net: u64,
    pub trakt_304: u64,
    pub trakt_cache_hit: u64,
    pub trakt_post: u64,
    pub trakt_429: u64,
    pub tmdb_net: u64,
    pub tmdb_429: u64,
    pub tmdb_cache_hit: u64,
}

impl RequestStats {
    pub fn snapshot(&self) -> RequestStatsSnapshot {
        RequestStatsSnapshot {
            trakt_net: self.trakt_net.load(Ordering::Relaxed),
            trakt_304: self.trakt_304.load(Ordering::Relaxed),
            trakt_cache_hit: self.trakt_cache_hit.load(Ordering::Relaxed),
            trakt_post: self.trakt_post.load(Ordering::Relaxed),
            trakt_429: self.trakt_429.load(Ordering::Relaxed),
            tmdb_net: self.tmdb_net.load(Ordering::Relaxed),
            tmdb_429: self.tmdb_429.load(Ordering::Relaxed),
            tmdb_cache_hit: self.tmdb_cache_hit.load(Ordering::Relaxed),
        }
    }

    pub fn reset(&self) {
        self.trakt_net.store(0, Ordering::Relaxed);
        self.trakt_304.store(0, Ordering::Relaxed);
        self.trakt_cache_hit.store(0, Ordering::Relaxed);
        self.trakt_post.store(0, Ordering::Relaxed);
        self.trakt_429.store(0, Ordering::Relaxed);
        self.tmdb_net.store(0, Ordering::Relaxed);
        self.tmdb_429.store(0, Ordering::Relaxed);
        self.tmdb_cache_hit.store(0, Ordering::Relaxed);
    }
}

impl RequestStatsSnapshot {
    pub fn diff(&self, before: &Self) -> Self {
        Self {
            trakt_net: self.trakt_net.saturating_sub(before.trakt_net),
            trakt_304: self.trakt_304.saturating_sub(before.trakt_304),
            trakt_cache_hit: self.trakt_cache_hit.saturating_sub(before.trakt_cache_hit),
            trakt_post: self.trakt_post.saturating_sub(before.trakt_post),
            trakt_429: self.trakt_429.saturating_sub(before.trakt_429),
            tmdb_net: self.tmdb_net.saturating_sub(before.tmdb_net),
            tmdb_429: self.tmdb_429.saturating_sub(before.tmdb_429),
            tmdb_cache_hit: self.tmdb_cache_hit.saturating_sub(before.tmdb_cache_hit),
        }
    }
}

#[tauri::command]
async fn get_request_stats(state: tauri::State<'_, AppState>) -> Result<RequestStatsSnapshot, String> {
    Ok(state.stats.snapshot())
}

#[tauri::command]
async fn reset_request_stats(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.stats.reset();
    Ok(())
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub http: reqwest::Client,
    pub http_epg: reqwest::Client,
    pub trakt: trakt::TraktState,
    pub tmdb: TmdbRuntime,
    pub stats: RequestStats,
    /// Coalesces concurrent image/logo downloads sharing the same URL.
    pub image_inflight: trakt::client::SharedInflightMap<Result<String, String>>,
    pub updater: updater::UpdaterState,
}

/// Runtime settings of the TMDB layer. The base URL is injectable so tests can point it at a mock server.
pub struct TmdbRuntime {
    pub base: String,
    /// Limits concurrent TMDB requests (sync enrichment + UI lookups share it).
    pub sem: tokio::sync::Semaphore,
    /// Pause before retrying a 5xx / network error.
    pub retry_delay: std::time::Duration,
    /// Serializes concurrent cache-backed TMDB lookups (`search:{lang}:{query}`, `id:{type}:{id}:{lang}`).
    pub cache_inflight: trakt::client::InflightMap,
    /// Coalesces concurrent raw HTTP TMDB calls sharing the same flight key.
    pub(crate) http_inflight: trakt::client::SharedInflightMap<Result<String, TmdbError>>,
    /// In-memory cache of the decrypted TMDB API key (`None` = unloaded, `Some(None)` = unset).
    pub(crate) api_key_cache: tokio::sync::RwLock<Option<Option<zeroize::Zeroizing<String>>>>,
}

impl TmdbRuntime {
    pub const DEFAULT_BASE: &'static str = "https://api.themoviedb.org/3";
    const MAX_CONCURRENT: usize = 4;

    pub fn new() -> Self {
        Self::with_base(Self::DEFAULT_BASE, std::time::Duration::from_millis(500))
    }

    pub fn with_base(base: impl Into<String>, retry_delay: std::time::Duration) -> Self {
        TmdbRuntime {
            base: base.into(),
            sem: tokio::sync::Semaphore::new(Self::MAX_CONCURRENT),
            retry_delay,
            cache_inflight: std::sync::Mutex::new(HashMap::new()),
            http_inflight: std::sync::Mutex::new(HashMap::new()),
            api_key_cache: tokio::sync::RwLock::new(None),
        }
    }
}

impl Default for TmdbRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Channel {
    id: String,
    name: String,
    original_name: Option<String>,
    custom_name: Option<String>,
    icon: String,
    custom_icon: Option<String>,
    cached_icon: Option<String>,
    #[serde(default)]
    icon_failed: bool,
    is_favorite: bool,
    is_hidden: bool,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    source_name: Option<String>,
    #[serde(default)]
    country_code: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Programme {
    channel: String,
    start: String,
    stop: String,
    title: String,
    desc: String,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    time_offset: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug)]
struct EpgResponse {
    channels: Vec<Channel>,
    programmes: Vec<Programme>,
}

use crate::migrations::{EMBEDDED_ENGLISH_LNG, EMBEDDED_HUNGARIAN_LNG};

fn get_embedded_language(lang: &str) -> Option<&'static str> {
    match lang.to_lowercase().as_str() {
        "english" | "en" => Some(EMBEDDED_ENGLISH_LNG),
        "hungarian" | "hu" => Some(EMBEDDED_HUNGARIAN_LNG),
        _ => None,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Source {
    id: String,
    name: String,
    url: String,
    last_downloaded: Option<String>,
    country_code: Option<String>,
    time_offset: Option<i32>,
    #[serde(default)]
    sort_position: Option<i64>,
    #[serde(default)]
    channel_count: Option<i64>,
    #[serde(default)]
    favorite_count: Option<i64>,
    #[serde(default)]
    hidden_count: Option<i64>,
}

fn get_app_db_path(filename: &str) -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("epg-viewer");
    fs::create_dir_all(&path).ok();
    path.push(filename);
    path
}

fn get_db_path() -> PathBuf {
    get_app_db_path("epg.db")
}

fn get_images_db_path() -> PathBuf {
    get_app_db_path("images.db")
}

fn open_and_init_db() -> Result<Connection, String> {
    let db_path = get_db_path();
    let mut conn = Connection::open(&db_path)
        .map_err(|e| format!("DB megnyitási hiba: {}", e))?;

    let _ = conn.execute("PRAGMA foreign_keys = ON;", []);

    // images.db csatolása külön adatbázisként
    let images_db_path = get_images_db_path();
    let attach_sql = format!(
        "ATTACH DATABASE '{}' AS images_db",
        images_db_path.to_string_lossy().replace('\'', "''")
    );
    conn.execute(&attach_sql, [])
        .map_err(|e| format!("images.db csatolási hiba: {}", e))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS images_db.images (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            sha256 TEXT NOT NULL UNIQUE,
            mime_type TEXT NOT NULL,
            byte_size INTEGER NOT NULL,
            data TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS images_db.idx_images_sha256 ON images (sha256);

        CREATE TABLE IF NOT EXISTS images_db.image_urls (
            url TEXT PRIMARY KEY,
            image_id INTEGER NOT NULL,
            image_type TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS images_db.idx_image_urls_image_id ON image_urls (image_id);

        CREATE TABLE IF NOT EXISTS images_db.failed_logo_cache (
            url TEXT PRIMARY KEY,
            status_code INTEGER,
            error TEXT NOT NULL,
            failed_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS images_db.idx_failed_logo_expires ON failed_logo_cache (expires_at);",
    )
    .map_err(|e| format!("images_db táblák inicializálási hiba: {}", e))?;

    // Ellenőrizzük, hogy létezik-e még korábbi valódi táblaként a main.logo_cache
    let is_old_table: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'logo_cache'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    if is_old_table {
        if let Ok(mut stmt) = conn.prepare("SELECT url, data, created_at FROM main.logo_cache") {
            let rows = stmt.query_map([], |row| {
                let url: String = row.get(0)?;
                let data: String = row.get(1)?;
                let cr: String = row.get(2).unwrap_or_default();
                Ok((url, data, cr))
            });

            if let Ok(iter) = rows {
                use sha2::{Digest, Sha256};
                for item in iter.flatten() {
                    let (url, data, created_at) = item;
                    if data.trim().is_empty() { continue; }

                    let mut hasher = Sha256::new();
                    hasher.update(data.as_bytes());
                    let hash_hex = format!("{:x}", hasher.finalize());

                    let mime = if data.contains("image/svg+xml") || url.ends_with(".svg") {
                        "image/svg+xml"
                    } else if data.contains("image/jpeg") {
                        "image/jpeg"
                    } else if data.contains("image/webp") {
                        "image/webp"
                    } else {
                        "image/png"
                    };
                    let byte_size = data.len() as i64;
                    let img_type = if url.contains("image.tmdb.org") { "poster" } else { "logo" };

                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO images_db.images (sha256, mime_type, byte_size, data, created_at)
                         VALUES (?1, ?2, ?3, ?4, COALESCE(NULLIF(?5, ''), datetime('now')))",
                        params![hash_hex, mime, byte_size, data, created_at],
                    );

                    let img_id: Option<i64> = conn.query_row(
                        "SELECT id FROM images_db.images WHERE sha256 = ?1",
                        params![hash_hex],
                        |r| r.get(0),
                    ).ok();

                    if let Some(id) = img_id {
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO images_db.image_urls (url, image_id, image_type, created_at)
                             VALUES (?1, ?2, ?3, COALESCE(NULLIF(?4, ''), datetime('now')))",
                            params![url, id, img_type, created_at],
                        );
                    }
                }
            }
        }
        let _ = conn.execute("DROP TABLE main.logo_cache", []);
        let _ = conn.execute("VACUUM", []);
    }

    // Régi kompatibilitási nézetek eltávolítása a lassú materializáció elkerülésére
    let _ = conn.execute("DROP VIEW IF EXISTS images_db.logo_cache", []);
    let _ = conn.execute("DROP VIEW IF EXISTS main.logo_cache", []);

    // 1. Minden korábbi WAL naplóbejegyzés azonnali beépítése az alap adatbázisfájlokba (checkpoint)
    let _ = conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", []);
    let _ = conn.execute("PRAGMA images_db.wal_checkpoint(TRUNCATE)", []);

    // 2. Visszaállítjuk a szabványos DELETE naplózási módot, amely automatikusan
    // beépíti az adatokat és törli a .db-wal és .db-shm fájlokat a lemezről.
    // Ezentúl minden adat közvetlenül és kizárólag az epg.db és images.db fájlokba kerül.
    let _ = conn.execute("PRAGMA journal_mode = DELETE", []);
    let _ = conn.execute("PRAGMA images_db.journal_mode = DELETE", []);

    // 3. Teljesítmény-optimalizáló PRAGMA beállítások (gyors memóriagyorsítótár + busy timeout)
    let _ = conn.execute_batch(
        "PRAGMA busy_timeout = 5000;
         PRAGMA synchronous = NORMAL;
         PRAGMA cache_size = -64000;
         PRAGMA temp_store = MEMORY;
         PRAGMA images_db.busy_timeout = 5000;
         PRAGMA images_db.synchronous = NORMAL;
         PRAGMA images_db.cache_size = -64000;",
    );

    if let Err(err) = migrations::run_migrations(&conn, Some(&db_path)) {
        drop(conn);
        let bak_path = db_path.with_file_name("epg.db.bak-v1");
        if bak_path.exists() {
            let _ = fs::copy(&bak_path, &db_path);
        }
        return Err(err);
    }

    let secret_store = trakt::secret::default_secret_store();
    if let Err(err) = migrate_plaintext_tmdb_api_key(&mut conn, secret_store.as_ref()) {
        eprintln!("[TMDB] API kulcs DPAPI migráció sikertelen: {}", err);
    }

    Ok(conn)
}

fn query_all_favorites_channels(conn: &Connection) -> Result<Vec<Channel>, String> {
    let now_ts = trakt::now_epoch_secs();
    let mut stmt = conn.prepare(
        "SELECT c.id,
                c.source_id,
                COALESCE(s.name, '') AS source_name,
                COALESCE(NULLIF(cus.custom_name, ''), c.name) AS display_name,
                c.name AS original_name,
                cus.custom_name,
                c.icon,
                cus.custom_icon,
                img.sha256 AS cached_sha,
                COALESCE(cus.is_favorite, 0),
                COALESCE(cus.is_hidden, 0),
                COALESCE(s.country_code, 'HU') AS country_code,
                CASE WHEN flc.url IS NOT NULL THEN 1 ELSE 0 END AS icon_failed
         FROM channels c
         JOIN sources s ON s.id = c.source_id
         JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id
         LEFT JOIN channel_orders co_fav ON co_fav.source_id = '__ALL_FAVORITES__' AND co_fav.channel_id = (c.source_id || '::' || c.id)
         LEFT JOIN images_db.image_urls iu ON iu.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
         LEFT JOIN images_db.images img ON img.id = iu.image_id
         LEFT JOIN images_db.failed_logo_cache flc
           ON flc.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
          AND flc.expires_at > ?1
         WHERE cus.is_favorite = 1 AND COALESCE(cus.is_hidden, 0) = 0
         ORDER BY
            CASE WHEN co_fav.position IS NOT NULL THEN 0 ELSE 1 END,
            co_fav.position ASC,
            COALESCE(s.sort_position, s.rowid) ASC,
            CASE WHEN co.position IS NOT NULL THEN 0 ELSE 1 END,
            co.position ASC,
            display_name ASC"
    ).map_err(|e| e.to_string())?;

    let channel_iter = stmt.query_map(params![now_ts], |row| {
        let raw_id: String = row.get(0)?;
        let src_id: String = row.get(1)?;
        let src_name: String = row.get(2)?;
        let composite_id = format!("{}::{}", src_id, raw_id);
        let cached_sha: Option<String> = row.get(8)?;
        let is_fav: i32 = row.get(9)?;
        let is_hid: i32 = row.get(10)?;
        let country_code: String = row.get(11)?;
        let icon_failed: i32 = row.get(12)?;
        let cached_icon = cached_sha
            .filter(|s| !s.trim().is_empty())
            .map(|sha| image_protocol_url(&sha));
        Ok(Channel {
            id: composite_id,
            name: row.get(3)?,
            original_name: row.get(4)?,
            custom_name: row.get(5)?,
            icon: row.get(6)?,
            custom_icon: row.get(7)?,
            cached_icon: cached_icon.clone(),
            icon_failed: cached_icon.is_none() && icon_failed == 1,
            is_favorite: is_fav == 1,
            is_hidden: is_hid == 1,
            source_id: Some(src_id),
            source_name: Some(src_name),
            country_code: Some(country_code),
        })
    }).map_err(|e| e.to_string())?;

    Ok(channel_iter.flatten().collect())
}

fn query_source_channels(conn: &Connection, source_id: &str) -> Result<Vec<Channel>, String> {
    let now_ts = trakt::now_epoch_secs();
    let mut stmt = conn.prepare(
        "SELECT c.id,
                COALESCE(NULLIF(cus.custom_name, ''), c.name) AS display_name,
                c.name AS original_name,
                cus.custom_name,
                c.icon,
                cus.custom_icon,
                img.sha256 AS cached_sha,
                COALESCE(cus.is_favorite, 0),
                COALESCE(cus.is_hidden, 0),
                COALESCE(s.country_code, 'HU') AS country_code,
                COALESCE(s.name, '') AS source_name,
                CASE WHEN flc.url IS NOT NULL THEN 1 ELSE 0 END AS icon_failed
         FROM channels c
         JOIN sources s ON s.id = c.source_id
         LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id
         LEFT JOIN images_db.image_urls iu ON iu.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
         LEFT JOIN images_db.images img ON img.id = iu.image_id
         LEFT JOIN images_db.failed_logo_cache flc
           ON flc.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
          AND flc.expires_at > ?2
         WHERE c.source_id = ?1
         ORDER BY
            COALESCE(cus.is_favorite, 0) DESC,
            CASE WHEN co.position IS NOT NULL THEN 0 ELSE 1 END,
            co.position ASC,
            display_name ASC"
    ).map_err(|e| e.to_string())?;

    let channel_iter = stmt.query_map(params![source_id, now_ts], |row| {
        let cached_sha: Option<String> = row.get(6)?;
        let is_fav: i32 = row.get(7)?;
        let is_hid: i32 = row.get(8)?;
        let country_code: String = row.get(9)?;
        let src_name: String = row.get(10)?;
        let icon_failed: i32 = row.get(11)?;
        let cached_icon = cached_sha
            .filter(|s| !s.trim().is_empty())
            .map(|sha| image_protocol_url(&sha));
        Ok(Channel {
            id: row.get(0)?,
            name: row.get(1)?,
            original_name: row.get(2)?,
            custom_name: row.get(3)?,
            icon: row.get(4)?,
            custom_icon: row.get(5)?,
            cached_icon: cached_icon.clone(),
            icon_failed: cached_icon.is_none() && icon_failed == 1,
            is_favorite: is_fav == 1,
            is_hidden: is_hid == 1,
            source_id: Some(source_id.to_string()),
            source_name: Some(src_name),
            country_code: Some(country_code),
        })
    }).map_err(|e| e.to_string())?;

    Ok(channel_iter.flatten().collect())
}

#[tauri::command]
async fn get_source_channels(state: tauri::State<'_, AppState>, source_id: String) -> Result<Vec<Channel>, String> {
    if source_id.trim().is_empty() {
        return Ok(Vec::new());
    }
    let conn = state.db.lock().await;
    if source_id == "__ALL_FAVORITES__" {
        query_all_favorites_channels(&conn)
    } else {
        query_source_channels(&conn, &source_id)
    }
}

async fn get_all_favorites_epg_data(state: &AppState) -> Result<EpgResponse, String> {
    let conn = state.db.lock().await;
    let channels = query_all_favorites_channels(&conn)?;

    let mut stmt = conn.prepare(
        "SELECT p.channel_id, p.source_id, p.start, p.stop, p.title, p.desc, COALESCE(s.time_offset, 0)
         FROM channel_user_settings cus
         JOIN sources s ON s.id = cus.source_id
         JOIN programmes p ON p.source_id = cus.source_id AND p.channel_id = cus.channel_id
         WHERE cus.is_favorite = 1 AND COALESCE(cus.is_hidden, 0) = 0"
    ).map_err(|e| e.to_string())?;

    let prog_iter = stmt.query_map([], |row| {
        let raw_ch_id: String = row.get(0)?;
        let src_id: String = row.get(1)?;
        let composite_id = format!("{}::{}", src_id, raw_ch_id);
        let offset: i32 = row.get(6)?;
        Ok(Programme {
            channel: composite_id,
            start: row.get(2)?,
            stop: row.get(3)?,
            title: row.get(4)?,
            desc: row.get(5)?,
            source_id: Some(src_id),
            time_offset: Some(offset),
        })
    }).map_err(|e| e.to_string())?;

    let mut programmes = Vec::new();
    for p in prog_iter.flatten() { programmes.push(p); }

    Ok(EpgResponse { channels, programmes })
}

#[tauri::command]
async fn get_epg_data(state: tauri::State<'_, AppState>, source_id: String) -> Result<EpgResponse, String> {
    if source_id.trim().is_empty() {
        return Ok(EpgResponse { channels: Vec::new(), programmes: Vec::new() });
    }

    if source_id == "__ALL_FAVORITES__" {
        return get_all_favorites_epg_data(state.inner()).await;
    }

    let conn = state.db.lock().await;
    let channels = query_source_channels(&conn, &source_id)?;

    let mut stmt = conn.prepare("SELECT channel_id, start, stop, title, desc FROM programmes WHERE source_id = ?1").map_err(|e| e.to_string())?;
    let prog_iter = stmt.query_map(params![source_id], |row| {
        Ok(Programme {
            channel: row.get(0)?,
            start: row.get(1)?,
            stop: row.get(2)?,
            title: row.get(3)?,
            desc: row.get(4)?,
            source_id: Some(source_id.clone()),
            time_offset: None,
        })
    }).map_err(|e| e.to_string())?;

    let mut programmes = Vec::new();
    for p in prog_iter.flatten() { programmes.push(p); }

    Ok(EpgResponse { channels, programmes })
}

#[tauri::command]
async fn get_sources(state: tauri::State<'_, AppState>) -> Result<Vec<Source>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.url, s.last_downloaded, s.country_code, s.time_offset, s.sort_position,
                (SELECT COUNT(*) FROM channels c WHERE c.source_id = s.id) AS ch_count,
                (SELECT COUNT(*) FROM channel_user_settings cus WHERE cus.source_id = s.id AND cus.is_favorite = 1) AS fav_count,
                (SELECT COUNT(*) FROM channel_user_settings cus WHERE cus.source_id = s.id AND cus.is_hidden = 1) AS hid_count
         FROM sources s
         ORDER BY COALESCE(s.sort_position, s.rowid) ASC"
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            last_downloaded: row.get(3)?,
            country_code: row.get(4)?,
            time_offset: row.get(5)?,
            sort_position: row.get(6)?,
            channel_count: Some(row.get(7)?),
            favorite_count: Some(row.get(8)?),
            hidden_count: Some(row.get(9)?),
        })
    }).map_err(|e| e.to_string())?;

    let sources: Vec<Source> = rows.flatten().collect();
    Ok(sources)
}

#[tauri::command]
async fn save_source(state: tauri::State<'_, AppState>, source: Source) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO sources (id, name, url, last_downloaded, country_code, time_offset, sort_position) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET 
            name = excluded.name, 
            url = excluded.url, 
            last_downloaded = COALESCE(excluded.last_downloaded, sources.last_downloaded),
            country_code = excluded.country_code,
            time_offset = excluded.time_offset,
            sort_position = COALESCE(excluded.sort_position, sources.sort_position),
            etag = CASE WHEN sources.url != excluded.url THEN NULL ELSE sources.etag END,
            last_modified = CASE WHEN sources.url != excluded.url THEN NULL ELSE sources.last_modified END",
        params![source.id, source.name, source.url, source.last_downloaded, source.country_code, source.time_offset, source.sort_position],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_source_in_conn(conn: &mut Connection, source_id: &str) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM sources WHERE id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channels WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM programmes WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_orders WHERE source_id = ?1", params![source_id]).ok();
    tx.execute(
        "DELETE FROM channel_orders WHERE source_id = '__ALL_FAVORITES__' AND substr(channel_id, 1, length(?1) + 2) = (?1 || '::')",
        params![source_id],
    ).ok();
    tx.execute("DELETE FROM channel_sort_presets WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_user_settings WHERE source_id = ?1", params![source_id]).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn delete_source(state: tauri::State<'_, AppState>, source_id: String) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    delete_source_in_conn(&mut conn, &source_id)
}


#[tauri::command]
async fn save_source_order(state: tauri::State<'_, AppState>, order: Vec<String>) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for (pos, src_id) in order.iter().enumerate() {
        tx.execute(
            "UPDATE sources SET sort_position = ?1 WHERE id = ?2",
            params![pos as i64, src_id],
        ).ok();
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn clear_entire_database(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM channels", []).ok();
    tx.execute("DELETE FROM programmes", []).ok();
    tx.execute("DELETE FROM tmdb_cache", []).ok();
    tx.execute("UPDATE sources SET last_downloaded = NULL, etag = NULL, last_modified = NULL", []).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FavoriteAiring {
    pub channel_id: String,
    pub channel_name: String,
    pub source_id: String,
    pub source_name: String,
    pub country_code: String,
    pub source_sort_pos: i64,
    pub start: String,
    pub stop: String,
    pub start_ts: i64,
    pub stop_ts: i64,
    pub time_offset: i32,
    pub title: String,
    pub desc: String,
}

pub(crate) fn query_favorite_programmes_airings(
    conn: &Connection,
    source_id: Option<&str>,
    include_past: bool,
    now_ts: i64,
) -> Result<Vec<FavoriteAiring>, String> {
    let mut where_clauses = vec!["COALESCE(cus.is_hidden, 0) = 0".to_string()];
    let mut param_values: Vec<rusqlite::types::Value> = Vec::new();

    match source_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some("__ALL_FAVORITES__") => {
            where_clauses.push("COALESCE(cus.is_favorite, 0) = 1".to_string());
        }
        Some(sid) if sid != "__ALL_SOURCES__" => {
            where_clauses.push("p.source_id = ?".to_string());
            param_values.push(rusqlite::types::Value::Text(sid.to_string()));
        }
        _ => {}
    }

    if !include_past {
        where_clauses.push(
            "(COALESCE(p.stop_ts, p.start_ts, 0) + COALESCE(s.time_offset, 0) * 3600) >= ?"
                .to_string(),
        );
        param_values.push(rusqlite::types::Value::Integer(now_ts));
    }

    let sql = format!(
        "WITH ranked AS (
            SELECT p.channel_id,
                   COALESCE(NULLIF(cus.custom_name, ''), c.name, p.channel_id) AS channel_name,
                   p.source_id,
                   COALESCE(s.name, p.source_id) AS source_name,
                   COALESCE(s.country_code, 'HU') AS country_code,
                   COALESCE(s.sort_position, s.rowid) AS source_sort_pos,
                   p.start,
                   p.stop,
                   COALESCE(p.start_ts, 0) AS start_ts,
                   COALESCE(p.stop_ts, p.start_ts, 0) AS stop_ts,
                   COALESCE(s.time_offset, 0) AS time_offset,
                   p.title,
                   COALESCE(p.desc, '') AS desc,
                   ROW_NUMBER() OVER (
                       PARTITION BY p.source_id, p.channel_id, COALESCE(CAST(p.start_ts AS TEXT), p.start), p.title
                       ORDER BY
                           CASE WHEN p.stop_ts IS NOT NULL AND p.start_ts IS NOT NULL AND p.stop_ts > p.start_ts THEN 0 ELSE 1 END,
                           LENGTH(COALESCE(p.desc, '')) DESC,
                           p.id ASC
                   ) AS rn
            FROM programmes p
            JOIN favorites f ON f.title = p.title
            JOIN sources s ON s.id = p.source_id
            LEFT JOIN channels c ON c.id = p.channel_id AND c.source_id = p.source_id
            LEFT JOIN channel_user_settings cus ON cus.source_id = p.source_id AND cus.channel_id = p.channel_id
            WHERE {}
        )
        SELECT channel_id, channel_name, source_id, source_name, country_code, source_sort_pos,
               start, stop, start_ts, stop_ts, time_offset, title, desc
        FROM ranked
        WHERE rn = 1
        ORDER BY (start_ts + time_offset * 3600) ASC, title ASC",
        where_clauses.join(" AND ")
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_values), |r| {
            Ok(FavoriteAiring {
                channel_id: r.get(0)?,
                channel_name: r.get(1)?,
                source_id: r.get(2)?,
                source_name: r.get(3)?,
                country_code: r.get(4)?,
                source_sort_pos: r.get(5)?,
                start: r.get(6)?,
                stop: r.get(7)?,
                start_ts: r.get(8)?,
                stop_ts: r.get(9)?,
                time_offset: r.get(10)?,
                title: r.get(11)?,
                desc: r.get(12)?,
            })
        })
        .map_err(|e| e.to_string())?;

    Ok(rows.flatten().collect())
}

#[tauri::command]
async fn get_favorite_programmes_airings(
    state: tauri::State<'_, AppState>,
    source_id: Option<String>,
    include_past: Option<bool>,
) -> Result<Vec<FavoriteAiring>, String> {
    let conn = state.db.lock().await;
    query_favorite_programmes_airings(
        &conn,
        source_id.as_deref(),
        include_past.unwrap_or(false),
        trakt::now_epoch_secs(),
    )
}

#[tauri::command]
async fn get_favorites(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT title FROM favorites ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut favs = Vec::new();
    for title in rows.flatten() { favs.push(title); }
    Ok(favs)
}

#[tauri::command]
async fn toggle_favorite(state: tauri::State<'_, AppState>, title: String) -> Result<bool, String> {
    let conn = state.db.lock().await;
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM favorites WHERE title = ?1)",
        params![title],
        |row| row.get(0),
    ).unwrap_or(false);

    if exists {
        conn.execute("DELETE FROM favorites WHERE title = ?1", params![title]).map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        conn.execute("INSERT OR REPLACE INTO favorites (title) VALUES (?1)", params![title]).map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
async fn save_channel_order(state: tauri::State<'_, AppState>, source_id: String, order: Vec<String>) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM channel_orders WHERE source_id = ?1", params![source_id]).ok();
    for (pos, ch_id) in order.iter().enumerate() {
        tx.execute(
            "INSERT INTO channel_orders (source_id, channel_id, position) VALUES (?1, ?2, ?3)",
            params![source_id, ch_id, pos as i64],
        ).ok();
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct SortPreset {
    id: i64,
    name: String,
    source_id: String,
    channel_order: Vec<String>,
    created_at: Option<String>,
}

#[tauri::command]
async fn get_sort_presets(state: tauri::State<'_, AppState>, source_id: String) -> Result<Vec<SortPreset>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare(
        "SELECT id, name, source_id, channel_order, created_at FROM channel_sort_presets WHERE source_id = ?1 ORDER BY id ASC"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![source_id], |row| {
        let order_str: String = row.get(3)?;
        let channel_order: Vec<String> = serde_json::from_str(&order_str).unwrap_or_default();
        Ok(SortPreset {
            id: row.get(0)?,
            name: row.get(1)?,
            source_id: row.get(2)?,
            channel_order,
            created_at: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;
    let mut presets = Vec::new();
    for item in rows.flatten() { presets.push(item); }
    Ok(presets)
}

#[tauri::command]
async fn save_sort_preset(state: tauri::State<'_, AppState>, source_id: String, name: String, order: Vec<String>) -> Result<SortPreset, String> {
    let conn = state.db.lock().await;
    let order_json = serde_json::to_string(&order).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO channel_sort_presets (name, source_id, channel_order, created_at) VALUES (?1, ?2, ?3, datetime('now', 'localtime'))",
        params![name, source_id, order_json],
    ).map_err(|e| e.to_string())?;
    let id = conn.last_insert_rowid();
    let created_at: Option<String> = conn.query_row("SELECT created_at FROM channel_sort_presets WHERE id = ?1", params![id], |r| r.get(0)).ok();
    Ok(SortPreset {
        id,
        name,
        source_id,
        channel_order: order,
        created_at,
    })
}

#[tauri::command]
async fn rename_sort_preset(state: tauri::State<'_, AppState>, preset_id: i64, new_name: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute(
        "UPDATE channel_sort_presets SET name = ?1 WHERE id = ?2",
        params![new_name, preset_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn delete_sort_preset(state: tauri::State<'_, AppState>, preset_id: i64) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute("DELETE FROM channel_sort_presets WHERE id = ?1", params![preset_id]).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn toggle_channel_favorite(state: tauri::State<'_, AppState>, source_id: String, channel_id: String) -> Result<bool, String> {
    let conn = state.db.lock().await;
    let current_fav: bool = conn.query_row(
        "SELECT COALESCE(is_favorite, 0) = 1 FROM channel_user_settings WHERE source_id = ?1 AND channel_id = ?2",
        params![source_id, channel_id],
        |row| row.get(0),
    ).unwrap_or(false);

    let new_fav = !current_fav;
    let val = if new_fav { 1 } else { 0 };
    conn.execute(
        "INSERT INTO channel_user_settings (source_id, channel_id, is_favorite) VALUES (?1, ?2, ?3)
         ON CONFLICT(source_id, channel_id) DO UPDATE SET is_favorite = excluded.is_favorite",
        params![source_id, channel_id, val],
    ).map_err(|e| e.to_string())?;
    Ok(new_fav)
}

#[tauri::command]
async fn toggle_channel_hidden(state: tauri::State<'_, AppState>, source_id: String, channel_id: String) -> Result<bool, String> {
    let conn = state.db.lock().await;
    let current_hidden: bool = conn.query_row(
        "SELECT COALESCE(is_hidden, 0) = 1 FROM channel_user_settings WHERE source_id = ?1 AND channel_id = ?2",
        params![source_id, channel_id],
        |row| row.get(0),
    ).unwrap_or(false);

    let new_hidden = !current_hidden;
    let val = if new_hidden { 1 } else { 0 };
    conn.execute(
        "INSERT INTO channel_user_settings (source_id, channel_id, is_hidden) VALUES (?1, ?2, ?3)
         ON CONFLICT(source_id, channel_id) DO UPDATE SET is_hidden = excluded.is_hidden",
        params![source_id, channel_id, val],
    ).map_err(|e| e.to_string())?;
    Ok(new_hidden)
}

#[tauri::command]
async fn set_channel_logo(state: tauri::State<'_, AppState>, source_id: String, channel_id: String, logo_url: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    let clean_url = logo_url.trim();
    let custom_icon = if clean_url.is_empty() { None } else { Some(clean_url.to_string()) };
    conn.execute(
        "INSERT INTO channel_user_settings (source_id, channel_id, custom_icon) VALUES (?1, ?2, ?3)
         ON CONFLICT(source_id, channel_id) DO UPDATE SET custom_icon = excluded.custom_icon",
        params![source_id, channel_id, custom_icon],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn rename_channel(state: tauri::State<'_, AppState>, source_id: String, channel_id: String, name: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    let clean_name = name.trim();
    let custom_name = if clean_name.is_empty() { None } else { Some(clean_name.to_string()) };
    conn.execute(
        "INSERT INTO channel_user_settings (source_id, channel_id, custom_name) VALUES (?1, ?2, ?3)
         ON CONFLICT(source_id, channel_id) DO UPDATE SET custom_name = excluded.custom_name",
        params![source_id, channel_id, custom_name],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

/// Converts an `images_db.images.data` value (legacy base64 `TEXT` data URI or raw `BLOB`) into a
/// `data:<mime>;base64,...` URI for the frontend.
#[cfg(test)]
pub(crate) fn image_row_to_data_uri(
    mime: Option<&str>,
    val: rusqlite::types::ValueRef<'_>,
) -> Option<String> {
    match val {
        rusqlite::types::ValueRef::Text(bytes) => {
            let s = std::str::from_utf8(bytes).ok()?;
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
        rusqlite::types::ValueRef::Blob(bytes) => {
            if bytes.is_empty() {
                return None;
            }
            use base64::Engine;
            let m = mime.filter(|s| !s.trim().is_empty()).unwrap_or("image/png");
            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
            Some(format!("data:{};base64,{}", m, b64))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn read_cached_image_data_uri(conn: &Connection, url: &str) -> Option<String> {
    conn.query_row(
        "SELECT img.mime_type, img.data FROM images_db.image_urls iu
         JOIN images_db.images img ON img.id = iu.image_id
         WHERE iu.url = ?1",
        params![url],
        |row| {
            let mime: Option<String> = row.get(0)?;
            let data = row
                .get_ref(1)
                .ok()
                .and_then(|v| image_row_to_data_uri(mime.as_deref(), v));
            Ok(data)
        },
    )
    .ok()
    .flatten()
}

/// Converts legacy base64 `TEXT` rows in `images_db.images` to raw `BLOB` storage (~25% smaller).
/// Safe to run repeatedly; rows already stored as `BLOB` are untouched.
pub(crate) fn convert_legacy_text_images_to_blobs(conn: &Connection) -> usize {
    let rows: Vec<(i64, String)> = {
        let Ok(mut stmt) = conn.prepare(
            "SELECT id, data FROM images_db.images
             WHERE typeof(data) = 'text' AND data LIKE 'data:%base64,%'",
        ) else {
            return 0;
        };
        let Ok(mapped) = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))) else {
            return 0;
        };
        mapped.flatten().collect()
    };

    if rows.is_empty() {
        return 0;
    }

    use base64::Engine;
    let Ok(mut upd) = conn.prepare(
        "UPDATE images_db.images SET data = ?1, byte_size = ?2 WHERE id = ?3",
    ) else {
        return 0;
    };

    let mut converted = 0usize;
    for (id, data_uri) in rows {
        let Some((_, b64_part)) = data_uri.split_once("base64,") else {
            continue;
        };
        let cleaned: String = b64_part.chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(&cleaned) {
            if !raw.is_empty()
                && upd
                    .execute(params![raw.as_slice(), raw.len() as i64, id])
                    .is_ok()
            {
                converted += 1;
            }
        }
    }
    converted
}

pub(crate) fn image_protocol_url(sha256: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("http://epgimg.localhost/{}", sha256)
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!("epgimg://localhost/{}", sha256)
    }
}

pub(crate) fn read_cached_image_url(conn: &Connection, url: &str) -> Option<String> {
    conn.query_row(
        "SELECT img.sha256 FROM images_db.image_urls iu
         JOIN images_db.images img ON img.id = iu.image_id
         WHERE iu.url = ?1",
        params![url],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .filter(|s| !s.trim().is_empty())
    .map(|sha| image_protocol_url(&sha))
}

pub(crate) fn read_image_bytes_by_sha256(conn: &Connection, sha256: &str) -> Option<(String, Vec<u8>)> {
    conn.query_row(
        "SELECT mime_type, data FROM images WHERE sha256 = ?1",
        params![sha256],
        |row| {
            let mime: String = row
                .get::<_, Option<String>>(0)?
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "image/png".to_string());
            let bytes = match row.get_ref(1)? {
                rusqlite::types::ValueRef::Blob(b) => Some(b.to_vec()),
                rusqlite::types::ValueRef::Text(t) => {
                    let s = std::str::from_utf8(t).ok();
                    s.and_then(|uri| uri.split_once("base64,"))
                        .and_then(|(_, b64)| {
                            use base64::Engine;
                            let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                            base64::engine::general_purpose::STANDARD.decode(&cleaned).ok()
                        })
                }
                _ => None,
            };
            Ok(bytes.map(|b| (mime, b)))
        },
    )
    .ok()
    .flatten()
}

#[tauri::command]
async fn get_cached_logo(state: tauri::State<'_, AppState>, url: String) -> Result<Option<String>, String> {
    let clean_url = url.trim();
    if clean_url.is_empty() {
        return Ok(None);
    }
    let conn = state.db.lock().await;
    Ok(read_cached_image_url(&conn, clean_url))
}

const MAX_CACHED_IMAGE_BYTES: usize = 15 * 1024 * 1024;

fn looks_like_svg(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(1024)];
    let Ok(text) = std::str::from_utf8(sample) else {
        return false;
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("<svg") {
        return true;
    }
    if (lower.starts_with("<?xml") || lower.starts_with("<!--") || lower.starts_with("<!doctype svg"))
        && lower.contains("<svg")
    {
        return true;
    }
    false
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(512)];
    let Ok(text) = std::str::from_utf8(sample) else {
        return false;
    };
    let trimmed = text.trim_start_matches('\u{feff}').trim_start().to_ascii_lowercase();
    trimmed.starts_with("<!doctype html") || trimmed.starts_with("<html") || trimmed.starts_with("<head") || trimmed.starts_with("<body")
}

pub(crate) fn detect_and_validate_image_mime(
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<String, String> {
    if bytes.is_empty() {
        return Err("Üres válasz az ikonkérésre".to_string());
    }
    if bytes.len() > MAX_CACHED_IMAGE_BYTES {
        return Err(format!(
            "A kép túl nagy ({} bájt, max {} bájt)",
            bytes.len(),
            MAX_CACHED_IMAGE_BYTES
        ));
    }

    // 1. Magic-byte / SVG detection takes priority (also handles CDNs sending application/octet-stream)
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Ok("image/png".to_string());
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok("image/jpeg".to_string());
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Ok("image/gif".to_string());
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok("image/webp".to_string());
    }
    if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        return Ok("image/x-icon".to_string());
    }
    if bytes.starts_with(b"BM") {
        return Ok("image/bmp".to_string());
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"avif" || brand == b"avis" {
            return Ok("image/avif".to_string());
        }
    }
    if looks_like_svg(bytes) {
        return Ok("image/svg+xml".to_string());
    }

    // 2. Fallback to Content-Type only when it explicitly declares an image/* MIME and isn't HTML
    if let Some(ct) = content_type {
        let mime_part = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if mime_part.starts_with("image/") && !looks_like_html(bytes) {
            if mime_part.contains("svg") {
                return Ok("image/svg+xml".to_string());
            }
            return Ok(mime_part);
        }
    }

    Err("A válasz nem érvényes képformátum".to_string())
}

const FAILED_LOGO_PERMANENT_TTL_SECS: i64 = 14 * 86_400;
const FAILED_LOGO_TRANSIENT_TTL_SECS: i64 = 6 * 3600;

fn read_negative_logo_cache(conn: &Connection, url: &str) -> Option<String> {
    let now_ts = trakt::now_epoch_secs();
    conn.query_row(
        "SELECT error FROM images_db.failed_logo_cache WHERE url = ?1 AND expires_at > ?2",
        params![url, now_ts],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

fn write_negative_logo_cache(
    conn: &Connection,
    url: &str,
    status_code: Option<i64>,
    error: &str,
    ttl_secs: i64,
) {
    let now_ts = trakt::now_epoch_secs();
    let expires_at = now_ts + ttl_secs;
    let _ = conn.execute(
        "INSERT INTO images_db.failed_logo_cache (url, status_code, error, failed_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(url) DO UPDATE SET
            status_code = excluded.status_code,
            error = excluded.error,
            failed_at = excluded.failed_at,
            expires_at = excluded.expires_at",
        params![url, status_code, error, now_ts, expires_at],
    );
}

fn clear_negative_logo_cache_entry(conn: &Connection, url: &str) {
    let _ = conn.execute(
        "DELETE FROM images_db.failed_logo_cache WHERE url = ?1",
        params![url],
    );
}

pub(crate) async fn cache_logo_inner(state: &AppState, url: String) -> Result<String, String> {
    cache_logo_with_type_inner(state, url, None).await
}

async fn cache_logo_with_type_inner(
    state: &AppState,
    url: String,
    image_type_override: Option<&str>,
) -> Result<String, String> {
    let clean_url = url.trim().to_string();
    if clean_url.is_empty() {
        return Err("Üres URL".to_string());
    }
    if clean_url.starts_with("data:")
        || clean_url.starts_with("http://epgimg.localhost/")
        || clean_url.starts_with("epgimg://")
    {
        return Ok(clean_url);
    }

    let resolved_override: Option<&'static str> =
        match image_type_override.map(str::trim).filter(|s| !s.is_empty()) {
            Some("avatar") => Some("avatar"),
            Some("poster") => Some("poster"),
            Some("logo") => Some("logo"),
            _ => None,
        };

    // 1. Előzetes gyorsítótár + negatív cache ellenőrzés (a zár azonnal feloldódik)
    {
        let conn = state.db.lock().await;
        if let Some(proto_url) = read_cached_image_url(&conn, &clean_url) {
            if let Some(target_type) = resolved_override {
                let _ = conn.execute(
                    "UPDATE images_db.image_urls SET image_type = ?1 WHERE url = ?2 AND image_type != ?1",
                    params![target_type, clean_url],
                );
            }
            return Ok(proto_url);
        }
        if let Some(cached_err) = read_negative_logo_cache(&conn, &clean_url) {
            return Err(cached_err);
        }
    }

    if !clean_url.starts_with("http://") && !clean_url.starts_with("https://") {
        return Err("Nem támogatott URL protokoll".to_string());
    }

    // 2. Single-flight deduplikáció: párhuzamos azonos URL kérések egyetlen HTTP letöltésen osztoznak
    let flight_url = clean_url.clone();
    trakt::client::single_flight_shared(&state.image_inflight, &flight_url, || async move {
        // Újraellenőrzés a hálózati hívás előtt (hátha épp most került cache-be)
        {
            let conn = state.db.lock().await;
            if let Some(proto_url) = read_cached_image_url(&conn, &clean_url) {
                return Ok(proto_url);
            }
            if let Some(cached_err) = read_negative_logo_cache(&conn, &clean_url) {
                return Err(cached_err);
            }
        }

        let resp = match state.http.get(&clean_url).send().await {
            Ok(r) => r,
            Err(e) => {
                let err_msg = e.to_string();
                let conn = state.db.lock().await;
                write_negative_logo_cache(&conn, &clean_url, None, &err_msg, FAILED_LOGO_TRANSIENT_TTL_SECS);
                return Err(err_msg);
            }
        };

        if !resp.status().is_success() {
            let code = resp.status().as_u16() as i64;
            let err_msg = format!("HTTP hiba: {}", resp.status());
            let ttl = if matches!(code, 403 | 404 | 410 | 451) {
                FAILED_LOGO_PERMANENT_TTL_SECS
            } else {
                FAILED_LOGO_TRANSIENT_TTL_SECS
            };
            let conn = state.db.lock().await;
            write_negative_logo_cache(&conn, &clean_url, Some(code), &err_msg, ttl);
            return Err(err_msg);
        }

        if let Some(len) = resp.content_length() {
            if len > MAX_CACHED_IMAGE_BYTES as u64 {
                let err_msg = format!(
                    "A kép túl nagy ({} bájt, max {} bájt)",
                    len, MAX_CACHED_IMAGE_BYTES
                );
                let conn = state.db.lock().await;
                write_negative_logo_cache(&conn, &clean_url, Some(200), &err_msg, FAILED_LOGO_PERMANENT_TTL_SECS);
                return Err(err_msg);
            }
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                let err_msg = e.to_string();
                let conn = state.db.lock().await;
                write_negative_logo_cache(&conn, &clean_url, None, &err_msg, FAILED_LOGO_TRANSIENT_TTL_SECS);
                return Err(err_msg);
            }
        };

        let mime = match detect_and_validate_image_mime(content_type.as_deref(), &bytes) {
            Ok(m) => m,
            Err(err_msg) => {
                let conn = state.db.lock().await;
                write_negative_logo_cache(&conn, &clean_url, Some(200), &err_msg, FAILED_LOGO_PERMANENT_TTL_SECS);
                return Err(err_msg);
            }
        };

        // 3. Tartalom-alapú SHA-256 hash generálás a nyers bájtokból
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash_hex = format!("{:x}", hasher.finalize());

        let byte_size = bytes.len() as i64;
        let img_type = match resolved_override {
            Some(t) => t,
            None if clean_url.contains("image.tmdb.org") => "poster",
            None => "logo",
        };

        let conn = state.db.lock().await;

        // 4. Deduplikáció: ha a hash már szerepel az images.db-ben, újrahasznosítjuk az ID-t
        let existing_id: Option<i64> = conn.query_row(
            "SELECT id FROM images_db.images WHERE sha256 = ?1",
            params![hash_hex],
            |row| row.get(0),
        ).ok();

        let image_id = match existing_id {
            Some(id) => id,
            None => {
                conn.execute(
                    "INSERT INTO images_db.images (sha256, mime_type, byte_size, data, created_at)
                     VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                    params![hash_hex, mime, byte_size, bytes.as_ref()],
                ).map_err(|e| e.to_string())?;
                conn.last_insert_rowid()
            }
        };

        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(url) DO UPDATE SET image_id = excluded.image_id, image_type = excluded.image_type, created_at = excluded.created_at",
            params![clean_url, image_id, img_type],
        ).map_err(|e| e.to_string())?;

        clear_negative_logo_cache_entry(&conn, &clean_url);

        Ok(image_protocol_url(&hash_hex))
    })
    .await
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct DuplicateLogoGroup {
    pub image_id: i64,
    pub sha256: String,
    pub sample_data: String,
    pub mime_type: String,
    pub byte_size: i64,
    pub urls: Vec<String>,
    pub channels: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct DuplicateReport {
    pub total_unique_images: usize,
    pub total_urls: usize,
    pub duplicate_groups: Vec<DuplicateLogoGroup>,
    pub redundant_count: usize,
    pub potential_saved_bytes: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct DeduplicationResult {
    pub cleaned_groups: usize,
    pub removed_urls: usize,
    pub removed_images: usize,
    pub freed_bytes: i64,
}

fn load_active_protected_image_urls(conn: &Connection) -> std::collections::HashSet<String> {
    let mut protected = std::collections::HashSet::new();
    if let Some(profile) = trakt::auth::load_cached_profile(conn) {
        if let Some(avatar) = profile.avatar_url {
            let trimmed = avatar.trim();
            if !trimmed.is_empty() {
                protected.insert(trimmed.to_string());
            }
        }
    }
    protected
}

fn find_duplicate_logos_inner(conn: &Connection) -> Result<DuplicateReport, String> {
    let protected_urls = load_active_protected_image_urls(conn);

    let total_unique_images: usize = conn.query_row(
        "SELECT COUNT(*) FROM images_db.images WHERE id IN (SELECT DISTINCT image_id FROM images_db.image_urls WHERE image_type = 'logo')",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let total_urls: usize = conn.query_row(
        "SELECT COUNT(*) FROM images_db.image_urls WHERE image_type = 'logo'",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let mut active_channel_icons: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut stmt = conn
            .prepare("SELECT DISTINCT icon FROM channels WHERE icon IS NOT NULL AND icon != ''")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        for u in rows.flatten() {
            active_channel_icons.insert(u);
        }
    }

    let mut active_custom_icons: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut stmt = conn
            .prepare("SELECT DISTINCT custom_icon FROM channel_user_settings WHERE custom_icon IS NOT NULL AND custom_icon != ''")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        for u in rows.flatten() {
            active_custom_icons.insert(u);
        }
    }

    let mut poster_image_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    {
        let mut stmt = conn
            .prepare("SELECT DISTINCT image_id FROM images_db.image_urls WHERE image_type != 'logo'")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string())?;
        for id in rows.flatten() {
            poster_image_ids.insert(id);
        }
    }

    let mut stmt = conn.prepare(
        "SELECT DISTINCT img.id, img.sha256, img.mime_type, img.byte_size
         FROM images_db.image_urls iu
         JOIN images_db.images img ON img.id = iu.image_id
         WHERE iu.image_type = 'logo'
         ORDER BY img.byte_size DESC, img.id ASC"
    ).map_err(|e| e.to_string())?;

    let group_rows: Vec<(i64, String, String, i64, String)> = {
        let mapped = stmt.query_map([], |row| {
            let sha256: String = row.get(1)?;
            let mime: String = row.get(2)?;
            let sample_url = image_protocol_url(&sha256);
            Ok((row.get(0)?, sha256, mime, row.get(3)?, sample_url))
        }).map_err(|e| e.to_string())?;
        mapped.flatten().collect()
    };
    drop(stmt);

    let mut duplicate_groups = Vec::new();
    let mut redundant_count = 0usize;
    let mut potential_saved_bytes = 0i64;

    let mut url_stmt = conn.prepare(
        "SELECT url FROM images_db.image_urls WHERE image_id = ?1 AND image_type = 'logo' ORDER BY length(url) ASC, url ASC"
    ).map_err(|e| e.to_string())?;

    let mut ch_stmt = conn.prepare(
        "SELECT DISTINCT COALESCE(NULLIF(cus.custom_name, ''), c.name) AS ch_name
         FROM channels c
         LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         WHERE COALESCE(NULLIF(cus.custom_icon, ''), c.icon) = ?1"
    ).map_err(|e| e.to_string())?;

    for (image_id, sha256, mime_type, byte_size, data) in group_rows {
        let urls: Vec<String> = url_stmt.query_map(params![image_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();

        if urls.is_empty() {
            continue;
        }

        let any_referenced = urls.iter().any(|u| {
            active_channel_icons.contains(u)
                || active_custom_icons.contains(u)
                || protected_urls.contains(u)
        });

        let (removable_urls, image_will_be_deleted): (Vec<&String>, bool) = if !any_referenced {
            (urls.iter().collect(), !poster_image_ids.contains(&image_id))
        } else if urls.len() > 1 {
            // Prefer an active channels.icon or protected avatar URL as canonical so it is never deleted
            let canonical = urls
                .iter()
                .find(|u| active_channel_icons.contains(*u) || protected_urls.contains(*u))
                .unwrap_or(&urls[0]);
            let rem: Vec<&String> = urls
                .iter()
                .filter(|u| {
                    *u != canonical
                        && !active_channel_icons.contains(*u)
                        && !protected_urls.contains(*u)
                })
                .collect();
            (rem, false)
        } else {
            (Vec::new(), false)
        };

        if removable_urls.is_empty() {
            continue;
        }

        let mut ch_names = Vec::new();
        for u in &urls {
            let names: Vec<String> = ch_stmt.query_map(params![u], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .flatten()
                .collect();
            ch_names.extend(names);
        }
        ch_names.sort();
        ch_names.dedup();

        redundant_count += removable_urls.len();
        let url_bytes: i64 = removable_urls.iter().map(|u| u.len() as i64).sum();
        potential_saved_bytes += url_bytes + if image_will_be_deleted { byte_size } else { 0 };

        duplicate_groups.push(DuplicateLogoGroup {
            image_id,
            sha256,
            sample_data: data,
            mime_type,
            byte_size,
            urls,
            channels: ch_names,
        });
    }

    Ok(DuplicateReport {
        total_unique_images,
        total_urls,
        duplicate_groups,
        redundant_count,
        potential_saved_bytes,
    })
}

fn deduplicate_logos_inner(conn: &Connection) -> Result<DeduplicationResult, String> {
    let protected_urls = load_active_protected_image_urls(conn);

    // Reclassify any legacy Trakt avatar rows stored as 'logo' to 'avatar'
    for avatar_url in &protected_urls {
        let _ = conn.execute(
            "UPDATE images_db.image_urls SET image_type = 'avatar' WHERE url = ?1 AND image_type = 'logo'",
            params![avatar_url],
        );
    }

    let mut active_channel_icons: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut stmt = conn
            .prepare("SELECT DISTINCT icon FROM channels WHERE icon IS NOT NULL AND icon != ''")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        for u in rows.flatten() {
            active_channel_icons.insert(u);
        }
    }

    let mut active_custom_icons: std::collections::HashSet<String> = std::collections::HashSet::new();
    {
        let mut stmt = conn
            .prepare("SELECT DISTINCT custom_icon FROM channel_user_settings WHERE custom_icon IS NOT NULL AND custom_icon != ''")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
        for u in rows.flatten() {
            active_custom_icons.insert(u);
        }
    }

    let image_ids: Vec<i64> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT image_id FROM images_db.image_urls WHERE image_type = 'logo'")
            .map_err(|e| e.to_string())?;
        let mapped = stmt.query_map([], |row| row.get(0)).map_err(|e| e.to_string())?;
        mapped.flatten().collect()
    };

    let mut cleaned_groups = 0usize;
    let mut removed_urls = 0usize;
    let mut freed_url_bytes = 0i64;

    if !image_ids.is_empty() {
        conn.execute_batch("BEGIN TRANSACTION;").map_err(|e| e.to_string())?;

        let res: Result<(), String> = (|| {
            let mut url_stmt = conn.prepare(
                "SELECT url FROM images_db.image_urls WHERE image_id = ?1 AND image_type = 'logo' ORDER BY length(url) ASC, url ASC"
            ).map_err(|e| e.to_string())?;

            let mut update_settings_stmt = conn.prepare(
                "UPDATE channel_user_settings SET custom_icon = ?1 WHERE custom_icon = ?2"
            ).map_err(|e| e.to_string())?;

            let mut delete_url_stmt = conn.prepare(
                "DELETE FROM images_db.image_urls WHERE url = ?1 AND image_type = 'logo'"
            ).map_err(|e| e.to_string())?;

            for image_id in image_ids {
                let urls: Vec<String> = url_stmt.query_map(params![image_id], |r| r.get(0))
                    .map_err(|e| e.to_string())?
                    .flatten()
                    .collect();

                if urls.is_empty() {
                    continue;
                }

                let any_referenced = urls.iter().any(|u| {
                    active_channel_icons.contains(u)
                        || active_custom_icons.contains(u)
                        || protected_urls.contains(u)
                });

                if !any_referenced {
                    // Orphaned logo image_urls: none referenced by channels.icon or custom_icon
                    for orphan_url in &urls {
                        let deleted = delete_url_stmt.execute(params![orphan_url]).map_err(|e| e.to_string())?;
                        if deleted > 0 {
                            removed_urls += deleted;
                            freed_url_bytes += orphan_url.len() as i64;
                        }
                    }
                    cleaned_groups += 1;
                    continue;
                }

                if urls.len() <= 1 {
                    continue;
                }

                // Prefer an active channels.icon or protected avatar URL as canonical so it is never broken
                let canonical_url = urls
                    .iter()
                    .find(|u| active_channel_icons.contains(*u) || protected_urls.contains(*u))
                    .unwrap_or(&urls[0]);

                let mut group_changed = false;
                for red_url in &urls {
                    if red_url == canonical_url || protected_urls.contains(red_url) {
                        continue;
                    }
                    if active_custom_icons.contains(red_url) {
                        update_settings_stmt
                            .execute(params![canonical_url, red_url])
                            .map_err(|e| e.to_string())?;
                        group_changed = true;
                    }
                    // Never delete URLs actively referenced by channels.icon
                    if !active_channel_icons.contains(red_url) {
                        let deleted = delete_url_stmt.execute(params![red_url]).map_err(|e| e.to_string())?;
                        if deleted > 0 {
                            removed_urls += deleted;
                            freed_url_bytes += red_url.len() as i64;
                            group_changed = true;
                        }
                    }
                }
                if group_changed {
                    cleaned_groups += 1;
                }
            }

            drop(url_stmt);
            drop(update_settings_stmt);
            drop(delete_url_stmt);

            conn.execute_batch("COMMIT;").map_err(|e| e.to_string())?;
            Ok(())
        })();

        if let Err(e) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(e);
        }
    }

    let freed_image_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(byte_size), 0) FROM images_db.images WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let removed_images = conn.execute(
        "DELETE FROM images_db.images WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
        [],
    ).unwrap_or(0);

    let _ = conn.execute("VACUUM images_db", []);

    Ok(DeduplicationResult {
        cleaned_groups,
        removed_urls,
        removed_images,
        freed_bytes: freed_url_bytes + freed_image_bytes,
    })
}

#[tauri::command]
async fn find_duplicate_logos(state: tauri::State<'_, AppState>) -> Result<DuplicateReport, String> {
    let conn = state.db.lock().await;
    find_duplicate_logos_inner(&conn)
}

#[tauri::command]
async fn deduplicate_logos(state: tauri::State<'_, AppState>) -> Result<DeduplicationResult, String> {
    let conn = state.db.lock().await;
    deduplicate_logos_inner(&conn)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub logo_count: usize,
    pub logo_bytes: i64,
    pub poster_count: usize,
    pub poster_bytes: i64,
    pub total_bytes: i64,
}

fn get_image_cache_stats_from_conn(conn: &Connection) -> CacheStats {
    let logo_count: usize = conn.query_row(
        "SELECT COUNT(DISTINCT url)
         FROM images_db.image_urls
         WHERE image_type = 'logo' AND url NOT LIKE '%image.tmdb.org%'",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let logo_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(byte_size), 0)
         FROM images_db.images
         WHERE id IN (
             SELECT DISTINCT image_id FROM images_db.image_urls
             WHERE image_type = 'logo' AND url NOT LIKE '%image.tmdb.org%'
         )",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let poster_count: usize = conn.query_row(
        "SELECT COUNT(DISTINCT url)
         FROM images_db.image_urls
         WHERE image_type = 'poster' OR url LIKE '%image.tmdb.org%'",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let poster_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(byte_size), 0)
         FROM images_db.images
         WHERE id IN (
             SELECT DISTINCT image_id FROM images_db.image_urls
             WHERE image_type = 'poster' OR url LIKE '%image.tmdb.org%'
         )",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    let total_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(byte_size), 0) FROM images_db.images",
        [],
        |r| r.get(0),
    ).unwrap_or(0);

    CacheStats {
        logo_count,
        logo_bytes,
        poster_count,
        poster_bytes,
        total_bytes,
    }
}

#[tauri::command]
async fn get_image_cache_stats(state: tauri::State<'_, AppState>) -> Result<CacheStats, String> {
    let conn = state.db.lock().await;
    Ok(get_image_cache_stats_from_conn(&conn))
}

#[tauri::command]
async fn clear_logo_cache(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let conn = state.db.lock().await;
    let count = conn.execute(
        "DELETE FROM images_db.image_urls WHERE image_type = 'logo' AND url NOT LIKE '%image.tmdb.org%'",
        [],
    ).map_err(|e| e.to_string())?;

    let _ = conn.execute(
        "DELETE FROM images_db.failed_logo_cache WHERE url NOT LIKE '%image.tmdb.org%'",
        [],
    );
    let _ = conn.execute(
        "DELETE FROM images_db.images WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
        [],
    );
    let _ = conn.execute("VACUUM images_db", []);
    Ok(count)
}

#[tauri::command]
async fn clear_poster_cache(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let conn = state.db.lock().await;
    let count = conn.execute(
        "DELETE FROM images_db.image_urls WHERE image_type = 'poster' OR url LIKE '%image.tmdb.org%'",
        [],
    ).map_err(|e| e.to_string())?;

    let _ = conn.execute(
        "DELETE FROM images_db.failed_logo_cache WHERE url LIKE '%image.tmdb.org%'",
        [],
    );
    let _ = conn.execute(
        "DELETE FROM images_db.images WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
        [],
    );
    let _ = conn.execute("VACUUM images_db", []);
    Ok(count)
}

#[tauri::command]
async fn clear_all_image_cache(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().await;
    let _ = conn.execute("DELETE FROM images_db.image_urls", []);
    let _ = conn.execute("DELETE FROM images_db.images", []);
    let _ = conn.execute("DELETE FROM images_db.failed_logo_cache", []);
    let _ = conn.execute("VACUUM images_db", []);
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStats {
    pub epg_db_bytes: u64,
    pub images_db_bytes: u64,
    pub total_db_bytes: u64,
    pub freelist_bytes: u64,
    pub expired_http_cache_rows: usize,
    pub expired_tmdb_negative_rows: usize,
    pub orphaned_image_rows: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceResult {
    pub pruned_http_cache: usize,
    pub pruned_tmdb_negatives: usize,
    pub pruned_orphan_images: usize,
    pub converted_blob_images: usize,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub freed_bytes: u64,
}

fn is_images_db_attached(conn: &Connection) -> bool {
    let Ok(mut stmt) = conn.prepare("PRAGMA database_list") else {
        return false;
    };
    let Ok(mut rows) = stmt.query([]) else {
        return false;
    };
    while let Ok(Some(row)) = rows.next() {
        if row.get::<_, String>(1).ok().as_deref() == Some("images_db") {
            return true;
        }
    }
    false
}

fn sqlite_db_bytes(conn: &Connection, schema: &str, file_path: Option<&std::path::Path>) -> u64 {
    if let Some(p) = file_path {
        if let Ok(meta) = fs::metadata(p) {
            return meta.len();
        }
    }
    let page_count: u64 = conn
        .query_row(&format!("PRAGMA {}.page_count", schema), [], |r| r.get(0))
        .unwrap_or(0);
    let page_size: u64 = conn
        .query_row(&format!("PRAGMA {}.page_size", schema), [], |r| r.get(0))
        .unwrap_or(0);
    page_count.saturating_mul(page_size)
}

fn sqlite_freelist_bytes(conn: &Connection, schema: &str) -> u64 {
    let freelist: u64 = conn
        .query_row(&format!("PRAGMA {}.freelist_count", schema), [], |r| r.get(0))
        .unwrap_or(0);
    let page_size: u64 = conn
        .query_row(&format!("PRAGMA {}.page_size", schema), [], |r| r.get(0))
        .unwrap_or(0);
    freelist.saturating_mul(page_size)
}

pub(crate) fn get_database_stats_inner(
    conn: &Connection,
    epg_path: Option<&std::path::Path>,
    images_path: Option<&std::path::Path>,
) -> DatabaseStats {
    let has_images = is_images_db_attached(conn);
    let epg_db_bytes = sqlite_db_bytes(conn, "main", epg_path);
    let images_db_bytes = if has_images {
        sqlite_db_bytes(conn, "images_db", images_path)
    } else {
        0
    };
    let freelist_bytes = sqlite_freelist_bytes(conn, "main")
        .saturating_add(if has_images {
            sqlite_freelist_bytes(conn, "images_db")
        } else {
            0
        });

    let expired_http_cache_rows: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM trakt_http_cache
             WHERE CAST(strftime('%s', fetched_at) AS INTEGER) + ttl_seconds + ?1
                   < CAST(strftime('%s', 'now') AS INTEGER)",
            params![trakt::client::HTTP_CACHE_PRUNE_GRACE_SECS],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize;

    let expired_tmdb_negative_rows: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM tmdb_cache
             WHERE media_type = 'none' AND is_manual = 0
               AND updated_at < datetime('now', 'localtime', ?1)",
            params![format!("-{} days", TMDB_NEGATIVE_TTL_DAYS)],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize;

    let orphaned_image_rows: usize = if has_images {
        conn.query_row(
            "SELECT COUNT(*) FROM images_db.images
             WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap_or(0)
        .max(0) as usize
    } else {
        0
    };

    DatabaseStats {
        epg_db_bytes,
        images_db_bytes,
        total_db_bytes: epg_db_bytes.saturating_add(images_db_bytes),
        freelist_bytes,
        expired_http_cache_rows,
        expired_tmdb_negative_rows,
        orphaned_image_rows,
    }
}

pub(crate) fn maintain_database_inner(
    conn: &Connection,
    epg_path: Option<&std::path::Path>,
    images_path: Option<&std::path::Path>,
) -> Result<MaintenanceResult, String> {
    let has_images = is_images_db_attached(conn);
    let before_bytes = sqlite_db_bytes(conn, "main", epg_path).saturating_add(if has_images {
        sqlite_db_bytes(conn, "images_db", images_path)
    } else {
        0
    });

    let pruned_http_cache = trakt::client::prune_http_cache(conn);
    let pruned_tmdb_negatives = conn
        .execute(
            "DELETE FROM tmdb_cache
             WHERE media_type = 'none' AND is_manual = 0
               AND updated_at < datetime('now', 'localtime', ?1)",
            params![format!("-{} days", TMDB_NEGATIVE_TTL_DAYS)],
        )
        .unwrap_or(0);

    let pruned_orphan_images = if has_images {
        let _ = conn.execute(
            "DELETE FROM images_db.failed_logo_cache WHERE expires_at <= ?1",
            params![trakt::now_epoch_secs()],
        );
        conn.execute(
            "DELETE FROM images_db.images
             WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
            [],
        )
        .unwrap_or(0)
    } else {
        0
    };

    let converted_blob_images = if has_images {
        convert_legacy_text_images_to_blobs(conn)
    } else {
        0
    };

    let _ = conn.execute("PRAGMA optimize", []);
    conn.execute("VACUUM", [])
        .map_err(|e| format!("epg.db VACUUM hiba: {}", e))?;
    if has_images {
        conn.execute("VACUUM images_db", [])
            .map_err(|e| format!("images.db VACUUM hiba: {}", e))?;
    }

    let after_bytes = sqlite_db_bytes(conn, "main", epg_path).saturating_add(if has_images {
        sqlite_db_bytes(conn, "images_db", images_path)
    } else {
        0
    });

    Ok(MaintenanceResult {
        pruned_http_cache,
        pruned_tmdb_negatives,
        pruned_orphan_images,
        converted_blob_images,
        before_bytes,
        after_bytes,
        freed_bytes: before_bytes.saturating_sub(after_bytes),
    })
}

#[tauri::command]
async fn get_database_stats(state: tauri::State<'_, AppState>) -> Result<DatabaseStats, String> {
    let epg_path = get_db_path();
    let images_path = get_images_db_path();
    let conn = state.db.lock().await;
    Ok(get_database_stats_inner(
        &conn,
        Some(epg_path.as_path()),
        Some(images_path.as_path()),
    ))
}

#[tauri::command]
async fn maintain_database(state: tauri::State<'_, AppState>) -> Result<MaintenanceResult, String> {
    let _sync_guard = state
        .trakt
        .sync_lock
        .try_lock()
        .map_err(|_| "TRAKT_SYNC_ACTIVE: Trakt szinkronizálás közben az adatbázis karbantartás nem futtatható.".to_string())?;
    let epg_path = get_db_path();
    let images_path = get_images_db_path();
    let conn = state.db.lock().await;
    maintain_database_inner(&conn, Some(epg_path.as_path()), Some(images_path.as_path()))
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct LogoSearchResult {
    url: String,
    thumb_url: String,
    source_name: String,
    format: String,
    title: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct MatchedChannelItem {
    channel_id: String,
    channel_name: String,
    logo_url: String,
    cached_data: Option<String>,
    source_name: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelLogoStats {
    total_channels: usize,
    with_logo_count: usize,
    missing_logo_count: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct MissingChannelInfo {
    id: String,
    source_id: String,
    name: String,
    country_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IptvOrgLogoItem {
    channel: String,
    url: String,
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TvLogoTreeItem {
    path: String,
}

#[derive(Debug, Deserialize)]
struct TvLogoGitTreeResponse {
    tree: Option<Vec<TvLogoTreeItem>>,
}

struct TvLogoEntry {
    path: String,
    country_dir: String,
    fname_lower: String,
    name_no_ext: String,
    base_name_lower: String,
    is_int: bool,
}

struct TvLogoIndex {
    entries: Vec<TvLogoEntry>,
    by_country_exact: HashMap<(String, String), usize>,
    by_global_base: HashMap<String, usize>,
    by_country: HashMap<String, Vec<usize>>,
}

impl TvLogoIndex {
    fn from_paths(paths: Vec<String>) -> Self {
        let mut entries = Vec::with_capacity(paths.len());
        let mut by_country_exact = HashMap::with_capacity(paths.len());
        let mut by_global_base = HashMap::with_capacity(paths.len());
        let mut by_country: HashMap<String, Vec<usize>> = HashMap::new();

        for path in paths {
            let Some(after_countries) = path.strip_prefix("countries/") else {
                continue;
            };
            let Some((country_dir_str, rel)) = after_countries.split_once('/') else {
                continue;
            };
            let country_dir = country_dir_str.to_ascii_lowercase();
            let fname = rel.rsplit('/').next().unwrap_or(rel);
            let fname_lower = fname.to_ascii_lowercase();
            let name_no_ext = fname.trim_end_matches(".png").to_string();
            let base_name_lower = name_no_ext
                .rsplit_once('-')
                .map(|(b, _)| b)
                .unwrap_or(&name_no_ext)
                .to_ascii_lowercase();
            let is_int = country_dir == "international";

            let idx = entries.len();
            by_country_exact
                .entry((country_dir.clone(), fname_lower.clone()))
                .or_insert(idx);
            by_global_base
                .entry(base_name_lower.clone())
                .or_insert(idx);
            by_country
                .entry(country_dir.clone())
                .or_default()
                .push(idx);

            entries.push(TvLogoEntry {
                path,
                country_dir,
                fname_lower,
                name_no_ext,
                base_name_lower,
                is_int,
            });
        }

        Self {
            entries,
            by_country_exact,
            by_global_base,
            by_country,
        }
    }

    fn make_result(&self, idx: usize, source_name: &str) -> LogoSearchResult {
        let e = &self.entries[idx];
        let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", e.path);
        LogoSearchResult {
            url: url.clone(),
            thumb_url: url,
            source_name: source_name.to_string(),
            format: "PNG".to_string(),
            title: e.name_no_ext.clone(),
        }
    }

    fn match_logo(
        &self,
        country_dir: Option<&str>,
        country_code: &str,
        slug: &str,
    ) -> Option<LogoSearchResult> {
        let slug_lower = slug.to_ascii_lowercase();
        let target_exact = format!("{}-{}.png", slug_lower, country_code.to_ascii_lowercase());
        let target_simple = format!("{}.png", slug_lower);

        // 1. Target country folder (O(1))
        if let Some(dir) = country_dir {
            let dir_lower = dir.to_ascii_lowercase();
            if !country_code.is_empty() {
                if let Some(&idx) = self.by_country_exact.get(&(dir_lower.clone(), target_exact)) {
                    return Some(self.make_result(idx, "tv-logo"));
                }
            }
            if let Some(&idx) = self.by_country_exact.get(&(dir_lower, target_simple.clone())) {
                return Some(self.make_result(idx, "tv-logo"));
            }
        }

        // 2. International folder (O(1))
        let target_int = format!("{}-int.png", slug_lower);
        if let Some(&idx) = self
            .by_country_exact
            .get(&("international".to_string(), target_int))
        {
            return Some(self.make_result(idx, "tv-logo (int)"));
        }
        if let Some(&idx) = self
            .by_country_exact
            .get(&("international".to_string(), target_simple))
        {
            return Some(self.make_result(idx, "tv-logo (int)"));
        }

        // 3. Any country folder exact base match (O(1))
        if let Some(&idx) = self.by_global_base.get(&slug_lower) {
            return Some(self.make_result(idx, "tv-logo (global)"));
        }

        // 4. Fuzzy match in target country folder only (scans only that country's entries)
        if let Some(dir) = country_dir {
            let dir_lower = dir.to_ascii_lowercase();
            if let Some(indices) = self.by_country.get(&dir_lower) {
                let mut best_target: Option<(f64, usize)> = None;
                for &idx in indices {
                    let score = strsim::jaro_winkler(&slug_lower, &self.entries[idx].base_name_lower);
                    if score >= 0.85 && best_target.is_none_or(|(best, _)| score > best) {
                        best_target = Some((score, idx));
                    }
                }
                if let Some((_, idx)) = best_target {
                    return Some(self.make_result(idx, "tv-logo"));
                }
            }
        }

        // 5. Fuzzy match globally (Jaro-Winkler >= 0.88)
        let mut best_global: Option<(f64, usize)> = None;
        for (idx, e) in self.entries.iter().enumerate() {
            let score = strsim::jaro_winkler(&slug_lower, &e.base_name_lower);
            if score >= 0.88 && best_global.is_none_or(|(best, _)| score > best) {
                best_global = Some((score, idx));
            }
        }
        if let Some((_, idx)) = best_global {
            let src = if self.entries[idx].is_int {
                "tv-logo (int)"
            } else {
                "tv-logo (global)"
            };
            return Some(self.make_result(idx, src));
        }

        None
    }

    fn search_logos(
        &self,
        country_dir: Option<&str>,
        country_code: &str,
        slug: &str,
        clean_name: &str,
    ) -> Vec<LogoSearchResult> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if let Some(exact) = self.match_logo(country_dir, country_code, slug) {
            seen.insert(exact.url.clone());
            results.push(exact);
        }

        let slug_lower = slug.to_ascii_lowercase();
        let clean_lower = clean_name.to_lowercase();

        let matches_entry = |e: &TvLogoEntry| -> bool {
            (slug_lower.len() >= 3 && e.fname_lower.contains(&slug_lower))
                || (!clean_lower.is_empty() && e.fname_lower.contains(&clean_lower))
                || strsim::jaro_winkler(&slug_lower, &e.base_name_lower) >= 0.82
        };

        // 1. Target country
        if let Some(dir) = country_dir {
            if let Some(indices) = self.by_country.get(&dir.to_ascii_lowercase()) {
                for &idx in indices {
                    if matches_entry(&self.entries[idx]) {
                        let r = self.make_result(idx, "tv-logo");
                        if seen.insert(r.url.clone()) {
                            results.push(r);
                        }
                    }
                }
            }
        }

        // 2. International
        if let Some(indices) = self.by_country.get("international") {
            for &idx in indices {
                if matches_entry(&self.entries[idx]) {
                    let r = self.make_result(idx, "tv-logo (int)");
                    if seen.insert(r.url.clone()) {
                        results.push(r);
                    }
                }
            }
        }

        // 3. Global substring match
        if slug_lower.len() >= 3 || !clean_lower.is_empty() {
            let target_dir_lower = country_dir.map(|d| d.to_ascii_lowercase());
            for (idx, e) in self.entries.iter().enumerate() {
                if e.is_int || target_dir_lower.as_deref() == Some(e.country_dir.as_str()) {
                    continue;
                }
                if (slug_lower.len() >= 3 && e.fname_lower.contains(&slug_lower))
                    || (!clean_lower.is_empty() && e.fname_lower.contains(&clean_lower))
                {
                    let r = self.make_result(idx, "tv-logo (global)");
                    if seen.insert(r.url.clone()) {
                        results.push(r);
                        if results.len() >= 40 {
                            break;
                        }
                    }
                }
            }
        }

        results
    }
}

struct IptvOrgEntry {
    channel: String,
    url: String,
    fmt: &'static str,
    base: String,
    base_alnum: String,
}

struct IptvOrgLogoIndex {
    entries: Vec<IptvOrgEntry>,
    by_country_exact: HashMap<(String, String), usize>,
    by_country_alnum: HashMap<(String, String), usize>,
    by_global_exact: HashMap<String, usize>,
    by_global_alnum: HashMap<String, usize>,
    by_country: HashMap<String, Vec<usize>>,
}

impl IptvOrgLogoIndex {
    fn from_items(items: Vec<IptvOrgLogoItem>) -> Self {
        let mut entries = Vec::with_capacity(items.len());
        let mut by_country_exact = HashMap::with_capacity(items.len());
        let mut by_country_alnum = HashMap::with_capacity(items.len());
        let mut by_global_exact = HashMap::with_capacity(items.len());
        let mut by_global_alnum = HashMap::with_capacity(items.len());
        let mut by_country: HashMap<String, Vec<usize>> = HashMap::new();

        for item in items {
            let ch_lower = item.channel.to_lowercase();
            let (base_str, country_opt) = match ch_lower.rsplit_once('.') {
                Some((b, c)) => (b, Some(c)),
                None => (ch_lower.as_str(), None),
            };
            let global_base = ch_lower.split('.').next().unwrap_or(&ch_lower);
            let base = base_str.to_string();
            let base_alnum: String = base.chars().filter(|c| c.is_alphanumeric()).collect();
            let global_alnum: String = global_base.chars().filter(|c| c.is_alphanumeric()).collect();
            let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };

            let idx = entries.len();
            if let Some(cc) = country_opt {
                let cc_s = cc.to_string();
                by_country_exact
                    .entry((cc_s.clone(), base.clone()))
                    .or_insert(idx);
                if !base_alnum.is_empty() {
                    by_country_alnum
                        .entry((cc_s.clone(), base_alnum.clone()))
                        .or_insert(idx);
                }
                by_country.entry(cc_s).or_default().push(idx);
            }
            by_global_exact.entry(global_base.to_string()).or_insert(idx);
            if !global_alnum.is_empty() {
                by_global_alnum.entry(global_alnum).or_insert(idx);
            }

            entries.push(IptvOrgEntry {
                channel: item.channel,
                url: item.url,
                fmt,
                base,
                base_alnum,
            });
        }

        Self {
            entries,
            by_country_exact,
            by_country_alnum,
            by_global_exact,
            by_global_alnum,
            by_country,
        }
    }

    fn make_result(&self, idx: usize, source_name: &str) -> LogoSearchResult {
        let e = &self.entries[idx];
        LogoSearchResult {
            url: e.url.clone(),
            thumb_url: e.url.clone(),
            source_name: source_name.to_string(),
            format: e.fmt.to_string(),
            title: e.channel.clone(),
        }
    }

    fn match_country(&self, slug: &str, country_code: &str) -> Option<LogoSearchResult> {
        let cc = country_code.to_lowercase();
        let slug_lower = slug.to_lowercase();
        let slug_alnum: String = slug_lower.chars().filter(|c| c.is_alphanumeric()).collect();

        // 1. Exact O(1)
        if let Some(&idx) = self.by_country_exact.get(&(cc.clone(), slug_lower)) {
            return Some(self.make_result(idx, "iptv-org"));
        }
        if !slug_alnum.is_empty() {
            if let Some(&idx) = self.by_country_alnum.get(&(cc.clone(), slug_alnum.clone())) {
                return Some(self.make_result(idx, "iptv-org"));
            }

            // 2. Fuzzy within country only
            if let Some(indices) = self.by_country.get(&cc) {
                let mut best: Option<(f64, usize)> = None;
                for &idx in indices {
                    let score = strsim::jaro_winkler(&slug_alnum, &self.entries[idx].base_alnum);
                    if score >= 0.88 && best.is_none_or(|(b, _)| score > b) {
                        best = Some((score, idx));
                    }
                }
                if let Some((_, idx)) = best {
                    return Some(self.make_result(idx, "iptv-org (fuzzy)"));
                }
            }
        }
        None
    }

    fn match_any(&self, slug: &str) -> Option<LogoSearchResult> {
        let slug_lower = slug.to_lowercase();
        let slug_alnum: String = slug_lower.chars().filter(|c| c.is_alphanumeric()).collect();
        if slug_alnum.is_empty() {
            return None;
        }

        if let Some(&idx) = self.by_global_exact.get(&slug_lower) {
            return Some(self.make_result(idx, "iptv-org"));
        }
        if let Some(&idx) = self.by_global_alnum.get(&slug_alnum) {
            return Some(self.make_result(idx, "iptv-org"));
        }

        let mut best: Option<(f64, usize)> = None;
        for (idx, e) in self.entries.iter().enumerate() {
            let score = strsim::jaro_winkler(&slug_alnum, &e.base_alnum);
            if score >= 0.90 && best.is_none_or(|(b, _)| score > b) {
                best = Some((score, idx));
            }
        }
        best.map(|(_, idx)| self.make_result(idx, "iptv-org (global fuzzy)"))
    }

    fn search(
        &self,
        slug: &str,
        clean_lower: &str,
        country_code: &str,
        seen_urls: &mut std::collections::HashSet<String>,
    ) -> Vec<LogoSearchResult> {
        let mut out = Vec::new();
        let cc = country_code.to_lowercase();

        if !cc.is_empty() {
            if let Some(indices) = self.by_country.get(&cc) {
                for &idx in indices {
                    let e = &self.entries[idx];
                    if ((slug.len() >= 3 && e.base.contains(slug)) || e.base.contains(clean_lower))
                        && seen_urls.insert(e.url.clone())
                    {
                        out.push(self.make_result(idx, "iptv-org"));
                    }
                }
            }
        }

        for (idx, e) in self.entries.iter().enumerate() {
            if ((slug.len() >= 3 && e.base.contains(slug)) || e.base.contains(clean_lower))
                && seen_urls.insert(e.url.clone())
            {
                out.push(self.make_result(idx, "iptv-org"));
                if out.len() >= 60 {
                    break;
                }
            }
        }

        out
    }
}

const LOGO_INDEX_MEM_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 86_400);
const WIKIMEDIA_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 3600);
const WIKIMEDIA_CACHE_MAX_ENTRIES: usize = 256;

type WikimediaCacheMap = HashMap<String, (std::time::Instant, Vec<LogoSearchResult>)>;

static TV_LOGO_INDEX_CACHE: RwLock<Option<(std::time::Instant, TvLogoIndex)>> = RwLock::new(None);
static IPTV_LOGOS_INDEX_CACHE: RwLock<Option<(std::time::Instant, IptvOrgLogoIndex)>> = RwLock::new(None);
static WIKIMEDIA_SEARCH_CACHE: LazyLock<RwLock<WikimediaCacheMap>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

#[derive(Debug, Deserialize)]
struct MwQueryResponse {
    query: Option<MwQuery>,
}

#[derive(Debug, Deserialize)]
struct MwQuery {
    pages: Option<HashMap<String, MwPage>>,
}

#[derive(Debug, Deserialize)]
struct MwPage {
    title: String,
    #[serde(default)]
    index: Option<i64>,
    imageinfo: Option<Vec<MwImageInfo>>,
}

#[derive(Debug, Deserialize)]
struct MwImageInfo {
    url: Option<String>,
    thumburl: Option<String>,
    mime: Option<String>,
}

static RE_CHANNEL_AT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"@[A-Za-z0-9]+").unwrap());
static RE_CHANNEL_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(HU|HUN|RO|DE|AT|SK|CZ|UK|US|PT|PL):\s*").unwrap());
static RE_CHANNEL_LANG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[]\s*(HU|HUN|RO|DE|AT|SK|CZ|UK|US|PT|PL|ENG|HUN/ENG)\s*[\)\]]").unwrap());
static RE_CHANNEL_PROVIDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[]\s*(Telekom|Vodafone|Digi|Yettel|Direct\s*One|UPC|backup|test|\d+)\s*[\)\]]").unwrap());
static RE_CHANNEL_QUALITY_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[]\s*(HD|FHD|4K|UHD|SD|HEVC|H\.?265|1080[pi]|720[pi]|50fps|60fps|100fps)\s*[\)\]]").unwrap());
static RE_CHANNEL_QUALITY_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(HD|FHD|4K|UHD|SD|HEVC|H\.?265|1080[pi]|720[pi]|50fps|60fps|100fps)\b").unwrap());
static RE_CHANNEL_REGION_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[]\s*(pacific|east|west)\s*[\)\]]").unwrap());
static RE_CHANNEL_REGION_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(pacific|east|west)\b").unwrap());
static RE_CHANNEL_DOMAINS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(hu|pt|us|uk|de|at|ro|sk|cz|pl|fr|it|es|nl|com|net|org|tv|ca|ch|se|no|dk|fi|eu|int)\b").unwrap());
static RE_SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

fn clean_channel_name(raw_name: &str) -> String {
    let mut name = RE_CHANNEL_AT.replace_all(raw_name, "").to_string();
    name = RE_CHANNEL_PREFIX.replace(&name, "").to_string();
    name = RE_CHANNEL_LANG.replace_all(&name, "").to_string();
    name = RE_CHANNEL_PROVIDER.replace_all(&name, "").to_string();
    name = RE_CHANNEL_QUALITY_BRACKET.replace_all(&name, "").to_string();
    name = RE_CHANNEL_QUALITY_WORD.replace_all(&name, "").to_string();
    name = RE_CHANNEL_REGION_BRACKET.replace_all(&name, "").to_string();
    name = RE_CHANNEL_REGION_WORD.replace_all(&name, "").to_string();
    name = RE_CHANNEL_DOMAINS.replace_all(&name, "").to_string();
    name = RE_SPACES.replace_all(&name, " ").to_string();
    name = name.trim().trim_matches(|c: char| c == '-' || c == '_' || c == ':' || c == '.' || c == '/' || c == '(' || c == ')').trim().to_string();

    if name.is_empty() {
        raw_name.trim().to_string()
    } else {
        name
    }
}

fn slugify_channel_name(clean_name: &str) -> String {
    let mut s = clean_name.to_lowercase();
    s = s.replace('+', " plus ");
    s = s.replace('&', " and ");

    let mut normalized = String::new();
    for c in s.chars() {
        match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => normalized.push('a'),
            'é' | 'è' | 'ë' | 'ê' => normalized.push('e'),
            'í' | 'ì' | 'ï' | 'î' => normalized.push('i'),
            'ó' | 'ö' | 'ő' | 'ò' | 'ô' | 'õ' | 'ø' => normalized.push('o'),
            'ú' | 'ü' | 'ű' | 'ù' | 'û' => normalized.push('u'),
            'ç' => normalized.push('c'),
            'ñ' => normalized.push('n'),
            'ß' => normalized.push_str("ss"),
            _ => normalized.push(c),
        }
    }

    let mut slug = String::new();
    let mut last_was_hyphen = true;
    for c in normalized.chars() {
        if c.is_alphanumeric() {
            slug.push(c);
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            slug.push('-');
            last_was_hyphen = true;
        }
    }

    slug.trim_matches('-').to_string()
}

fn map_country_code(country_code: &str) -> (Option<&'static str>, String) {
    let trimmed = country_code.trim();
    match trimmed.to_uppercase().as_str() {
        "AL" => (Some("albania"), "al".to_string()),
        "AR" => (Some("argentina"), "ar".to_string()),
        "AU" => (Some("australia"), "au".to_string()),
        "AT" => (Some("austria"), "at".to_string()),
        "AZ" => (Some("azerbaijan"), "az".to_string()),
        "BE" => (Some("belgium"), "be".to_string()),
        "BR" => (Some("brazil"), "br".to_string()),
        "BG" => (Some("bulgaria"), "bg".to_string()),
        "CA" => (Some("canada"), "ca".to_string()),
        "CL" => (Some("chile"), "cl".to_string()),
        "CR" => (Some("costa-rica"), "cr".to_string()),
        "HR" => (Some("croatia"), "hr".to_string()),
        "CZ" => (Some("czech-republic"), "cz".to_string()),
        "FR" => (Some("france"), "fr".to_string()),
        "DE" => (Some("germany"), "de".to_string()),
        "GR" => (Some("greece"), "gr".to_string()),
        "HK" => (Some("hong-kong"), "hk".to_string()),
        "HU" => (Some("hungary"), "hu".to_string()),
        "IN" => (Some("india"), "in".to_string()),
        "ID" => (Some("indonesia"), "id".to_string()),
        "INT" => (Some("international"), "int".to_string()),
        "IE" => (Some("ireland"), "ie".to_string()),
        "IL" => (Some("israel"), "il".to_string()),
        "IT" => (Some("italy"), "it".to_string()),
        "LB" => (Some("lebanon"), "lb".to_string()),
        "LT" => (Some("lithuania"), "lt".to_string()),
        "LU" => (Some("luxembourg"), "lu".to_string()),
        "MY" => (Some("malaysia"), "my".to_string()),
        "MT" => (Some("malta"), "mt".to_string()),
        "MX" => (Some("mexico"), "mx".to_string()),
        "NL" => (Some("netherlands"), "nl".to_string()),
        "NZ" => (Some("new-zealand"), "nz".to_string()),
        "SE" => (Some("nordic"), "se".to_string()),
        "NO" => (Some("nordic"), "no".to_string()),
        "DK" => (Some("nordic"), "dk".to_string()),
        "FI" => (Some("nordic"), "fi".to_string()),
        "IS" => (Some("nordic"), "is".to_string()),
        "PH" => (Some("philippines"), "ph".to_string()),
        "PL" => (Some("poland"), "pl".to_string()),
        "PT" => (Some("portugal"), "pt".to_string()),
        "RO" => (Some("romania"), "ro".to_string()),
        "RU" => (Some("russia"), "ru".to_string()),
        "RS" => (Some("serbia"), "rs".to_string()),
        "SG" => (Some("singapore"), "sg".to_string()),
        "SK" => (Some("slovakia"), "sk".to_string()),
        "SI" => (Some("slovenia"), "si".to_string()),
        "ZA" => (Some("south-africa"), "za".to_string()),
        "ES" => (Some("spain"), "es".to_string()),
        "CH" => (Some("switzerland"), "ch".to_string()),
        "TR" => (Some("turkey"), "tr".to_string()),
        "UA" => (Some("ukraine"), "ua".to_string()),
        "AE" => (Some("united-arab-emirates"), "ae".to_string()),
        "GB" | "UK" | "EN" => (Some("united-kingdom"), "uk".to_string()),
        "US" => (Some("united-states"), "us".to_string()),
        other => (None, other.to_lowercase()),
    }
}

async fn fetch_tv_logo_full_tree(state: &AppState) -> Result<Vec<String>, String> {
    let cache_key = "tv_logo_full_tree_paths";

    let cached: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT content FROM external_logo_cache WHERE source_key = ?1 AND updated_at > datetime('now', '-7 days')",
            params![cache_key],
            |row| row.get(0),
        ).ok()
    };

    if let Some(json_str) = cached {
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(&json_str) {
            if !paths.is_empty() {
                return Ok(paths);
            }
        }
    }

    let url = "https://api.github.com/repos/tv-logo/tv-logos/git/trees/main?recursive=1";
    let resp = state.http.get(url).send().await;
    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(text) = r.text().await {
                if let Ok(tree_resp) = serde_json::from_str::<TvLogoGitTreeResponse>(&text) {
                    if let Some(tree) = tree_resp.tree {
                        let paths: Vec<String> = tree.into_iter()
                            .map(|item| item.path)
                            .filter(|p| p.starts_with("countries/") && p.ends_with(".png"))
                            .collect();

                        if let Ok(ser) = serde_json::to_string(&paths) {
                            let conn = state.db.lock().await;
                            let _ = conn.execute(
                                "INSERT INTO external_logo_cache (source_key, content, updated_at) VALUES (?1, ?2, datetime('now'))
                                 ON CONFLICT(source_key) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
                                params![cache_key, ser],
                            );
                        }
                        return Ok(paths);
                    }
                }
            }
        }
        _ => {}
    }

    let fallback: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT content FROM external_logo_cache WHERE source_key = ?1",
            params![cache_key],
            |row| row.get(0),
        ).ok()
    };
    if let Some(json_str) = fallback {
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(&json_str) {
            if !paths.is_empty() {
                return Ok(paths);
            }
        }
    }

    Ok(Vec::new())
}

#[cfg(test)]
fn match_tv_logo_from_tree(
    paths: &[String],
    country_dir: Option<&str>,
    country_code: &str,
    slug: &str,
) -> Option<LogoSearchResult> {
    TvLogoIndex::from_paths(paths.to_vec()).match_logo(country_dir, country_code, slug)
}


async fn fetch_iptv_org_logos(state: &AppState) -> Result<Vec<IptvOrgLogoItem>, String> {
    let cache_key = "iptv_org_logos".to_string();

    let cached: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT content FROM external_logo_cache WHERE source_key = ?1 AND updated_at > datetime('now', '-7 days')",
            params![cache_key],
            |row| row.get(0),
        ).ok()
    };

    if let Some(json_str) = cached {
        if let Ok(logos) = serde_json::from_str::<Vec<IptvOrgLogoItem>>(&json_str) {
            if !logos.is_empty() {
                return Ok(logos);
            }
        }
    }

    let resp = state.http.get("https://iptv-org.github.io/api/logos.json").send().await;
    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(text) = r.text().await {
                if let Ok(logos) = serde_json::from_str::<Vec<IptvOrgLogoItem>>(&text) {
                    {
                        let conn = state.db.lock().await;
                        let _ = conn.execute(
                            "INSERT INTO external_logo_cache (source_key, content, updated_at) VALUES (?1, ?2, datetime('now'))
                             ON CONFLICT(source_key) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
                            params![cache_key, text],
                        );
                    }
                    return Ok(logos);
                }
            }
        }
        _ => {}
    }

    let fallback: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT content FROM external_logo_cache WHERE source_key = ?1",
            params![cache_key],
            |row| row.get(0),
        ).ok()
    };
    if let Some(json_str) = fallback {
        if let Ok(logos) = serde_json::from_str::<Vec<IptvOrgLogoItem>>(&json_str) {
            if !logos.is_empty() {
                return Ok(logos);
            }
        }
    }

    Ok(Vec::new())
}

#[cfg(test)]
fn match_iptv_org_logos(
    logos: &[IptvOrgLogoItem],
    slug: &str,
    country_code: &str,
) -> Option<LogoSearchResult> {
    IptvOrgLogoIndex::from_items(logos.to_vec()).match_country(slug, country_code)
}

#[cfg(test)]
fn match_iptv_org_logos_any(
    logos: &[IptvOrgLogoItem],
    slug: &str,
) -> Option<LogoSearchResult> {
    IptvOrgLogoIndex::from_items(logos.to_vec()).match_any(slug)
}

async fn search_wikimedia_commons(state: &AppState, clean_name: &str, country_name: &str) -> Vec<LogoSearchResult> {
    let query = if country_name.trim().is_empty() {
        format!("{} logo", clean_name.trim())
    } else {
        format!("{} {} logo", clean_name.trim(), country_name.trim())
    };
    let cache_key = query.to_lowercase();

    {
        let r = WIKIMEDIA_SEARCH_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((fetched_at, cached)) = r.get(&cache_key) {
            if fetched_at.elapsed() < WIKIMEDIA_CACHE_TTL {
                return cached.clone();
            }
        }
    }

    let client = &state.http;

    let resp = client
        .get("https://commons.wikimedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("generator", "search"),
            ("gsrsearch", query.as_str()),
            ("gsrnamespace", "6"),
            ("prop", "imageinfo"),
            ("iiprop", "url|mime"),
            ("iiurlwidth", "200"),
            ("format", "json"),
        ])
        .send()
        .await;

    let mut results = Vec::new();
    let mut got_valid_response = false;
    if let Ok(r) = resp {
        if r.status().is_success() {
            if let Ok(text) = r.text().await {
                if let Ok(data) = serde_json::from_str::<MwQueryResponse>(&text) {
                    got_valid_response = true;
                    if let Some(q) = data.query {
                        if let Some(pages) = q.pages {
                            let mut sorted_pages: Vec<MwPage> = pages.into_values().collect();
                            sorted_pages.sort_by_key(|p| p.index.unwrap_or(9999));

                            for p in sorted_pages {
                                if let Some(info_list) = p.imageinfo {
                                    if let Some(info) = info_list.into_iter().next() {
                                        if let Some(url) = info.url {
                                            let thumb = info.thumburl.unwrap_or_else(|| url.clone());
                                            let mime = info.mime.unwrap_or_default().to_lowercase();
                                            if mime.contains("svg")
                                                || mime.contains("png")
                                                || mime.contains("jpeg")
                                                || mime.contains("jpg")
                                                || mime.contains("webp")
                                            {
                                                let fmt = if mime.contains("svg") || url.ends_with(".svg") {
                                                    "SVG"
                                                } else {
                                                    "PNG"
                                                };
                                                let title = p
                                                    .title
                                                    .trim_start_matches("File:")
                                                    .trim_start_matches("file:")
                                                    .to_string();
                                                results.push(LogoSearchResult {
                                                    url,
                                                    thumb_url: thumb,
                                                    source_name: "Wikimedia Commons".to_string(),
                                                    format: fmt.to_string(),
                                                    title,
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if got_valid_response {
        let mut w = WIKIMEDIA_SEARCH_CACHE.write().unwrap_or_else(|e| e.into_inner());
        if w.len() >= WIKIMEDIA_CACHE_MAX_ENTRIES {
            w.retain(|_, (ts, _)| ts.elapsed() < WIKIMEDIA_CACHE_TTL);
            if w.len() >= WIKIMEDIA_CACHE_MAX_ENTRIES {
                if let Some(oldest_key) = w
                    .iter()
                    .min_by_key(|(_, (ts, _))| *ts)
                    .map(|(k, _)| k.clone())
                {
                    w.remove(&oldest_key);
                }
            }
        }
        w.insert(cache_key, (std::time::Instant::now(), results.clone()));
    }

    results
}


#[tauri::command]
async fn clean_and_slugify_channel(name: String) -> Result<(String, String), String> {
    let clean = clean_channel_name(&name);
    let slug = slugify_channel_name(&clean);
    Ok((clean, slug))
}

async fn get_missing_logo_channels_inner(state: &AppState, source_id: Option<String>) -> Result<Vec<MissingChannelInfo>, String> {
    let conn = state.db.lock().await;
    let filter_source = match source_id.as_deref() {
        Some(s) if !s.is_empty() && s != "__ALL_SOURCES__" => Some(s.to_string()),
        _ => None,
    };

    let missing = if let Some(ref sid) = filter_source {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.source_id, c.name, COALESCE(s.country_code, 'HU') as country_code
             FROM channels c
             JOIN sources s ON s.id = c.source_id
             LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
             WHERE c.source_id = ?1
               AND (cus.custom_icon IS NULL OR cus.custom_icon = '')
               AND (c.icon IS NULL OR c.icon = '')
             ORDER BY c.name ASC",
        ).map_err(|e| e.to_string())?;

        let iter = stmt.query_map(params![sid], |row| {
            Ok(MissingChannelInfo {
                id: row.get(0)?,
                source_id: row.get(1)?,
                name: row.get(2)?,
                country_code: row.get(3)?,
            })
        }).map_err(|e| e.to_string())?;

        iter.filter_map(|r| r.ok()).collect()
    } else {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.source_id, c.name, COALESCE(s.country_code, 'HU') as country_code
             FROM channels c
             JOIN sources s ON s.id = c.source_id
             LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
             WHERE (cus.custom_icon IS NULL OR cus.custom_icon = '')
               AND (c.icon IS NULL OR c.icon = '')
             ORDER BY c.name ASC",
        ).map_err(|e| e.to_string())?;

        let iter = stmt.query_map([], |row| {
            Ok(MissingChannelInfo {
                id: row.get(0)?,
                source_id: row.get(1)?,
                name: row.get(2)?,
                country_code: row.get(3)?,
            })
        }).map_err(|e| e.to_string())?;

        iter.filter_map(|r| r.ok()).collect()
    };

    Ok(missing)
}

async fn ensure_tv_logo_tree_loaded(state: &AppState) -> Result<(), String> {
    let already_fresh = {
        let r = TV_LOGO_INDEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        r.as_ref().is_some_and(|(ts, _)| ts.elapsed() < LOGO_INDEX_MEM_TTL)
    };
    if !already_fresh {
        if let Ok(paths) = fetch_tv_logo_full_tree(state).await {
            let index = TvLogoIndex::from_paths(paths);
            let mut w = TV_LOGO_INDEX_CACHE.write().unwrap_or_else(|e| e.into_inner());
            *w = Some((std::time::Instant::now(), index));
        }
    }
    Ok(())
}

async fn ensure_iptv_logos_loaded(state: &AppState) -> Result<(), String> {
    let already_fresh = {
        let r = IPTV_LOGOS_INDEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        r.as_ref().is_some_and(|(ts, _)| ts.elapsed() < LOGO_INDEX_MEM_TTL)
    };
    if !already_fresh {
        if let Ok(logos) = fetch_iptv_org_logos(state).await {
            let index = IptvOrgLogoIndex::from_items(logos);
            let mut w = IPTV_LOGOS_INDEX_CACHE.write().unwrap_or_else(|e| e.into_inner());
            *w = Some((std::time::Instant::now(), index));
        }
    }
    Ok(())
}

#[tauri::command]
async fn preload_logo_sources(state: tauri::State<'_, AppState>, _country_codes: Vec<String>) -> Result<(), String> {
    let _ = tokio::join!(
        ensure_tv_logo_tree_loaded(state.inner()),
        ensure_iptv_logos_loaded(state.inner()),
    );
    Ok(())
}

async fn find_channel_logo_github_inner(state: &AppState, 
    source_id: String,
    channel_id: String,
    channel_name: String,
    country_code: String,
) -> Result<Option<MatchedChannelItem>, String> {
    let clean = clean_channel_name(&channel_name);
    let slug = slugify_channel_name(&clean);
    let (country_dir, c_code) = map_country_code(&country_code);

    let mut candidate: Option<(String, String)> = None;

    // 1. Forrás A: tv-logo indexed lookup
    let _ = ensure_tv_logo_tree_loaded(state).await;
    {
        let r = TV_LOGO_INDEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((_, ref idx)) = *r {
            if let Some(res) = idx.match_logo(country_dir, &c_code, &slug) {
                candidate = Some((res.url, res.source_name));
            }
        }
    }

    // 2. Forrás B: iptv-org indexed lookup
    if candidate.is_none() {
        let _ = ensure_iptv_logos_loaded(state).await;
        let r = IPTV_LOGOS_INDEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((_, ref idx)) = *r {
            if !c_code.is_empty() {
                if let Some(res) = idx.match_country(&slug, &c_code) {
                    candidate = Some((res.url, "iptv-org".to_string()));
                }
            }
            if candidate.is_none() && c_code != "int" {
                if let Some(res) = idx.match_country(&slug, "int") {
                    candidate = Some((res.url, "iptv-org (int)".to_string()));
                }
            }
            if candidate.is_none() {
                if let Some(res) = idx.match_any(&slug) {
                    candidate = Some((res.url, "iptv-org (global)".to_string()));
                }
            }
        }
    }

    if let Some((logo_url, src_name)) = candidate {
        let conn = state.db.lock().await;
        let _ = conn.execute(
            "INSERT INTO channel_user_settings (source_id, channel_id, custom_icon) VALUES (?1, ?2, ?3)
             ON CONFLICT(source_id, channel_id) DO UPDATE SET custom_icon = excluded.custom_icon",
            params![source_id, channel_id, logo_url],
        );

        return Ok(Some(MatchedChannelItem {
            channel_id,
            channel_name,
            logo_url,
            cached_data: None,
            source_name: src_name,
        }));
    }

    Ok(None)
}

async fn find_channel_logo_wikimedia_inner(state: &AppState, 
    source_id: String,
    channel_id: String,
    channel_name: String,
    country_code: String,
) -> Result<Option<MatchedChannelItem>, String> {
    let clean = clean_channel_name(&channel_name);
    let slug = slugify_channel_name(&clean);
    let (country_dir, _) = map_country_code(&country_code);

    let mut candidate: Option<(String, String)> = None;

    // 3. Forrás C: Wikimedia Commons (Country)
    if let Some(dir) = country_dir {
        let wiki_results = search_wikimedia_commons(state, &clean, dir).await;
        for w in &wiki_results {
            let title_lower = w.title.to_lowercase();
            if title_lower.contains(&slug) || title_lower.contains(&clean.to_lowercase()) {
                candidate = Some((w.url.clone(), "Wikimedia Commons".to_string()));
                break;
            }
        }
    }

    // 3b. Forrás C: Wikimedia Commons (Fallback: Without Country / Global)
    if candidate.is_none() {
        let wiki_global = search_wikimedia_commons(state, &clean, "").await;
        for w in &wiki_global {
            let title_lower = w.title.to_lowercase();
            if title_lower.contains(&slug) || title_lower.contains(&clean.to_lowercase()) {
                candidate = Some((w.url.clone(), "Wikimedia Commons (global)".to_string()));
                break;
            }
        }
    }

    if let Some((logo_url, src_name)) = candidate {
        let conn = state.db.lock().await;
        let _ = conn.execute(
            "INSERT INTO channel_user_settings (source_id, channel_id, custom_icon) VALUES (?1, ?2, ?3)
             ON CONFLICT(source_id, channel_id) DO UPDATE SET custom_icon = excluded.custom_icon",
            params![source_id, channel_id, logo_url],
        );

        return Ok(Some(MatchedChannelItem {
            channel_id,
            channel_name,
            logo_url,
            cached_data: None,
            source_name: src_name,
        }));
    }

    Ok(None)
}

#[tauri::command]
async fn cache_logo(
    state: tauri::State<'_, AppState>,
    url: String,
    image_type: Option<String>,
) -> Result<String, String> {
    cache_logo_with_type_inner(state.inner(), url, image_type.as_deref()).await
}

#[tauri::command]
async fn get_missing_logo_channels(state: tauri::State<'_, AppState>, source_id: Option<String>) -> Result<Vec<MissingChannelInfo>, String> {
    get_missing_logo_channels_inner(state.inner(), source_id).await
}

#[tauri::command]
async fn find_channel_logo_github(state: tauri::State<'_, AppState>, source_id: String, channel_id: String, channel_name: String, country_code: String) -> Result<Option<MatchedChannelItem>, String> {
    find_channel_logo_github_inner(state.inner(), source_id, channel_id, channel_name, country_code).await
}

#[tauri::command]
async fn find_channel_logo_wikimedia(state: tauri::State<'_, AppState>, source_id: String, channel_id: String, channel_name: String, country_code: String) -> Result<Option<MatchedChannelItem>, String> {
    find_channel_logo_wikimedia_inner(state.inner(), source_id, channel_id, channel_name, country_code).await
}

#[tauri::command]
async fn get_channel_logo_stats(state: tauri::State<'_, AppState>, source_id: Option<String>) -> Result<ChannelLogoStats, String> {
    let conn = state.db.lock().await;
    let filter_source = source_id.as_deref().filter(|s| !s.is_empty() && *s != "__ALL_SOURCES__");

    let (total, with_logo): (i64, i64) = if let Some(sid) = filter_source {
        let sql = "SELECT 
                COUNT(*) as total,
                SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END) as with_logo
             FROM channels c
             LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
             WHERE c.source_id = ?1";
        conn.query_row(sql, params![sid], |r| {
            let tot: i64 = r.get(0).unwrap_or(0);
            let wl: i64 = r.get(1).unwrap_or(0);
            Ok((tot, wl))
        }).unwrap_or((0, 0))
    } else {
        let sql = "SELECT 
                COUNT(*) as total,
                SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END) as with_logo
             FROM channels c
             LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id";
        conn.query_row(sql, [], |r| {
            let tot: i64 = r.get(0).unwrap_or(0);
            let wl: i64 = r.get(1).unwrap_or(0);
            Ok((tot, wl))
        }).unwrap_or((0, 0))
    };

    let total_usize = total.max(0) as usize;
    let with_logo_usize = with_logo.max(0) as usize;
    let missing = total_usize.saturating_sub(with_logo_usize);

    Ok(ChannelLogoStats {
        total_channels: total_usize,
        with_logo_count: with_logo_usize,
        missing_logo_count: missing,
    })
}

#[tauri::command]
async fn search_channel_logos(state: tauri::State<'_, AppState>, query: String, country_code: String) -> Result<Vec<LogoSearchResult>, String> {
    let clean = clean_channel_name(&query);
    let slug = slugify_channel_name(&clean);
    let (country_dir, c_code) = map_country_code(&country_code);

    let mut all_results = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    // Load both indexes and run Wikimedia searches concurrently
    let (_, _, wiki_country, wiki_global) = tokio::join!(
        ensure_tv_logo_tree_loaded(state.inner()),
        ensure_iptv_logos_loaded(state.inner()),
        async {
            if let Some(dir) = country_dir {
                search_wikimedia_commons(state.inner(), &clean, dir).await
            } else {
                Vec::new()
            }
        },
        search_wikimedia_commons(state.inner(), &clean, ""),
    );

    // 1. Forrás A: tv-logo (indexed)
    {
        let r = TV_LOGO_INDEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((_, ref idx)) = *r {
            for m in idx.search_logos(country_dir, &c_code, &slug, &clean) {
                if seen_urls.insert(m.url.clone()) {
                    all_results.push(m);
                }
            }
        }
    }

    // 2. Forrás B: iptv-org (indexed)
    let clean_lower = clean.to_lowercase();
    {
        let r = IPTV_LOGOS_INDEX_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((_, ref idx)) = *r {
            all_results.extend(idx.search(&slug, &clean_lower, &c_code, &mut seen_urls));
        }
    }

    // 3. Forrás C: Wikimedia Commons (country + global)
    for m in wiki_country.into_iter().chain(wiki_global) {
        if seen_urls.insert(m.url.clone()) {
            all_results.push(m);
        }
    }

    // Fuzzy rendezés előre kiszámolt kulcsokkal (O(N) regex/Jaro-Winkler az O(N log N) helyett)
    let mut scored: Vec<(f64, LogoSearchResult)> = all_results
        .into_iter()
        .map(|item| {
            let item_slug = slugify_channel_name(&clean_channel_name(&item.title));
            let score = strsim::jaro_winkler(&slug, &item_slug);
            (score, item)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    Ok(scored.into_iter().map(|(_, item)| item).collect())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelUserSettingExport {
    source_id: String,
    channel_id: String,
    is_favorite: bool,
    is_hidden: bool,
    custom_icon: Option<String>,
    custom_name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelOrderExport {
    source_id: String,
    channel_id: String,
    position: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct BackupData {
    version: u32,
    exported_at: String,
    sources: Vec<Source>,
    favorites: Vec<String>,
    channel_user_settings: Vec<ChannelUserSettingExport>,
    channel_orders: Vec<ChannelOrderExport>,
    #[serde(default)]
    channel_sort_presets: Vec<SortPreset>,
    settings: HashMap<String, String>,
}

// ── Channel-data export/import ───────────────────────────────────────────────

fn deserialize_bool_or_i32<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct BoolOrIntVisitor;
    impl<'de> serde::de::Visitor<'de> for BoolOrIntVisitor {
        type Value = i32;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a boolean or integer (0/1)")
        }

        fn visit_bool<E>(self, v: bool) -> Result<i32, E> {
            Ok(if v { 1 } else { 0 })
        }

        fn visit_i64<E>(self, v: i64) -> Result<i32, E> {
            Ok(if v != 0 { 1 } else { 0 })
        }

        fn visit_u64<E>(self, v: u64) -> Result<i32, E> {
            Ok(if v != 0 { 1 } else { 0 })
        }
    }

    deserializer.deserialize_any(BoolOrIntVisitor)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelExportRow {
    source_id: String,
    source_name: String,
    source_url: String,
    channel_id: String,
    channel_name: String,
    #[serde(deserialize_with = "deserialize_bool_or_i32")]
    is_favorite: i32,
    #[serde(deserialize_with = "deserialize_bool_or_i32")]
    is_hidden: i32,
    position: Option<i64>,
    custom_icon: Option<String>,
    custom_name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelDataExport {
    version: u32,
    exported_at: String,
    channels: Vec<ChannelExportRow>,
}

#[tauri::command]
async fn export_channel_data(state: tauri::State<'_, AppState>) -> Result<ChannelDataExport, String> {
    let conn = state.db.lock().await;

    let mut stmt = conn.prepare(
        "SELECT
            c.source_id,
            COALESCE(s.name, '') AS source_name,
            COALESCE(s.url, '')  AS source_url,
            c.id                 AS channel_id,
            c.name               AS channel_name,
            COALESCE(cu.is_favorite, 0) AS is_favorite,
            COALESCE(cu.is_hidden,   0) AS is_hidden,
            co.position,
            cu.custom_icon,
            cu.custom_name
         FROM channels c
         LEFT JOIN sources s ON c.source_id = s.id
         LEFT JOIN channel_user_settings cu
               ON  cu.source_id  = c.source_id
               AND cu.channel_id = c.id
         LEFT JOIN channel_orders co
               ON  co.source_id  = c.source_id
               AND co.channel_id = c.id
         ORDER BY s.name, co.position, c.id",
    ).map_err(|e| e.to_string())?;

    let rows = stmt.query_map([], |row| {
        Ok(ChannelExportRow {
            source_id:    row.get(0)?,
            source_name:  row.get(1)?,
            source_url:   row.get(2)?,
            channel_id:   row.get(3)?,
            channel_name: row.get(4)?,
            is_favorite:  row.get(5)?,
            is_hidden:    row.get(6)?,
            position:     row.get(7)?,
            custom_icon:  row.get(8)?,
            custom_name:  row.get(9)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut channels: Vec<ChannelExportRow> = Vec::new();
    for row in rows.flatten() { channels.push(row); }

    let exported_at: String = conn
        .query_row("SELECT datetime('now', 'localtime')", [], |r| r.get(0))
        .unwrap_or_default();

    Ok(ChannelDataExport {
        version: 1,
        exported_at,
        channels,
    })
}

#[tauri::command]
async fn import_channel_data(state: tauri::State<'_, AppState>, data: ChannelDataExport) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    for row in data.channels {
        // Upsert channel_user_settings
        tx.execute(
            "INSERT INTO channel_user_settings
                (source_id, channel_id, is_favorite, is_hidden, custom_icon, custom_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source_id, channel_id) DO UPDATE SET
                is_favorite = excluded.is_favorite,
                is_hidden   = excluded.is_hidden,
                custom_icon = excluded.custom_icon,
                custom_name = excluded.custom_name",
            params![
                row.source_id,
                row.channel_id,
                row.is_favorite,
                row.is_hidden,
                row.custom_icon,
                row.custom_name,
            ],
        ).ok();

        // Upsert channel_orders (only if position is specified)
        if let Some(pos) = row.position {
            tx.execute(
                "INSERT INTO channel_orders (source_id, channel_id, position)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(source_id, channel_id) DO UPDATE SET
                    position = excluded.position",
                params![row.source_id, row.channel_id, pos],
            ).ok();
        }
    }

    tx.commit().map_err(|e| format!("Import mentési hiba: {}", e))?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────

const TMDB_SECURE_KEY: &str = "tmdb_secure_key";
const LEGACY_TMDB_API_KEY: &str = "tmdb_api_key";

/// Keys that hold secrets or internal bookkeeping and must never be exposed via `get_settings` /
/// `export_settings` or written via `set_setting` / `import_settings`.
pub(crate) fn is_internal_or_protected_setting_key(key: &str) -> bool {
    key == LEGACY_TMDB_API_KEY
        || key.starts_with("trakt_secure_")
        || key.starts_with("tmdb_secure_")
        || key.starts_with("google_secure_")
        || key.starts_with("lng_hash_")
        || key.starts_with("migration_")
        || key.starts_with("update_")
}

pub(crate) fn export_settings_from_conn(conn: &Connection) -> Result<BackupData, String> {
    // Sources
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.url, s.last_downloaded, s.country_code, s.time_offset, s.sort_position,
                (SELECT COUNT(*) FROM channels c WHERE c.source_id = s.id) AS ch_count,
                (SELECT COUNT(*) FROM channel_user_settings cus WHERE cus.source_id = s.id AND cus.is_favorite = 1) AS fav_count,
                (SELECT COUNT(*) FROM channel_user_settings cus WHERE cus.source_id = s.id AND cus.is_hidden = 1) AS hid_count
         FROM sources s
         ORDER BY COALESCE(s.sort_position, s.rowid) ASC"
    ).map_err(|e| e.to_string())?;
    let sources_iter = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            last_downloaded: row.get(3)?,
            country_code: row.get(4)?,
            time_offset: row.get(5)?,
            sort_position: row.get(6)?,
            channel_count: Some(row.get(7)?),
            favorite_count: Some(row.get(8)?),
            hidden_count: Some(row.get(9)?),
        })
    }).map_err(|e| e.to_string())?;
    let sources: Vec<Source> = sources_iter.flatten().collect();

    // Favorites
    let mut stmt = conn.prepare("SELECT title FROM favorites ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let fav_iter = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut favorites = Vec::new();
    for title in fav_iter.flatten() { favorites.push(title); }

    // Channel user settings
    let mut stmt = conn.prepare("SELECT source_id, channel_id, is_favorite, is_hidden, custom_icon, custom_name FROM channel_user_settings").map_err(|e| e.to_string())?;
    let cus_iter = stmt.query_map([], |row| {
        let is_fav: i32 = row.get(2)?;
        let is_hid: i32 = row.get(3)?;
        Ok(ChannelUserSettingExport {
            source_id: row.get(0)?,
            channel_id: row.get(1)?,
            is_favorite: is_fav == 1,
            is_hidden: is_hid == 1,
            custom_icon: row.get(4)?,
            custom_name: row.get(5)?,
        })
    }).map_err(|e| e.to_string())?;
    let mut channel_user_settings = Vec::new();
    for c in cus_iter.flatten() { channel_user_settings.push(c); }

    // Channel orders
    let mut stmt = conn.prepare("SELECT source_id, channel_id, position FROM channel_orders").map_err(|e| e.to_string())?;
    let co_iter = stmt.query_map([], |row| {
        Ok(ChannelOrderExport {
            source_id: row.get(0)?,
            channel_id: row.get(1)?,
            position: row.get(2)?,
        })
    }).map_err(|e| e.to_string())?;
    let mut channel_orders = Vec::new();
    for c in co_iter.flatten() { channel_orders.push(c); }

    // Channel sort presets
    let mut stmt = conn.prepare("SELECT id, name, source_id, channel_order, created_at FROM channel_sort_presets").map_err(|e| e.to_string())?;
    let csp_iter = stmt.query_map([], |row| {
        let order_str: String = row.get(3)?;
        let channel_order: Vec<String> = serde_json::from_str(&order_str).unwrap_or_default();
        Ok(SortPreset {
            id: row.get(0)?,
            name: row.get(1)?,
            source_id: row.get(2)?,
            channel_order,
            created_at: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;
    let mut channel_sort_presets = Vec::new();
    for p in csp_iter.flatten() { channel_sort_presets.push(p); }

    // Settings (excluding sensitive and internal keys)
    let mut stmt = conn.prepare("SELECT key, value FROM settings").map_err(|e| e.to_string())?;
    let s_iter = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).map_err(|e| e.to_string())?;
    let mut settings = HashMap::new();
    for (k, v) in s_iter.flatten() {
        if !is_internal_or_protected_setting_key(&k) {
            settings.insert(k, v);
        }
    }

    let exported_at: String = conn.query_row("SELECT datetime('now', 'localtime')", [], |r| r.get(0)).unwrap_or_default();

    Ok(BackupData {
        version: 1,
        exported_at,
        sources,
        favorites,
        channel_user_settings,
        channel_orders,
        channel_sort_presets,
        settings,
    })
}

#[tauri::command]
async fn export_settings(state: tauri::State<'_, AppState>) -> Result<BackupData, String> {
    let conn = state.db.lock().await;
    export_settings_from_conn(&conn)
}

pub(crate) fn import_settings_into_conn(conn: &mut Connection, backup: BackupData) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    // Sources
    for (idx, s) in backup.sources.into_iter().enumerate() {
        let sort_pos = s.sort_position.unwrap_or(idx as i64);
        tx.execute(
            "INSERT INTO sources (id, name, url, last_downloaded, country_code, time_offset, sort_position) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET 
                name = excluded.name, 
                url = excluded.url, 
                last_downloaded = COALESCE(excluded.last_downloaded, sources.last_downloaded),
                country_code = excluded.country_code,
                time_offset = excluded.time_offset,
                sort_position = excluded.sort_position",
            params![s.id, s.name, s.url, s.last_downloaded, s.country_code, s.time_offset, sort_pos],
        ).map_err(|e| format!("Forrás importálási hiba: {}", e))?;
    }

    // Favorites
    tx.execute("DELETE FROM favorites", [])
        .map_err(|e| format!("Kedvencek törlési hiba: {}", e))?;
    for fav in backup.favorites {
        tx.execute(
            "INSERT OR REPLACE INTO favorites (title) VALUES (?1)",
            params![fav],
        ).map_err(|e| format!("Kedvenc importálási hiba: {}", e))?;
    }

    // Channel user settings
    for cus in backup.channel_user_settings {
        let is_fav = if cus.is_favorite { 1 } else { 0 };
        let is_hid = if cus.is_hidden { 1 } else { 0 };
        tx.execute(
            "INSERT INTO channel_user_settings (source_id, channel_id, is_favorite, is_hidden, custom_icon, custom_name)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(source_id, channel_id) DO UPDATE SET
                is_favorite = excluded.is_favorite,
                is_hidden = excluded.is_hidden,
                custom_icon = excluded.custom_icon,
                custom_name = excluded.custom_name",
            params![cus.source_id, cus.channel_id, is_fav, is_hid, cus.custom_icon, cus.custom_name],
        ).map_err(|e| format!("Csatorna beállítás importálási hiba: {}", e))?;
    }

    // Channel orders
    for co in backup.channel_orders {
        tx.execute(
            "INSERT INTO channel_orders (source_id, channel_id, position)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(source_id, channel_id) DO UPDATE SET
                position = excluded.position",
            params![co.source_id, co.channel_id, co.position],
        ).map_err(|e| format!("Csatornasorrend importálási hiba: {}", e))?;
    }

    // Channel sort presets
    if !backup.channel_sort_presets.is_empty() {
        tx.execute("DELETE FROM channel_sort_presets", [])
            .map_err(|e| format!("Rendezési preset törlési hiba: {}", e))?;
        for csp in backup.channel_sort_presets {
            let order_json = serde_json::to_string(&csp.channel_order).unwrap_or_else(|_| "[]".to_string());
            tx.execute(
                "INSERT INTO channel_sort_presets (id, name, source_id, channel_order, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![csp.id, csp.name, csp.source_id, order_json, csp.created_at],
            ).map_err(|e| format!("Rendezési preset importálási hiba: {}", e))?;
        }
    }

    // Settings
    for (k, v) in backup.settings {
        if is_internal_or_protected_setting_key(&k) {
            continue;
        }
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![k, v],
        ).map_err(|e| format!("Beállítás importálási hiba: {}", e))?;
    }

    tx.commit().map_err(|e| format!("Import mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn import_settings(state: tauri::State<'_, AppState>, backup: BackupData) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    import_settings_into_conn(&mut conn, backup)
}

pub(crate) fn get_settings_from_conn(conn: &Connection) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    for (k, v) in rows.flatten() {
        if !is_internal_or_protected_setting_key(&k) {
            map.insert(k, v);
        }
    }
    Ok(map)
}

#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let conn = state.db.lock().await;
    get_settings_from_conn(&conn)
}

pub(crate) fn set_setting_in_conn(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    if is_internal_or_protected_setting_key(key) {
        return Err("Tiltott beállítási kulcs".to_string());
    }
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn set_setting(state: tauri::State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    set_setting_in_conn(&conn, &key, &value)
}

fn load_language_content_from_conn(conn: &Connection, lang: &str) -> Result<String, String> {
    let normalized = lang.to_lowercase();
    let mut stmt = conn
        .prepare("SELECT content FROM app_languages WHERE lang = ?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query(params![normalized])
        .map_err(|e| e.to_string())?;

    if let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let content: String = row.get(0).map_err(|e| e.to_string())?;
        if !content.trim().is_empty() {
            return Ok(content);
        }
    }

    if let Some(embedded) = get_embedded_language(&normalized) {
        return Ok(embedded.to_string());
    }

    Ok(EMBEDDED_ENGLISH_LNG.to_string())
}

fn extract_app_title(lang_json: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(lang_json).ok()?;
    let title = parsed.get("app_title")?.as_str()?.trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

fn resolve_initial_window_title(conn: &Connection) -> String {
    let lang = conn
        .query_row("SELECT value FROM settings WHERE key = 'language'", [], |r| {
            r.get::<_, String>(0)
        })
        .ok()
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "hungarian".to_string());

    if let Ok(content) = load_language_content_from_conn(conn, &lang) {
        if let Some(title) = extract_app_title(&content) {
            return title;
        }
    }

    get_embedded_language(&lang)
        .and_then(extract_app_title)
        .or_else(|| extract_app_title(EMBEDDED_ENGLISH_LNG))
        .unwrap_or_else(|| "EPG Viewer".to_string())
}

#[tauri::command]
async fn get_language_content(state: tauri::State<'_, AppState>, lang: String) -> Result<String, String> {
    let conn = state.db.lock().await;
    load_language_content_from_conn(&conn, &lang)
}

#[tauri::command]
async fn save_language_content(state: tauri::State<'_, AppState>, lang: String, content: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    let normalized = lang.to_lowercase();
    conn.execute(
        "INSERT INTO app_languages (lang, content, updated_at) VALUES (?1, ?2, datetime('now', 'localtime'))
         ON CONFLICT(lang) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
        params![normalized, content],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_language_files_to_disk(files: &[(&str, &str)]) {
    let mut candidate_dirs = vec![
        PathBuf::from("public/languages"),
        PathBuf::from("../public/languages"),
    ];
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            candidate_dirs.push(parent.join("public/languages"));
            candidate_dirs.push(parent.join("languages"));
        }
    }

    for dir in candidate_dirs {
        if dir.exists() || dir.parent().is_some_and(|p| p.exists()) {
            let _ = fs::create_dir_all(&dir);
            for (lang, content) in files {
                let _ = fs::write(dir.join(format!("{}.lng", lang)), content);
            }
        }
    }
}

#[tauri::command]
async fn download_languages_from_github(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let langs = ["english", "hungarian"];
    let mut downloaded: Vec<(&str, String)> = Vec::with_capacity(langs.len());

    for lang in &langs {
        let url = format!(
            "https://raw.githubusercontent.com/norti/epg-viewer/main/public/languages/{}.lng",
            lang
        );
        let resp = state.http.get(&url).send().await.map_err(|e| format!("Letöltési hiba ({url}): {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("GitHub letöltés sikertelen ({url}): HTTP {}", resp.status()));
        }
        let text = resp.text().await.map_err(|e| format!("Válasz olvasási hiba ({url}): {e}"))?;
        let _: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("Érvénytelen JSON ({lang}.lng): {e}"))?;
        downloaded.push((lang, text));
    }

    {
        let conn = state.db.lock().await;
        for (lang, text) in &downloaded {
            conn.execute(
                "INSERT INTO app_languages (lang, content, updated_at) VALUES (?1, ?2, datetime('now', 'localtime'))
                 ON CONFLICT(lang) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
                params![lang, text],
            ).map_err(|e| e.to_string())?;
        }
    }

    let disk_pairs: Vec<(&str, &str)> = downloaded.iter().map(|(l, t)| (*l, t.as_str())).collect();
    write_language_files_to_disk(&disk_pairs);

    let mut result = HashMap::new();
    for (lang, text) in downloaded {
        result.insert(lang.to_string(), text);
    }

    Ok(result)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TmdbDetails {
    pub tmdb_id: i64,
    pub title: String,
    pub hu_title: Option<String>,
    pub en_title: Option<String>,
    pub original_title: Option<String>,
    pub original_language: Option<String>,
    pub media_type: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub overview: Option<String>,
    pub vote_average: Option<f64>,
    pub release_date: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TmdbSearchResultItem {
    pub id: i64,
    pub media_type: String,
    pub title: String,
    pub original_title: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub overview: Option<String>,
    pub vote_average: Option<f64>,
    pub release_date: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TmdbSearchResponse {
    results: Option<Vec<TmdbRawItem>>,
}

#[derive(Deserialize, Debug)]
struct TmdbRawItem {
    id: i64,
    media_type: Option<String>,
    title: Option<String>,
    name: Option<String>,
    original_title: Option<String>,
    original_name: Option<String>,
    original_language: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    vote_average: Option<f64>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    #[serde(default)]
    translations: Option<TmdbTranslationsResponse>,
}

#[derive(Deserialize, Debug)]
struct TmdbTranslationsResponse {
    translations: Option<Vec<TmdbTranslationItem>>,
}

#[derive(Deserialize, Debug)]
struct TmdbTranslationItem {
    iso_639_1: String,
    data: Option<TmdbTranslationData>,
}

#[derive(Deserialize, Debug)]
struct TmdbTranslationData {
    title: Option<String>,
    name: Option<String>,
    overview: Option<String>,
}

static RE_TITLE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[\{]\s*(?:19|20)\d{2}\s*[\)\]\}]").unwrap());
static RE_TITLE_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[\(\[\{]\s*(?:12|16|18|BA|KN|HD|UHD|FHD|4K|3D|élő|live|ism\.?|premier|új|uj|feliratos|szinkronizált|magyarul beszélő[^)]*)\s*[\)\]\}]").unwrap());
static RE_TITLE_HU_SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[-–—:]?\s*(?:első|második|harmadik|negyedik|ötödik|hatodik|hetedik|nyolcadik|kilencedik|tizedik)\s+évad.*$").unwrap());
static RE_TITLE_EP1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*[-–—:]\s*(?:s\d+(?:e\d+)?|\d+[\.:]?\s*(?:rész|évad|epizód)|season\s*\d+|episode\s*\d+|ep\.\s*\d+).*$").unwrap());
static RE_TITLE_EP2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+(?:s\d+e\d+|\d+[\.:]?\s*(?:rész|évad|epizód)|season\s*\d+|episode\s*\d+|ep\.\s*\d+).*$").unwrap());
static RE_TITLE_ROMAN_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*(?:[-–—:]\s*)?\b(?:[IVXLCDM]+)\.?\s*(?:rész|évad|epizód).*$").unwrap());
static RE_TITLE_SLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*(?:[-–—:]\s*)?\b(?:[IVXLCDM]+|\d+)\.?\s*/\s*\d+[\.:]?(?:\s*(?:rész|évad|epizód))?.*$").unwrap());
static RE_TITLE_TAIL_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[\(\[\{][^\(\)\[\]\{\}]*[\)\]\}]\s*$").unwrap());
static RE_YEAR_HINT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\(\[]\s*((?:19|20)\d{2})\s*[\)\]]").unwrap());

pub(crate) fn clean_title(title: &str) -> String {
    let mut s = title.replace('\u{a0}', " ").trim().to_string();

    // 1. Remove year in parentheses / brackets e.g. (1999), [2021]
    s = RE_TITLE_YEAR.replace_all(&s, "").to_string();

    // 2. Remove rating and tech tags e.g. (12), (16), (18), (HD), (élő), (ism.), etc.
    s = RE_TITLE_TAGS.replace_all(&s, "").to_string();

    // 3. Remove written-out Hungarian season names e.g. " - Második évad"
    let trimmed_cand = RE_TITLE_HU_SEASON.replace(&s, "").to_string();
    if trimmed_cand.trim().len() >= 2 {
        s = trimmed_cand;
    }

    // 4. Remove episode / season patterns with delimiter or explicit keyword:
    // e.g. " - 1. rész", " 2. évad", " - S01E02", " - Ep. 5", " 1. epizód"
    let trimmed_cand = RE_TITLE_EP1.replace(&s, "").to_string();
    if trimmed_cand.trim().len() >= 2 {
        s = trimmed_cand;
    }

    let trimmed_cand = RE_TITLE_EP2.replace(&s, "").to_string();
    if trimmed_cand.trim().len() >= 2 {
        s = trimmed_cand;
    }

    // 5. Roman numeral season/part WITH explicit keyword only (e.g. " - I. évad", " II. rész")
    // Requires word boundary \b and explicit keyword to prevent matching words starting with Roman letters (like "Das...", "Megtorlás")
    let trimmed_cand = RE_TITLE_ROMAN_PART.replace(&s, "").to_string();
    if trimmed_cand.trim().len() >= 2 {
        s = trimmed_cand;
    }

    // 6. Season / episode slash numbering e.g. " I./3.", " I/3.", " - I./3.", " 1./3.", " 1/3", " II/4. rész"
    let trimmed_cand = RE_TITLE_SLASH.replace(&s, "").to_string();
    if trimmed_cand.trim().len() >= 2 {
        s = trimmed_cand;
    }

    // 7. Remove trailing generic parenthesis/bracket tag if it still exists at the end
    let trimmed_cand = RE_TITLE_TAIL_PAREN.replace(&s, "").to_string();
    if trimmed_cand.trim().len() >= 2 {
        s = trimmed_cand;
    }

    // 8. Remove trailing punctuation: dashes, colons, dots, slashes, commas
    s = s.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | ':' | ',' | '.' | '/' | '\\')).to_string();

    // 9. Collapse multiple spaces
    s = RE_SPACES.replace_all(&s, " ").to_string();

    s = s.trim().to_string();
    if s.len() < 2 {
        title.trim().to_string()
    } else {
        s
    }
}

// Extract a 4-digit year from parentheses/brackets in a raw EPG title, e.g. "Film (2018)" → Some(2018).
// Called BEFORE clean_title() so the year is not yet stripped.
pub(crate) fn extract_year_hint(raw_title: &str) -> Option<u32> {
    RE_YEAR_HINT
        .captures(raw_title)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

fn fold_diacritic(c: char) -> &'static str {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => "a",
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => "e",
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => "i",
        'ó' | 'ò' | 'ô' | 'ö' | 'ő' | 'õ' | 'ø' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Ő' | 'Õ' | 'Ø' => "o",
        'ú' | 'ù' | 'û' | 'ü' | 'ű' | 'Ú' | 'Ù' | 'Û' | 'Ü' | 'Ű' => "u",
        'ç' | 'č' | 'ć' | 'Ç' | 'Č' | 'Ć' => "c",
        'ñ' | 'ň' | 'ń' | 'Ñ' | 'Ň' | 'Ń' => "n",
        'ý' | 'ÿ' | 'Ý' | 'Ÿ' => "y",
        'ž' | 'ź' | 'ż' | 'Ž' | 'Ź' | 'Ż' => "z",
        'š' | 'ś' | 'ş' | 'Š' | 'Ś' | 'Ş' => "s",
        'ř' | 'ŕ' | 'Ř' | 'Ŕ' => "r",
        'ď' | 'đ' | 'Ď' | 'Đ' => "d",
        'ť' | 'ţ' | 'Ť' | 'Ţ' => "t",
        'ł' | 'ľ' | 'ĺ' | 'Ł' | 'Ľ' | 'Ĺ' => "l",
        'ß' => "ss",
        _ => "",
    }
}

// Lowercase + fold diacritics + keep only alphanumeric chars and spaces, then collapse whitespace.
pub fn normalise_for_match(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        let folded = fold_diacritic(c);
        if !folded.is_empty() {
            out.push_str(folded);
            prev_space = false;
        } else if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    out.trim().to_string()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn xmltv_to_epoch(ts: &str) -> Option<i64> {
    let trimmed = ts.trim();
    if trimmed.len() < 12 {
        return None;
    }
    let digits_end = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let dt_part = &trimmed[..digits_end];
    if dt_part.len() < 12 {
        return None;
    }

    let y: i64 = dt_part.get(0..4)?.parse().ok()?;
    let m: i64 = dt_part.get(4..6)?.parse().ok()?;
    let d: i64 = dt_part.get(6..8)?.parse().ok()?;
    let h: i64 = dt_part.get(8..10)?.parse().ok()?;
    let min: i64 = dt_part.get(10..12)?.parse().ok()?;
    let sec: i64 = if dt_part.len() >= 14 {
        dt_part.get(12..14)?.parse().ok()?
    } else {
        0
    };

    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 || min > 59 || sec > 59 {
        return None;
    }

    let mut tz_offset_secs: i64 = 0;
    let rest = trimmed[digits_end..].trim();
    if !rest.is_empty() && !rest.eq_ignore_ascii_case("Z") {
        let sign = if rest.starts_with('-') {
            -1
        } else if rest.starts_with('+') {
            1
        } else {
            0
        };
        if sign != 0 {
            let tz_digits: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            if tz_digits.len() >= 4 {
                let tz_h: i64 = tz_digits[0..2].parse().ok()?;
                let tz_m: i64 = tz_digits[2..4].parse().ok()?;
                tz_offset_secs = sign * (tz_h * 3600 + tz_m * 60);
            }
        }
    }

    let days = days_from_civil(y, m, d);
    Some(days * 86400 + h * 3600 + min * 60 + sec - tz_offset_secs)
}

// Score how well a TMDB candidate title matches the cleaned query.
// Returns a value in roughly [0.0, 1.5]; higher is better.
pub(crate) fn tmdb_title_score(
    query: &str,
    cand_title: &str,
    cand_orig: Option<&str>,
    year_hint: Option<u32>,
    cand_year: Option<u32>,
) -> f64 {
    let nq = normalise_for_match(query);
    let nc = normalise_for_match(cand_title);
    let no = cand_orig.map(normalise_for_match);

    let jaccard = |a: &str, b: &str| -> f64 {
        if a.is_empty() && b.is_empty() { return 1.0; }
        if a.is_empty() || b.is_empty() { return 0.0; }
        let ta: std::collections::HashSet<&str> = a.split_whitespace().collect();
        let tb: std::collections::HashSet<&str> = b.split_whitespace().collect();
        let inter = ta.intersection(&tb).count() as f64;
        let denom = ta.len().max(tb.len()) as f64;
        inter / denom
    };

    let mut score = jaccard(&nq, &nc);
    let jaro = strsim::jaro_winkler(&nq, &nc);
    score = score.max(jaro * 0.9);

    if let Some(ref no_str) = no {
        score = score.max(jaccard(&nq, no_str));
        let jaro_orig = strsim::jaro_winkler(&nq, no_str);
        score = score.max(jaro_orig * 0.9);
    }

    // Bonus: exact substring match (query inside title or vice versa)
    if !nq.is_empty() && !nc.is_empty() && (nc.contains(nq.as_str()) || nq.contains(nc.as_str())) {
        score += 0.2;
    } else if let Some(ref no_str) = no {
        if !nq.is_empty() && !no_str.is_empty() && (no_str.contains(nq.as_str()) || nq.contains(no_str.as_str())) {
            score += 0.2;
        }
    }

    // Year hint bonus/penalty for franchise disambiguation
    if let Some(hint) = year_hint {
        if let Some(cy) = cand_year {
            if cy == hint {
                score += 0.5;
            } else {
                score -= 0.3;
            }
        }
        // No penalty when candidate has no year (don't penalise old uncatalogued entries)
    }

    score
}

// Pick the best movie/tv candidate from a list by title similarity.
// Tie-breaks by original TMDB relevance order (position).
fn best_tmdb_match(
    candidates: Vec<TmdbRawItem>,
    clean_query: &str,
    year_hint: Option<u32>,
) -> Option<TmdbRawItem> {
    let scored: Vec<(usize, f64, TmdbRawItem)> = candidates
        .into_iter()
        .enumerate()
        .filter(|(_, item)| matches!(item.media_type.as_deref(), Some("movie") | Some("tv")))
        .map(|(pos, item)| {
            let cand_title = item.title.as_deref()
                .or(item.name.as_deref())
                .unwrap_or("");
            let cand_orig = item.original_title.as_deref()
                .or(item.original_name.as_deref());
            let cand_year: Option<u32> = item.release_date.as_deref()
                .or(item.first_air_date.as_deref())
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse().ok());
            let s = tmdb_title_score(clean_query, cand_title, cand_orig, year_hint, cand_year);
            (pos, s, item)
        })
        .collect();

    // Best score wins; tie-break by lowest position (TMDB relevance order)
    scored.into_iter().max_by(|a, b| {
        a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            .then(b.0.cmp(&a.0)) // lower position wins on tie
    }).map(|(_, _, item)| item)
}

fn is_tmdb_v4_key(key: &str) -> bool {
    key.starts_with("eyJ") || key.len() > 40
}

pub(crate) fn apply_tmdb_auth(req: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder {
    if is_tmdb_v4_key(api_key) {
        req.header("Authorization", format!("Bearer {}", api_key))
    } else {
        req.query(&[("api_key", api_key)])
    }
}

fn build_tmdb_search_req(
    state: &AppState,
    api_key: &str,
    query: &str,
    lang: &str,
) -> reqwest::RequestBuilder {
    let req = state.http.get(format!("{}/search/multi", state.tmdb.base))
        .header("Accept", "application/json")
        .query(&[("query", query), ("language", lang), ("include_adult", "false")]);
    apply_tmdb_auth(req, api_key)
}

/// Why a TMDB request failed. Only a successful (2xx) answer may ever be turned into cache rows;
/// every `TmdbError` must leave the cache untouched (a transient 429/5xx must not become a permanent "not found").
#[derive(Debug, Clone)]
pub(crate) enum TmdbError {
    Unauthorized,
    NotFound,
    RateLimited,
    Transient(String),
}

impl TmdbError {
    fn into_user_message(self, context: &str) -> String {
        match self {
            TmdbError::Unauthorized => "Érvénytelen TMDB API kulcs / Invalid TMDB API key".to_string(),
            TmdbError::NotFound => format!("{}: nem található (404)", context),
            TmdbError::RateLimited => format!("{}: TMDB sebességkorlát (429), próbáld később", context),
            TmdbError::Transient(m) => format!("{}: {}", context, m),
        }
    }
}

const TMDB_MAX_RATE_LIMIT_RETRIES: u32 = 2;
const TMDB_MAX_SERVER_RETRIES: u32 = 1;
const TMDB_MAX_RETRY_AFTER_SECS: u64 = 10;
/// Automatic "no match" rows are searched again after this many days. Manual rows never expire.
const TMDB_NEGATIVE_TTL_DAYS: i64 = 7;

/// Sends a TMDB GET (rebuilt via `make_req` for every attempt) and returns the body of a 2xx answer.
///  * 401 -> `Unauthorized`, 404 -> `NotFound` (no retry)
///  * 429 -> waits `Retry-After` (capped), at most `TMDB_MAX_RATE_LIMIT_RETRIES` retries, then `RateLimited`
///  * 5xx / network error -> one retry after `state.tmdb.retry_delay`, then `Transient`
///
/// The concurrency permit is held for the whole call (including waits) but never together with the DB lock.
pub(crate) async fn tmdb_get_text<F>(
    state: &AppState,
    flight_key: &str,
    make_req: F,
) -> Result<String, TmdbError>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    trakt::client::single_flight_shared(&state.tmdb.http_inflight, flight_key, || async {
        let _permit = state
            .tmdb
            .sem
            .acquire()
            .await
            .map_err(|_| TmdbError::Transient("TMDB limiter closed".to_string()))?;

        let mut attempt: u32 = 0;
        loop {
            state.stats.tmdb_net.fetch_add(1, Ordering::Relaxed);
            match make_req().send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return resp
                            .text()
                            .await
                            .map_err(|e| TmdbError::Transient(e.to_string()));
                    }
                    match status.as_u16() {
                        401 => return Err(TmdbError::Unauthorized),
                        404 => return Err(TmdbError::NotFound),
                        429 => {
                            state.stats.tmdb_429.fetch_add(1, Ordering::Relaxed);
                            if attempt >= TMDB_MAX_RATE_LIMIT_RETRIES {
                                return Err(TmdbError::RateLimited);
                            }
                            let wait_secs = resp
                                .headers()
                                .get(reqwest::header::RETRY_AFTER)
                                .and_then(|v| v.to_str().ok())
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(1)
                                .min(TMDB_MAX_RETRY_AFTER_SECS);
                            tokio::time::sleep(std::time::Duration::from_secs(wait_secs)).await;
                        }
                        s if s >= 500 => {
                            if attempt >= TMDB_MAX_SERVER_RETRIES {
                                return Err(TmdbError::Transient(format!("HTTP {}", s)));
                            }
                            tokio::time::sleep(state.tmdb.retry_delay).await;
                        }
                        s => return Err(TmdbError::Transient(format!("HTTP {}", s))),
                    }
                }
                Err(e) => {
                    if attempt >= TMDB_MAX_SERVER_RETRIES {
                        return Err(TmdbError::Transient(e.to_string()));
                    }
                    tokio::time::sleep(state.tmdb.retry_delay).await;
                }
            }
            attempt += 1;
        }
    })
    .await
}

/// `search/multi` -> raw candidates. Any failure (HTTP or unparsable 2xx body) is an `Err`, never an empty list.
async fn tmdb_search_items(
    state: &AppState,
    api_key: &str,
    query: &str,
    lang: &str,
) -> Result<Vec<TmdbRawItem>, String> {
    let flight_key = format!("search:{}:{}", lang, query);
    let text = tmdb_get_text(state, &flight_key, || build_tmdb_search_req(state, api_key, query, lang))
        .await
        .map_err(|e| e.into_user_message("TMDB keresési hiba"))?;
    let parsed: TmdbSearchResponse =
        serde_json::from_str(&text).map_err(|e| format!("TMDB JSON hiba: {}", e))?;
    Ok(parsed.results.unwrap_or_default())
}

/// The user's own "this title is NOT that item" marker (clear_tmdb_match). Never expires.
fn store_manual_none(conn: &Connection, cache_key: &str, lang: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at) VALUES (?1, ?2, 'none', 1, datetime('now', 'localtime'))",
        params![cache_key, lang],
    )
}

/// "Clear TMDB cache": manual matches and the user's "not this title" markers survive.
pub(crate) fn clear_automatic_tmdb_cache(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute("DELETE FROM tmdb_cache WHERE is_manual = 0", [])
}

pub(crate) fn save_tmdb_api_key_with_store(
    conn: &Connection,
    store: &dyn trakt::secret::SecretStore,
    api_key: &str,
) -> Result<(), String> {
    let clean_key = api_key.trim();
    if clean_key.is_empty() {
        conn.execute(
            "DELETE FROM settings WHERE key IN (?1, ?2)",
            params![TMDB_SECURE_KEY, LEGACY_TMDB_API_KEY],
        )
        .map_err(|e| e.to_string())?;
    } else {
        let blob = trakt::secret::protect_str(store, clean_key)?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![TMDB_SECURE_KEY, blob],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            params![LEGACY_TMDB_API_KEY],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn load_tmdb_api_key_with_store(
    conn: &Connection,
    store: &dyn trakt::secret::SecretStore,
) -> Option<String> {
    let blob: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![TMDB_SECURE_KEY],
            |row| row.get(0),
        )
        .ok();
    blob.and_then(|b| {
        if b.trim().is_empty() {
            return None;
        }
        trakt::secret::unprotect_str(store, &b)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Migrates any legacy plaintext `tmdb_api_key` in `settings` to DPAPI-encrypted `tmdb_secure_key`
/// and deletes the plaintext row.
pub(crate) fn migrate_plaintext_tmdb_api_key(
    conn: &mut Connection,
    store: &dyn trakt::secret::SecretStore,
) -> Result<(), String> {
    let legacy: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![LEGACY_TMDB_API_KEY],
            |r| r.get(0),
        )
        .ok();
    let Some(raw_key) = legacy else {
        return Ok(());
    };

    let has_secure: bool = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![TMDB_SECURE_KEY],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);

    let clean = raw_key.trim();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if !has_secure && !clean.is_empty() {
        let blob = trakt::secret::protect_str(store, clean)?;
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![TMDB_SECURE_KEY, blob],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![LEGACY_TMDB_API_KEY],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) async fn get_tmdb_api_key_from_state(state: &AppState) -> Option<String> {
    {
        let r = state.tmdb.api_key_cache.read().await;
        if let Some(ref cached) = *r {
            return cached.as_ref().map(|z| z.as_str().to_string());
        }
    }

    let loaded = {
        let store = trakt::secret::default_secret_store();
        let conn = state.db.lock().await;
        load_tmdb_api_key_with_store(&conn, store.as_ref())
    };

    let mut w = state.tmdb.api_key_cache.write().await;
    *w = Some(loaded.clone().map(zeroize::Zeroizing::new));
    loaded
}

#[tauri::command]
async fn get_tmdb_api_key(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(get_tmdb_api_key_from_state(state.inner()).await)
}

#[tauri::command]
async fn set_tmdb_api_key(state: tauri::State<'_, AppState>, api_key: String) -> Result<(), String> {
    {
        let store = trakt::secret::default_secret_store();
        let conn = state.db.lock().await;
        save_tmdb_api_key_with_store(&conn, store.as_ref(), &api_key)?;
    }
    let clean = api_key.trim();
    let mut w = state.tmdb.api_key_cache.write().await;
    *w = Some(if clean.is_empty() {
        None
    } else {
        Some(zeroize::Zeroizing::new(clean.to_string()))
    });
    Ok(())
}

#[tauri::command]
async fn clear_tmdb_cache(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().await;
    clear_automatic_tmdb_cache(&conn).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn test_tmdb_api_key(state: tauri::State<'_, AppState>, api_key: String) -> Result<bool, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(false);
    }
    let req = apply_tmdb_auth(
        state.http.get(format!("{}/authentication", state.tmdb.base))
            .header("Accept", "application/json"),
        key,
    );

    match req.send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(e) => Err(format!("Hálózati hiba: {}", e)),
    }
}

#[tauri::command]
async fn get_tmdb_details(state: tauri::State<'_, AppState>, title: String, lang: Option<String>) -> Result<Option<TmdbDetails>, String> {
    get_tmdb_details_inner(state.inner(), title, lang).await
}

pub(crate) async fn get_tmdb_details_inner(
    state: &AppState,
    title: String,
    lang: Option<String>,
) -> Result<Option<TmdbDetails>, String> {
    // Extract year hint from raw title BEFORE cleaning strips it (used for scoring later)
    let year_hint = extract_year_hint(&title);
    let clean = clean_title(&title);
    if clean.is_empty() {
        return Ok(None);
    }

    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());
    let flight_key = format!("search:{}:{}", target_lang, clean);

    trakt::client::single_flight(&state.tmdb.cache_inflight, &flight_key, |_waited| async move {
        // 1. Check local cache
        let cached = {
            let conn = state.db.lock().await;
            conn.query_row(
            "SELECT tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date,
                    (media_type = 'none' AND is_manual = 0 AND updated_at < datetime('now', 'localtime', ?3)) AS stale_negative
             FROM tmdb_cache WHERE query_title = ?1 AND lang = ?2",
            params![clean, target_lang, format!("-{} days", TMDB_NEGATIVE_TTL_DAYS)],
            |row| {
                let tmdb_id: Option<i64> = row.get(0)?;
                let media_type: Option<String> = row.get(1)?;
                let title: Option<String> = row.get(2)?;
                let hu_title: Option<String> = row.get(3)?;
                let en_title: Option<String> = row.get(4)?;
                let original_title: Option<String> = row.get(5)?;
                let original_language: Option<String> = row.get(6)?;
                let poster_path: Option<String> = row.get(7)?;
                let backdrop_path: Option<String> = row.get(8)?;
                let overview: Option<String> = row.get(9)?;
                let vote_average: Option<f64> = row.get(10)?;
                let release_date: Option<String> = row.get(11)?;
                let stale_negative: Option<i64> = row.get(12)?;
                Ok((tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date, stale_negative))
            }
        ).ok()
        };

        if let Some((tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date, stale_negative)) = cached {
            let is_negative = media_type.as_deref() == Some("none") || tmdb_id.is_none();
            // An automatic negative older than TMDB_NEGATIVE_TTL_DAYS is searched again.
            // Manual rows (the user's "not this title" marker included) never expire.
            let expired_negative = is_negative && stale_negative.unwrap_or(0) == 1;
            if !expired_negative {
                state.stats.tmdb_cache_hit.fetch_add(1, Ordering::Relaxed);
                if is_negative {
                    return Ok(None);
                }
                return Ok(Some(TmdbDetails {
                    tmdb_id: tmdb_id.unwrap_or(0),
                    title: title.unwrap_or_else(|| clean.clone()),
                    hu_title,
                    en_title,
                    original_title,
                    original_language,
                    media_type,
                    poster_path,
                    backdrop_path,
                    overview,
                    vote_average,
                    release_date,
                }));
            }
        }

        // 1b. Sibling-language lookup for full app language switches (0 HTTP calls when the other
        //     language row already carries both title and overview, or a manual negative marker).
        {
            let conn = state.db.lock().await;
            let sibling: Option<(Option<i64>, Option<String>, i64)> = conn
                .query_row(
                    "SELECT tmdb_id, media_type, is_manual
                     FROM tmdb_cache
                     WHERE query_title = ?1 AND lang != ?2
                       AND ((media_type = 'none' AND is_manual = 1)
                            OR (tmdb_id IS NOT NULL AND media_type IN ('movie', 'tv')))
                     ORDER BY is_manual DESC, updated_at DESC
                     LIMIT 1",
                    params![clean, target_lang],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .ok();
            if let Some((sib_id, sib_mt, sib_manual)) = sibling {
                if sib_mt.as_deref() == Some("none") && sib_manual != 0 {
                    state.stats.tmdb_cache_hit.fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }
                if let (Some(tid), Some(mt)) = (sib_id, sib_mt.as_deref()) {
                    if let Some((details, hu_over, en_over)) =
                        derive_details_from_sibling_lang(&conn, tid, mt, &target_lang)
                    {
                        let _ = write_tmdb_row(
                            &conn,
                            &clean,
                            &target_lang,
                            &details,
                            &hu_over,
                            &en_over,
                            sib_manual != 0,
                        );
                        state.stats.tmdb_cache_hit.fetch_add(1, Ordering::Relaxed);
                        return Ok(Some(details));
                    }
                }
            }
        }

        // 2. Retrieve TMDB API key from settings
        let api_key = match get_tmdb_api_key_from_state(state).await {
            Some(k) => k,
            _ => return Ok(None),
        };

        // 3. Search. A failed request is an Err and is NEVER cached; only a successful answer with no
        //    acceptable candidate becomes an (expiring) negative row.
        let search_res = tmdb_search_items(state, &api_key, &clean, &target_lang).await?;

        // Pick best-matching movie/tv candidate instead of blindly taking the first result
        let mut matched_item = best_tmdb_match(search_res, &clean, year_hint);

        if matched_item.is_none() && target_lang != "en-US" {
            let fallback = tmdb_search_items(state, &api_key, &clean, "en-US").await?;
            matched_item = best_tmdb_match(fallback, &clean, year_hint);
        }

        let item = match matched_item {
            Some(it) => it,
            None => {
                let conn = state.db.lock().await;
                conn.execute(
                    "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at) VALUES (?1, ?2, 'none', 0, datetime('now', 'localtime'))",
                    params![clean, target_lang],
                ).ok();
                return Ok(None);
            }
        };

        let details = fetch_tmdb_item_details(
            state,
            item,
            &clean,
            &clean,
            &target_lang,
            &api_key,
            false,
        ).await?;

        Ok(Some(details))
    })
    .await
}

/// Builds `TmdbDetails` from a raw TMDB item (+ translations) and stores it under `cache_key`.
///  * `fallback_title` is only used when TMDB returns no title; it is deliberately separate from the cache key.
///  * `is_manual` marks a row written by the user's own match (set_tmdb_match).
///  * If the translations request failed, the partial details are still returned (the UI can show them)
///    but nothing is cached, so the next lookup tries again instead of keeping incomplete data forever.
async fn fetch_tmdb_item_details(
    state: &AppState,
    mut item: TmdbRawItem,
    cache_key: &str,
    fallback_title: &str,
    target_lang: &str,
    api_key: &str,
    is_manual: bool,
) -> Result<TmdbDetails, String> {
    let tmdb_id = item.id;
    let media_type = item.media_type.take().unwrap_or_else(|| "movie".to_string());
    let display_title = item.title.clone().or_else(|| item.name.clone()).unwrap_or_else(|| fallback_title.to_string());
    let orig_title = item.original_title.clone().or_else(|| item.original_name.clone());
    let rel_date = item.release_date.clone().or_else(|| item.first_air_date.clone());

    let mut hu_title: Option<String> = None;
    let mut en_title: Option<String> = None;
    let mut hu_overview: Option<String> = None;
    let mut en_overview: Option<String> = None;
    let mut translations_complete = true;

    // An English-original item searched in Hungarian already carries everything /translations would add:
    // the Hungarian title/overview come from the hu-HU search hit, the English title is the original one.
    // (Other combinations still need the translations: e.g. a hu title for an en-US search.)
    let translations_redundant = item.translations.is_none()
        && item.original_language.as_deref() == Some("en")
        && target_lang.starts_with("hu");

    let trans_data_opt = if let Some(embedded) = item.translations.take() {
        Some(embedded)
    } else if translations_redundant {
        None
    } else {
        let trans_url = format!("{}/{}/{}/translations", state.tmdb.base, media_type, tmdb_id);
        let trans_key = format!("translations:{}:{}", media_type, tmdb_id);
        match tmdb_get_text(state, &trans_key, || {
            apply_tmdb_auth(
                state.http.get(&trans_url).header("Accept", "application/json"),
                api_key,
            )
        })
        .await
        {
            Ok(text) => match serde_json::from_str::<TmdbTranslationsResponse>(&text) {
                Ok(parsed) => Some(parsed),
                Err(_) => {
                    translations_complete = false;
                    None
                }
            },
            Err(TmdbError::Unauthorized) => {
                return Err(TmdbError::Unauthorized.into_user_message("TMDB"));
            }
            Err(_) => {
                translations_complete = false;
                None
            }
        }
    };

    if let Some(trans_data) = trans_data_opt {
        if let Some(translations) = trans_data.translations {
            for t in translations {
                if t.iso_639_1 == "hu" {
                    if let Some(d) = t.data {
                        let t_title = d.title.or(d.name).filter(|s| !s.trim().is_empty());
                        if t_title.is_some() {
                            hu_title = t_title;
                        }
                        let t_over = d.overview.filter(|s| !s.trim().is_empty());
                        if t_over.is_some() {
                            hu_overview = t_over;
                        }
                    }
                } else if t.iso_639_1 == "en" {
                    if let Some(d) = t.data {
                        let t_title = d.title.or(d.name).filter(|s| !s.trim().is_empty());
                        if t_title.is_some() {
                            en_title = t_title;
                        }
                        let t_over = d.overview.filter(|s| !s.trim().is_empty());
                        if t_over.is_some() {
                            en_overview = t_over;
                        }
                    }
                }
            }
        }
    }

    if hu_title.is_none() && target_lang.starts_with("hu") {
        hu_title = Some(display_title.clone());
    }
    if en_title.is_none() {
        if target_lang.starts_with("en") {
            en_title = Some(display_title.clone());
        } else {
            en_title = orig_title.clone();
        }
    }

    // Both language overviews go into the row, so a UI language switch needs no request.
    // The search hit's overview is in the *target* language; use it where the translations did not provide one.
    let search_overview = item.overview.clone().filter(|s| !s.trim().is_empty());
    let hu_overview_col = hu_overview
        .clone()
        .or_else(|| search_overview.clone().filter(|_| target_lang.starts_with("hu")));
    let en_overview_col = en_overview
        .clone()
        .or_else(|| search_overview.clone().filter(|_| target_lang.starts_with("en")));

    let final_overview = if target_lang.starts_with("hu") {
        hu_overview.or(search_overview)
    } else if target_lang.starts_with("en") {
        search_overview.or(en_overview)
    } else {
        search_overview
    };

    let details = TmdbDetails {
        tmdb_id,
        title: display_title,
        hu_title,
        en_title,
        original_title: orig_title,
        original_language: item.original_language,
        media_type: Some(media_type),
        poster_path: item.poster_path,
        backdrop_path: item.backdrop_path,
        overview: final_overview,
        vote_average: item.vote_average,
        release_date: rel_date,
    };

    if !translations_complete {
        return Ok(details);
    }

    let conn = state.db.lock().await;
    write_tmdb_row(&conn, cache_key, target_lang, &details, &hu_overview_col, &en_overview_col, is_manual)
        .map_err(|e| e.to_string())?;

    Ok(details)
}

fn write_tmdb_row(
    conn: &Connection,
    cache_key: &str,
    lang: &str,
    details: &TmdbDetails,
    hu_overview: &Option<String>,
    en_overview: &Option<String>,
    is_manual: bool,
) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT OR REPLACE INTO tmdb_cache (
            query_title, lang, tmdb_id, media_type, title, hu_title, en_title, original_title,
            original_language, poster_path, backdrop_path, overview, vote_average, release_date, is_manual,
            hu_overview, en_overview, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now', 'localtime'))",
        params![
            cache_key,
            lang,
            details.tmdb_id,
            details.media_type,
            details.title,
            details.hu_title,
            details.en_title,
            details.original_title,
            details.original_language,
            details.poster_path,
            details.backdrop_path,
            details.overview,
            details.vote_average,
            details.release_date,
            is_manual as i64,
            hu_overview,
            en_overview,
        ],
    )
}

/// Builds the details of `lang` from the row of the *other* UI language (same TMDB item) when that row holds
/// both title and overview of `lang`, so a language switch costs no request. Rows written before the overview
/// columns existed lack them and fall through to the network. Poster, backdrop and rating are language
/// independent and reused. Returns the details plus the overviews to store next to them.
fn derive_details_from_sibling_lang(
    conn: &Connection,
    tmdb_id: i64,
    media_type: &str,
    lang: &str,
) -> Option<(TmdbDetails, Option<String>, Option<String>)> {
    let want_hu = lang.starts_with("hu");
    if !want_hu && !lang.starts_with("en") {
        return None;
    }
    let row: (TmdbDetails, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT hu_title, en_title, original_title, original_language, poster_path, backdrop_path,
                    vote_average, release_date, hu_overview, en_overview
             FROM tmdb_cache
             WHERE tmdb_id = ?1 AND media_type = ?2 AND lang != ?3
               AND (hu_overview IS NOT NULL OR en_overview IS NOT NULL)
             ORDER BY updated_at DESC
             LIMIT 1",
            params![tmdb_id, media_type, lang],
            |r| {
                let hu_title: Option<String> = r.get(0)?;
                let en_title: Option<String> = r.get(1)?;
                let hu_over: Option<String> = r.get(8)?;
                let en_over: Option<String> = r.get(9)?;
                let details = TmdbDetails {
                    tmdb_id,
                    title: String::new(),
                    hu_title,
                    en_title,
                    original_title: r.get(2)?,
                    original_language: r.get(3)?,
                    media_type: Some(media_type.to_string()),
                    poster_path: r.get(4)?,
                    backdrop_path: r.get(5)?,
                    overview: None,
                    vote_average: r.get(6)?,
                    release_date: r.get(7)?,
                };
                Ok((details, hu_over, en_over))
            },
        )
        .ok()?;
    let (mut details, hu_over, en_over) = row;
    let (title, overview) = if want_hu {
        (details.hu_title.clone(), hu_over.clone())
    } else {
        (details.en_title.clone(), en_over.clone())
    };
    details.title = title.filter(|t| !t.trim().is_empty())?;
    details.overview = Some(overview.filter(|o| !o.trim().is_empty())?);
    Some((details, hu_over, en_over))
}

/// Lazy refresh of old automatic id rows (rating, poster, overview change over time). Runs after a sync,
/// outside the read path, and touches at most `limit` rows per call. Manual rows and title mappings
/// (`query_title` without the `~id:` prefix) are never refreshed automatically.
pub(crate) async fn refresh_stale_tmdb(state: &AppState, limit: usize) -> usize {
    if get_tmdb_api_key_from_state(state).await.is_none() {
        return 0;
    }
    let stale: Vec<(i64, String, String)> = {
        let conn = state.db.lock().await;
        let Ok(mut stmt) = conn.prepare(
            "SELECT tmdb_id, media_type, lang FROM tmdb_cache
             WHERE is_manual = 0 AND query_title LIKE '~id:%' AND tmdb_id IS NOT NULL
               AND media_type IN ('tv', 'movie')
               AND updated_at < datetime('now', 'localtime', CASE media_type WHEN 'movie' THEN '-90 days' ELSE '-30 days' END)
             ORDER BY updated_at ASC
             LIMIT ?1",
        ) else {
            return 0;
        };
        let Ok(mapped) = stmt.query_map(params![limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))) else {
            return 0;
        };
        mapped.flatten().collect()
    };

    let mut refreshed = 0;
    for (tmdb_id, media_type, lang) in stale {
        if get_tmdb_by_id(state, tmdb_id, &media_type, &lang, None, true).await.is_ok() {
            refreshed += 1;
        }
    }
    refreshed
}

pub async fn get_tmdb_by_id(
    state: &AppState,
    tmdb_id: i64,
    media_type: &str,
    lang: &str,
    cache_key_override: Option<&str>,
    force_refresh: bool,
) -> Result<TmdbDetails, String> {
    get_tmdb_by_id_inner(state, tmdb_id, media_type, lang, cache_key_override, force_refresh, false).await
}

/// `cache_key_override`: the title key of a title -> item mapping (set_tmdb_match / language switch).
/// Without it the row is stored under an id-only key (`~id:<type>:<id>`): title-keyed rows are
/// *mappings* the matcher reads, and two items sharing a title (a film and a series, a remake)
/// would otherwise collide on the (query_title, lang) primary key and never be cached.
/// `mark_manual`: the row is the user's own choice and is protected from cache clearing / expiry.
async fn get_tmdb_by_id_inner(
    state: &AppState,
    tmdb_id: i64,
    media_type: &str,
    lang: &str,
    cache_key_override: Option<&str>,
    force_refresh: bool,
    mark_manual: bool,
) -> Result<TmdbDetails, String> {
    let flight_key = format!("id:{}:{}:{}", media_type, tmdb_id, lang);
    trakt::client::single_flight(&state.tmdb.cache_inflight, &flight_key, |waited| async move {
        let effective_force = if waited && !mark_manual {
            false
        } else {
            force_refresh
        };
        if !effective_force {
            let conn = state.db.lock().await;
            let cached = if let Some(key) = cache_key_override {
                conn.query_row(
                    "SELECT tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date
                     FROM tmdb_cache WHERE query_title = ?1 AND lang = ?2",
                    params![key, lang],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, Option<f64>>(10)?,
                            row.get::<_, Option<String>>(11)?,
                        ))
                    },
                ).ok()
            } else {
                conn.query_row(
                    "SELECT tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date
                     FROM tmdb_cache WHERE tmdb_id = ?1 AND media_type = ?2 AND lang = ?3 LIMIT 1",
                    params![tmdb_id, media_type, lang],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, Option<String>>(6)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, Option<String>>(9)?,
                            row.get::<_, Option<f64>>(10)?,
                            row.get::<_, Option<String>>(11)?,
                        ))
                    },
                ).ok()
            };

            if let Some((Some(cached_id), media, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date)) = cached {
                if media.as_deref() == Some(media_type) && cached_id == tmdb_id {
                    state.stats.tmdb_cache_hit.fetch_add(1, Ordering::Relaxed);
                    return Ok(TmdbDetails {
                        tmdb_id: cached_id,
                        title: title.unwrap_or_default(),
                        hu_title,
                        en_title,
                        original_title,
                        original_language,
                        media_type: media,
                        poster_path,
                        backdrop_path,
                        overview,
                        vote_average,
                        release_date,
                    });
                }
            }

            // No row for this language: the other language's row may already hold everything (no request).
            if let Some((details, hu_over, en_over)) = derive_details_from_sibling_lang(&conn, tmdb_id, media_type, lang) {
                let key = match cache_key_override {
                    Some(k) => k.to_string(),
                    None => format!("~id:{}:{}", media_type, tmdb_id),
                };
                let _ = write_tmdb_row(&conn, &key, lang, &details, &hu_over, &en_over, mark_manual);
                state.stats.tmdb_cache_hit.fetch_add(1, Ordering::Relaxed);
                return Ok(details);
            }
        }

        let api_key = match get_tmdb_api_key_from_state(state).await {
            Some(k) => k,
            _ => return Err("TMDB API kulcs nincs beállítva / TMDB API key is not configured".to_string()),
        };

        let details_url = format!("{}/{}/{}", state.tmdb.base, media_type, tmdb_id);
        let http_key = format!("id:{}:{}:{}", media_type, tmdb_id, lang);
        let text = tmdb_get_text(state, &http_key, || {
            apply_tmdb_auth(
                state.http.get(&details_url)
                    .header("Accept", "application/json")
                    .query(&[
                        ("language", lang),
                        ("append_to_response", "translations,external_ids"),
                    ]),
                &api_key,
            )
        })
        .await
        .map_err(|e| e.into_user_message("TMDB lekérési hiba"))?;

        let mut raw_item: TmdbRawItem = serde_json::from_str(&text).map_err(|e| format!("JSON hiba: {}", e))?;
        raw_item.media_type = Some(media_type.to_string());

        let fallback_title = raw_item
            .title
            .clone()
            .or_else(|| raw_item.name.clone())
            .unwrap_or_default();
        let cache_key = match cache_key_override {
            Some(k) => k.to_string(),
            None => format!("~id:{}:{}", media_type, tmdb_id),
        };

        fetch_tmdb_item_details(
            state,
            raw_item,
            &cache_key,
            &fallback_title,
            lang,
            &api_key,
            mark_manual,
        )
        .await
    })
    .await
}

#[tauri::command]
async fn get_tmdb_details_for_lang(state: tauri::State<'_, AppState>, 
    tmdb_id: i64,
    media_type: String,
    lang: String,
    query_title: String,
) -> Result<TmdbDetails, String> {
    let cache_key = {
        let clean = clean_title(&query_title);
        if clean.is_empty() { query_title.trim().to_string() } else { clean }
    };
    let key_opt = if cache_key.is_empty() { None } else { Some(cache_key.as_str()) };

    get_tmdb_by_id(state.inner(), tmdb_id, &media_type, &lang, key_opt, false).await
}

pub(crate) async fn search_tmdb_inner(
    state: &AppState,
    query: String,
    lang: Option<String>,
) -> Result<Vec<TmdbSearchResultItem>, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }

    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());

    let api_key = match get_tmdb_api_key_from_state(state).await {
        Some(k) => k,
        _ => return Err("TMDB API kulcs nincs beállítva / TMDB API key is not configured".to_string()),
    };

    let mut results = tmdb_search_items(state, &api_key, &q, &target_lang).await?;

    // If no results and clean_title(q) differs from q, try searching with clean_title
    let cleaned = clean_title(&q);
    if results.is_empty() && !cleaned.is_empty() && cleaned != q {
        results = tmdb_search_items(state, &api_key, &cleaned, &target_lang).await?;
    }

    // Fallback to en-US if still empty and language was not en-US
    if results.is_empty() && target_lang != "en-US" {
        let query_term = if !cleaned.is_empty() { &cleaned } else { &q };
        results = tmdb_search_items(state, &api_key, query_term, "en-US").await?;
    }

    let mut items = Vec::new();
    for it in results {
        let mtype = match it.media_type.as_deref() {
            Some("movie") => "movie".to_string(),
            Some("tv") => "tv".to_string(),
            _ => continue,
        };

        let title = it.title.clone().or_else(|| it.name.clone()).unwrap_or_default();
        let orig_title = it.original_title.clone().or_else(|| it.original_name.clone());
        let rel_date = it.release_date.clone().or_else(|| it.first_air_date.clone());

        items.push(TmdbSearchResultItem {
            id: it.id,
            media_type: mtype,
            title,
            original_title: orig_title,
            poster_path: it.poster_path,
            backdrop_path: it.backdrop_path,
            overview: it.overview,
            vote_average: it.vote_average,
            release_date: rel_date,
        });
    }

    Ok(items)
}

#[tauri::command]
async fn search_tmdb(state: tauri::State<'_, AppState>, query: String, lang: Option<String>) -> Result<Vec<TmdbSearchResultItem>, String> {
    search_tmdb_inner(state.inner(), query, lang).await
}

#[tauri::command]
async fn set_tmdb_match(state: tauri::State<'_, AppState>, 
    app: tauri::AppHandle,
    query_title: String,
    tmdb_id: i64,
    media_type: String,
    lang: Option<String>,
) -> Result<TmdbDetails, String> {
    let clean = clean_title(&query_title);
    let cache_key = if clean.is_empty() { query_title.trim().to_string() } else { clean };
    if cache_key.is_empty() {
        return Err("A megadott cím érvénytelen / Invalid title".to_string());
    }

    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());
    let details = get_tmdb_by_id_inner(state.inner(), tmdb_id, &media_type, &target_lang, Some(&cache_key), true, true).await?;
    trakt::matcher::invalidate_match_cache(state.inner()).await;
    {
        use tauri::Emitter;
        let ver = state.trakt.watchlist_version.load(std::sync::atomic::Ordering::SeqCst);
        let _ = app.emit("trakt-watchlist-changed", ver);
    }
    Ok(details)
}

#[tauri::command]
async fn clear_tmdb_match(state: tauri::State<'_, AppState>, app: tauri::AppHandle, query_title: String, lang: Option<String>) -> Result<(), String> {
    let clean = clean_title(&query_title);
    let cache_key = if clean.is_empty() { query_title.trim().to_string() } else { clean };
    if cache_key.is_empty() {
        return Ok(());
    }
    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());
    {
        let conn = state.db.lock().await;
        store_manual_none(&conn, &cache_key, &target_lang).map_err(|e| e.to_string())?;
    }
    trakt::matcher::invalidate_match_cache(state.inner()).await;
    {
        use tauri::Emitter;
        let ver = state.trakt.watchlist_version.load(std::sync::atomic::Ordering::SeqCst);
        let _ = app.emit("trakt-watchlist-changed", ver);
    }
    Ok(())
}

fn extract_xml_encoding_label(raw: &[u8]) -> Option<String> {
    let head_len = raw.len().min(512);
    let head = String::from_utf8_lossy(&raw[..head_len]);
    let decl_start = head.find("<?xml")?;
    let rest = &head[decl_start..];
    let decl_end = rest.find("?>")?;
    let decl = &rest[..decl_end];
    let lower = decl.to_ascii_lowercase();
    let enc_idx = lower.find("encoding")?;
    let after_enc = decl[enc_idx + "encoding".len()..].trim_start();
    let after_eq = after_enc.strip_prefix('=')?.trim_start();
    let quote = after_eq.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let val_rest = &after_eq[1..];
    let end_quote = val_rest.find(quote)?;
    let label = val_rest[..end_quote].trim();
    if label.is_empty() {
        None
    } else {
        Some(label.to_string())
    }
}

fn decode_xmltv_bytes(bytes: &[u8]) -> Result<String, String> {
    let owned_raw: Vec<u8>;
    let raw: &[u8] = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(bytes);
        let mut buf = Vec::new();
        decoder
            .read_to_end(&mut buf)
            .map_err(|e| format!("Kitömörítési hiba: {}", e))?;
        owned_raw = buf;
        &owned_raw
    } else {
        bytes
    };

    if let Some((enc, bom_len)) = encoding_rs::Encoding::for_bom(raw) {
        if enc == encoding_rs::UTF_8 {
            if let Ok(s) = std::str::from_utf8(&raw[bom_len..]) {
                return Ok(s.to_string());
            }
        } else {
            let (decoded, _, _) = enc.decode(&raw[bom_len..]);
            return Ok(decoded.into_owned());
        }
    }

    if let Ok(s) = std::str::from_utf8(raw) {
        return Ok(s.to_string());
    }

    if let Some(label) = extract_xml_encoding_label(raw) {
        if let Some(enc) = encoding_rs::Encoding::for_label(label.as_bytes()) {
            let (decoded, _, _) = enc.decode(raw);
            return Ok(decoded.into_owned());
        }
        return Err(format!("Nem támogatott XML karakterkódolás: {}", label));
    }

    Err("UTF8 konverziós hiba: a fájl nem érvényes UTF-8 és nincs felismerhető XML encoding fejléc".to_string())
}

pub(crate) async fn update_source_epg_inner(
    state: &AppState,
    source_id: &str,
    url: &str,
) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("Az EPG URL címe üres!".to_string());
    }

    // Rewrite github.com raw view URLs to raw.githubusercontent.com to avoid 504 errors.
    // github.com/owner/repo/raw/refs/heads/branch/path
    //   -> raw.githubusercontent.com/owner/repo/refs/heads/branch/path
    // github.com/owner/repo/raw/branch/path  (legacy form, no refs/heads)
    //   -> raw.githubusercontent.com/owner/repo/branch/path
    let url = {
        let trimmed = url.trim();
        if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
            let parts: Vec<&str> = rest.splitn(4, '/').collect();
            // parts: [owner, repo, "raw", "refs/heads/branch/path" or "branch/path"]
            if parts.len() == 4 && parts[2] == "raw" {
                format!("https://raw.githubusercontent.com/{}/{}/{}", parts[0], parts[1], parts[3])
            } else {
                trimmed.to_string()
            }
        } else {
            trimmed.to_string()
        }
    };

    // Read cached HTTP validators only if we actually have imported channels for this source
    let (cached_etag, cached_last_modified) = {
        let conn = state.db.lock().await;
        let has_channels: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM channels WHERE source_id = ?1)",
                params![source_id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if has_channels {
            conn.query_row(
                "SELECT etag, last_modified FROM sources WHERE id = ?1",
                params![source_id],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .unwrap_or((None, None))
        } else {
            (None, None)
        }
    };

    let mut req = state.http_epg.get(&url);
    if let Some(ref etag) = cached_etag {
        if !etag.trim().is_empty() {
            req = req.header(reqwest::header::IF_NONE_MATCH, etag.trim());
        }
    }
    if let Some(ref lm) = cached_last_modified {
        if !lm.trim().is_empty() {
            req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm.trim());
        }
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("Hálózati hiba: {}", e))?;

    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let conn = state.db.lock().await;
        conn.execute(
            "UPDATE sources SET last_downloaded = datetime('now', 'localtime') WHERE id = ?1",
            params![source_id],
        )
        .ok();
        return Ok(());
    }

    if !response.status().is_success() {
        return Err(format!("A szerver hibakódot adott vissza: {}", response.status()));
    }

    let new_etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let new_last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let bytes = response.bytes().await.map_err(|e| format!("Adatolvasási hiba: {}", e))?;

    // Offload gzip decompression, XML parsing, deduplication and stop-time repair off the async runtime
    // and without holding the SQLite mutex.
    let (parsed_channels, programmes) = tokio::task::spawn_blocking(move || {
        let xml_string = decode_xmltv_bytes(&bytes)?;
        parse_xmltv_str(&xml_string)
    })
    .await
    .map_err(|e| format!("Háttérszál hiba: {}", e))??;

    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    write_parsed_xmltv_into_tx(&tx, source_id, parsed_channels, programmes)?;

    tx.execute(
        "UPDATE sources SET last_downloaded = datetime('now', 'localtime'), etag = ?2, last_modified = ?3 WHERE id = ?1",
        params![source_id, new_etag, new_last_modified],
    ).ok();

    tx.commit().map_err(|e| format!("Mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn update_source_epg(state: tauri::State<'_, AppState>, source_id: String, url: String) -> Result<(), String> {
    update_source_epg_inner(state.inner(), &source_id, &url).await
}

pub(crate) const DUMMY_STOP_SECS: i64 = 1800;
pub(crate) const MAX_REPAIR_SPAN_SECS: i64 = 12 * 3600;

pub(crate) trait RepairableProgramme {
    fn channel_id(&self) -> &str;
    fn start_str(&self) -> &str;
    fn start_ts(&self) -> Option<i64>;
    fn stop_ts(&self) -> Option<i64>;
    fn set_stop(&mut self, stop: String, stop_ts: i64);
}

/// Repairs missing/zero-duration `stop` times and 30-minute scraper placeholder `stop` times
/// within a single source by aligning them to the next programme's `start` on the same channel.
pub(crate) fn repair_source_programmes<T: RepairableProgramme>(programmes: &mut [T]) {
    if programmes.is_empty() {
        return;
    }

    programmes.sort_by(|a, b| {
        a.channel_id()
            .cmp(b.channel_id())
            .then_with(|| a.start_ts().is_none().cmp(&b.start_ts().is_none()))
            .then_with(|| a.start_ts().cmp(&b.start_ts()))
    });

    type ChannelPlan = (usize, usize, usize, usize, Vec<Option<usize>>);
    let mut channel_plans: Vec<ChannelPlan> = Vec::new();
    let mut src_30m: usize = 0;
    let mut src_other: usize = 0;

    let mut start_idx = 0;
    while start_idx < programmes.len() {
        let ch_id = programmes[start_idx].channel_id().to_string();
        let mut end_idx = start_idx + 1;
        while end_idx < programmes.len() && programmes[end_idx].channel_id() == ch_id {
            end_idx += 1;
        }

        let ch_progs = &programmes[start_idx..end_idx];
        let n = ch_progs.len();
        let mut next_idx: Vec<Option<usize>> = vec![None; n];
        let mut next_distinct: Option<usize> = None;
        for i in (0..n).rev() {
            let Some(s) = ch_progs[i].start_ts() else {
                continue;
            };
            if let Some(nd) = next_distinct {
                if ch_progs[nd].start_ts() > Some(s) {
                    next_idx[i] = Some(nd);
                } else {
                    next_idx[i] = next_idx[nd];
                }
            }
            if next_distinct.is_none_or(|nd| ch_progs[nd].start_ts() != Some(s)) {
                next_distinct = Some(i);
            }
        }

        let mut ch_30m: usize = 0;
        let mut ch_other: usize = 0;
        for i in 0..n {
            let (Some(s), Some(e), Some(j)) = (
                ch_progs[i].start_ts(),
                ch_progs[i].stop_ts(),
                next_idx[i],
            ) else {
                continue;
            };
            let Some(ns) = ch_progs[j].start_ts() else {
                continue;
            };
            let dur = e - s;
            if dur <= 0 {
                continue;
            }
            if (ns > e && ns - s <= MAX_REPAIR_SPAN_SECS) || ns < e {
                if dur == DUMMY_STOP_SECS {
                    ch_30m += 1;
                } else {
                    ch_other += 1;
                }
            }
        }

        src_30m += ch_30m;
        src_other += ch_other;
        channel_plans.push((start_idx, end_idx, ch_30m, ch_other, next_idx));
        start_idx = end_idx;
    }

    for (start_idx, end_idx, ch_30m, ch_other, next_idx) in channel_plans {
        let source_is_30m_dummy = src_30m >= 10 && src_30m > src_other.saturating_mul(3);
        let fix_30m_dummy = (ch_30m > ch_other) || (source_is_30m_dummy && ch_30m > 0);
        let ch_progs = &mut programmes[start_idx..end_idx];
        for i in 0..ch_progs.len() {
            let Some(s) = ch_progs[i].start_ts() else {
                continue;
            };
            let Some(j) = next_idx[i] else {
                continue;
            };
            let Some(ns) = ch_progs[j].start_ts() else {
                continue;
            };
            let e_opt = ch_progs[i].stop_ts();
            let should_repair = if e_opt.is_none_or(|e| e <= s) {
                ns - s <= MAX_REPAIR_SPAN_SECS
            } else if fix_30m_dummy && e_opt == Some(s + DUMMY_STOP_SECS) {
                (ns > s + DUMMY_STOP_SECS && ns - s <= MAX_REPAIR_SPAN_SECS)
                    || ns < s + DUMMY_STOP_SECS
            } else {
                false
            };
            if should_repair {
                let next_start_str = ch_progs[j].start_str().to_string();
                ch_progs[i].set_stop(next_start_str, ns);
            }
        }
    }
}

struct ParsedProgramme {
    channel_id: String,
    start: String,
    stop: String,
    title: String,
    desc: String,
    title_norm: String,
    start_ts: Option<i64>,
    stop_ts: Option<i64>,
}

impl RepairableProgramme for ParsedProgramme {
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
    }
}

#[derive(Hash, Eq, PartialEq)]
enum StartDedupKey {
    Epoch(i64),
    Raw(String),
}

#[derive(Hash, Eq, PartialEq)]
struct ProgrammeDedupKey {
    channel_id: String,
    start: StartDedupKey,
    title_key: String,
}

type ParsedXmltvResult = (Vec<(String, String, String)>, Vec<ParsedProgramme>);

fn parse_xmltv_str(xml_string: &str) -> Result<ParsedXmltvResult, String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(xml_string);
    reader.trim_text(true);

    let mut buf = Vec::new();
    let mut current_tag = String::new();
    let mut tag_buf = String::new();
    let mut in_channel = false;
    let mut in_programme = false;
    let mut curr_ch_id = String::new();
    let mut curr_ch_name = String::new();
    let mut curr_ch_icon = String::new();

    let mut curr_prog_ch = String::new();
    let mut curr_prog_start = String::new();
    let mut curr_prog_stop = String::new();
    let mut curr_prog_title = String::new();
    let mut curr_prog_desc = String::new();

    let mut parsed_channels: Vec<(String, String, String)> = Vec::new();
    let mut programmes: Vec<ParsedProgramme> = Vec::new();
    let mut seen_programmes: std::collections::HashMap<ProgrammeDedupKey, usize> =
        std::collections::HashMap::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                current_tag = name.clone();
                tag_buf.clear();

                if name == "channel" {
                    in_channel = true;
                    curr_ch_id.clear(); curr_ch_name.clear(); curr_ch_icon.clear();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"id" {
                            curr_ch_id = String::from_utf8_lossy(&attr.value).to_string();
                        }
                    }
                } else if name == "programme" {
                    in_programme = true;
                    curr_prog_ch.clear(); curr_prog_start.clear(); curr_prog_stop.clear();
                    curr_prog_title.clear(); curr_prog_desc.clear();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"channel" {
                            curr_prog_ch = String::from_utf8_lossy(&attr.value).to_string();
                        } else if attr.key.as_ref() == b"start" {
                            curr_prog_start = String::from_utf8_lossy(&attr.value).to_string();
                        } else if attr.key.as_ref() == b"stop" {
                            curr_prog_stop = String::from_utf8_lossy(&attr.value).to_string();
                        }
                    }
                } else if name == "icon" && in_channel {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            curr_ch_icon = attr
                                .unescape_value()
                                .map(|v| v.into_owned())
                                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
                        }
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                if in_channel && e.name().as_ref() == b"icon" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            curr_ch_icon = attr
                                .unescape_value()
                                .map(|v| v.into_owned())
                                .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string());
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                if matches!(current_tag.as_str(), "display-name" | "title" | "desc") {
                    let text = e.unescape().unwrap_or_default();
                    tag_buf.push_str(&text);
                }
            }
            Ok(Event::CData(e)) => {
                if matches!(current_tag.as_str(), "display-name" | "title" | "desc") {
                    let text = String::from_utf8_lossy(e.as_ref());
                    tag_buf.push_str(&text);
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "display-name" && in_channel {
                    let val = tag_buf.trim();
                    if curr_ch_name.is_empty() && !val.is_empty() {
                        curr_ch_name = val.to_string();
                    }
                } else if name == "title" && in_programme {
                    let val = tag_buf.trim();
                    if curr_prog_title.is_empty() && !val.is_empty() {
                        curr_prog_title = val.to_string();
                    }
                } else if name == "desc" && in_programme {
                    let val = tag_buf.trim();
                    if curr_prog_desc.is_empty() && !val.is_empty() {
                        curr_prog_desc = val.to_string();
                    }
                } else if name == "channel" {
                    in_channel = false;
                    parsed_channels.push((
                        curr_ch_id.clone(),
                        curr_ch_name.clone(),
                        curr_ch_icon.clone(),
                    ));
                } else if name == "programme" {
                    in_programme = false;
                    if curr_prog_ch.trim().is_empty()
                        || curr_prog_start.trim().is_empty()
                        || curr_prog_title.trim().is_empty()
                    {
                        current_tag.clear();
                        tag_buf.clear();
                        buf.clear();
                        continue;
                    }
                    let title_norm = normalise_for_match(&clean_title(&curr_prog_title));
                    let start_ts = xmltv_to_epoch(&curr_prog_start);
                    let stop_ts = xmltv_to_epoch(&curr_prog_stop);

                    let dedup_key = ProgrammeDedupKey {
                        channel_id: curr_prog_ch.clone(),
                        start: match start_ts {
                            Some(ts) => StartDedupKey::Epoch(ts),
                            None => StartDedupKey::Raw(curr_prog_start.clone()),
                        },
                        title_key: if !title_norm.is_empty() {
                            title_norm.clone()
                        } else {
                            curr_prog_title.trim().to_lowercase()
                        },
                    };

                    let mut cand = ParsedProgramme {
                        channel_id: curr_prog_ch.clone(),
                        start: curr_prog_start.clone(),
                        stop: curr_prog_stop.clone(),
                        title: curr_prog_title.clone(),
                        desc: curr_prog_desc.clone(),
                        title_norm,
                        start_ts,
                        stop_ts,
                    };

                    if let Some(&idx) = seen_programmes.get(&dedup_key) {
                        let existing = &mut programmes[idx];
                        let existing_valid = existing
                            .stop_ts
                            .zip(existing.start_ts)
                            .is_some_and(|(e, s)| e > s);
                        let cand_valid = cand
                            .stop_ts
                            .zip(cand.start_ts)
                            .is_some_and(|(e, s)| e > s);

                        if !existing_valid && cand_valid {
                            if cand.desc.is_empty() && !existing.desc.is_empty() {
                                cand.desc = std::mem::take(&mut existing.desc);
                            }
                            *existing = cand;
                        } else if existing_valid && !cand_valid {
                            if existing.desc.is_empty() && !cand.desc.is_empty() {
                                existing.desc = cand.desc;
                            }
                        } else if existing_valid == cand_valid {
                            let existing_is_dummy_30m = existing_valid
                                && existing
                                    .stop_ts
                                    .zip(existing.start_ts)
                                    .is_some_and(|(e, s)| e - s == DUMMY_STOP_SECS);
                            let cand_is_dummy_30m = cand_valid
                                && cand
                                    .stop_ts
                                    .zip(cand.start_ts)
                                    .is_some_and(|(e, s)| e - s == DUMMY_STOP_SECS);

                            if cand.desc.len() > existing.desc.len() {
                                if existing_valid && !existing_is_dummy_30m && cand_is_dummy_30m {
                                    existing.desc = cand.desc;
                                } else {
                                    *existing = cand;
                                }
                            } else if cand.desc.len() == existing.desc.len()
                                && existing_is_dummy_30m
                                && !cand_is_dummy_30m
                            {
                                *existing = cand;
                            }
                        }
                    } else {
                        let idx = programmes.len();
                        seen_programmes.insert(dedup_key, idx);
                        programmes.push(cand);
                    }
                }
                current_tag.clear();
                tag_buf.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML feldolgozási hiba: {}", e)),
            _ => (),
        }
        buf.clear();
    }

    repair_source_programmes(&mut programmes);
    Ok((parsed_channels, programmes))
}

fn write_parsed_xmltv_into_tx(
    tx: &rusqlite::Transaction<'_>,
    source_id: &str,
    parsed_channels: Vec<(String, String, String)>,
    programmes: Vec<ParsedProgramme>,
) -> Result<(), String> {
    tx.execute("DELETE FROM channels WHERE source_id = ?1", params![source_id])
        .map_err(|e| format!("Csatornák törlési hiba: {}", e))?;
    tx.execute("DELETE FROM programmes WHERE source_id = ?1", params![source_id])
        .map_err(|e| format!("Műsorok törlési hiba: {}", e))?;

    let mut ch_stmt = tx
        .prepare("INSERT OR REPLACE INTO channels (id, source_id, name, icon) VALUES (?1, ?2, ?3, ?4)")
        .map_err(|e| format!("DB előkészítési hiba: {}", e))?;

    for (ch_id, ch_name, ch_icon) in parsed_channels {
        ch_stmt.execute(params![ch_id, source_id, ch_name, ch_icon]).ok();
    }

    let mut prog_stmt = tx
        .prepare(
            "INSERT INTO programmes (channel_id, source_id, start, stop, title, desc, title_norm, start_ts, stop_ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(|e| format!("DB előkészítési hiba: {}", e))?;

    for p in programmes {
        prog_stmt
            .execute(params![
                p.channel_id,
                source_id,
                p.start,
                p.stop,
                p.title,
                p.desc,
                p.title_norm,
                p.start_ts,
                p.stop_ts,
            ])
            .ok();
    }

    Ok(())
}

#[cfg(test)]
fn import_xmltv_into_tx(
    tx: &rusqlite::Transaction<'_>,
    source_id: &str,
    xml_string: &str,
) -> Result<(), String> {
    let (parsed_channels, programmes) = parse_xmltv_str(xml_string)?;
    write_parsed_xmltv_into_tx(tx, source_id, parsed_channels, programmes)
}

#[cfg(target_os = "windows")]
fn apply_windows_dark_titlebar(hwnd: isize) {
    use std::ffi::c_void;
    type DwmSetWindowAttributeFn = unsafe extern "system" fn(
        hwnd: isize,
        dw_attribute: u32,
        pv_attribute: *const c_void,
        cb_attribute: u32,
    ) -> i32;

    extern "system" {
        fn LoadLibraryA(lp_lib_file_name: *const u8) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *mut c_void;
    }

    unsafe {
        let dwmapi = LoadLibraryA(c"dwmapi.dll".as_ptr().cast::<u8>());
        if !dwmapi.is_null() {
            let func = GetProcAddress(dwmapi, c"DwmSetWindowAttribute".as_ptr().cast::<u8>());
            if let Some(set_attr) =
                std::mem::transmute::<*mut c_void, Option<DwmSetWindowAttributeFn>>(func)
            {
                let dark_mode: i32 = 1;
                set_attr(hwnd, 20, &dark_mode as *const _ as *const c_void, 4);

                let caption_color: u32 = 0x001F1F1F;
                set_attr(hwnd, 35, &caption_color as *const _ as *const c_void, 4);

                let text_color: u32 = 0x00FFFFFF;
                set_attr(hwnd, 36, &text_color as *const _ as *const c_void, 4);
            }
        }
    }
}

fn validate_http_url(url: &str) -> Result<&str, String> {
    let u = url.trim();
    if !u.starts_with("http://") && !u.starts_with("https://") {
        return Err("Csak http(s) URL nyitható meg.".to_string());
    }
    Ok(u)
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    let u = validate_http_url(&url)?;
    open::that(u).map_err(|e| e.to_string())
}

fn spawn_programmes_backfill(app_handle: tauri::AppHandle) {
    use tauri::Manager;
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app_handle.state::<AppState>();
            let updated_count = {
                let mut conn = state.db.lock().await;
                let rows: Vec<(i64, String, String, String)> = {
                    let mut stmt = match conn.prepare(
                        "SELECT id, title, start, stop FROM programmes WHERE title_norm IS NULL LIMIT 5000",
                    ) {
                        Ok(s) => s,
                        Err(_) => break,
                    };
                    let mapped = match stmt.query_map([], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1).unwrap_or_default(),
                            r.get::<_, String>(2).unwrap_or_default(),
                            r.get::<_, String>(3).unwrap_or_default(),
                        ))
                    }) {
                        Ok(m) => m,
                        Err(_) => break,
                    };
                    mapped.flatten().collect()
                };

                if rows.is_empty() {
                    0
                } else {
                    let count = rows.len();
                    if let Ok(tx) = conn.transaction() {
                        if let Ok(mut upd) = tx.prepare(
                            "UPDATE programmes SET title_norm = ?1, start_ts = ?2, stop_ts = ?3 WHERE id = ?4",
                        ) {
                            for (id, title, start, stop) in rows {
                                let norm = normalise_for_match(&clean_title(&title));
                                let s_ts = xmltv_to_epoch(&start);
                                let e_ts = xmltv_to_epoch(&stop);
                                let _ = upd.execute(params![norm, s_ts, e_ts, id]);
                            }
                        }
                        let _ = tx.commit();
                    }
                    count
                }
            };

            if updated_count == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/159.0.0.0 Safari/537.36 EPGViewer/1.0";

fn base_http_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(10))
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .pool_max_idle_per_host(10)
}

fn main() {
    let http_client = base_http_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("HTTP kliens létrehozási hiba");

    // Dedicated client for EPG downloads: shares base settings (user agent, redirects, pool limits)
    // with gzip transport compression enabled for plain .xml feeds (while decode_xmltv_bytes still
    // transparently handles .xml.gz files via magic bytes) and a 60s timeout for large XML/GZ downloads.
    let http_epg_client = base_http_builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("EPG HTTP kliens létrehozási hiba");

    let db_conn = open_and_init_db().expect("Adatbázis inicializálási hiba");
    let initial_window_title = resolve_initial_window_title(&db_conn);
    // Drop Trakt HTTP cache rows that expired long ago (they otherwise pile up, e.g. one calendar body per day).
    let _ = trakt::client::prune_http_cache(&db_conn);
    let trakt_state = trakt::TraktState::new(&db_conn);

    // Dedicated read-only connection to images.db for serving cached logos/posters via epgimg://
    // without ever blocking behind epg.db write transactions.
    let images_ro_conn = rusqlite::Connection::open_with_flags(
        get_images_db_path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
    .map(|c| {
        let _ = c.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA cache_size = -16000;");
        std::sync::Arc::new(std::sync::Mutex::new(c))
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol("epgimg", move |_ctx, request, responder| {
            let sha = request.uri().path().trim_start_matches('/').to_string();
            let conn_opt = images_ro_conn.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let not_found = || {
                    tauri::http::Response::builder()
                        .status(404)
                        .header("Content-Type", "text/plain")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(Vec::new())
                        .unwrap()
                };

                if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    responder.respond(not_found());
                    return;
                }

                let found = conn_opt.and_then(|arc| {
                    let conn = arc.lock().ok()?;
                    read_image_bytes_by_sha256(&conn, &sha)
                });

                match found {
                    Some((mime, bytes)) => {
                        let resp = tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", mime)
                            .header("Cache-Control", "public, max-age=31536000, immutable")
                            .header("Access-Control-Allow-Origin", "*")
                            .body(bytes)
                            .unwrap();
                        responder.respond(resp);
                    }
                    None => responder.respond(not_found()),
                }
            });
        })
        .manage(AppState {
            db: Mutex::new(db_conn),
            http: http_client,
            http_epg: http_epg_client,
            trakt: trakt_state,
            tmdb: TmdbRuntime::new(),
            stats: RequestStats::default(),
            image_inflight: std::sync::Mutex::new(HashMap::new()),
            updater: updater::UpdaterState::default(),
        })
        .setup(move |app| {
            {
                use tauri::Manager;
                for window in app.webview_windows().values() {
                    let _ = window.set_title(&initial_window_title);
                    #[cfg(target_os = "windows")]
                    if let Ok(hwnd) = window.hwnd() {
                        apply_windows_dark_titlebar(hwnd.0 as isize);
                    }
                }
            }
            spawn_programmes_backfill(app.handle().clone());
            let startup_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Manager;
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                let st = startup_handle.state::<AppState>();
                let should_sync = {
                    let conn = st.db.lock().await;
                    trakt::is_trakt_enabled(&conn)
                        && conn
                            .query_row(
                                "SELECT EXISTS(SELECT 1 FROM trakt_auth WHERE id = 1 AND access_blob != '')",
                                [],
                                |r| r.get::<_, bool>(0),
                            )
                            .unwrap_or(false)
                };
                if should_sync {
                    let _ = trakt::sync::run_sync(st.inner(), Some(&startup_handle), false).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_epg_data,
            get_source_channels,
            get_sources,
            save_source,
            delete_source,
            save_source_order,
            clear_entire_database,
            update_source_epg,
            get_favorites,
            get_favorite_programmes_airings,
            toggle_favorite,
            save_channel_order,
            get_sort_presets,
            save_sort_preset,
            rename_sort_preset,
            delete_sort_preset,
            toggle_channel_favorite,
            toggle_channel_hidden,
            set_channel_logo,
            rename_channel,
            cache_logo,
            get_cached_logo,
            find_duplicate_logos,
            deduplicate_logos,
            get_image_cache_stats,
            clear_logo_cache,
            clear_poster_cache,
            clear_all_image_cache,
            get_database_stats,
            maintain_database,
            get_channel_logo_stats,
            get_missing_logo_channels,
            preload_logo_sources,
            find_channel_logo_github,
            find_channel_logo_wikimedia,
            search_channel_logos,
            clean_and_slugify_channel,
            export_settings,
            import_settings,
            export_channel_data,
            import_channel_data,
            get_settings,
            set_setting,
            get_language_content,
            save_language_content,
            download_languages_from_github,
            get_tmdb_details,
            get_tmdb_details_for_lang,
            get_tmdb_api_key,
            set_tmdb_api_key,
            clear_tmdb_cache,
            test_tmdb_api_key,
            search_tmdb,
            set_tmdb_match,
            clear_tmdb_match,
            get_request_stats,
            reset_request_stats,
            open_url,
            updater::get_app_about_info,
            updater::check_for_updates,
            updater::install_available_update,
            trakt::commands::trakt_get_auth_status,
            trakt::commands::trakt_start_login,
            trakt::commands::trakt_cancel_login,
            trakt::commands::trakt_logout,
            trakt::commands::trakt_set_enabled,
            trakt::commands::trakt_set_custom_keys,
            trakt::commands::trakt_sync,
            trakt::commands::trakt_cancel_sync,
            trakt::commands::trakt_get_sync_status,
            trakt::commands::trakt_get_dashboard_data,
            trakt::commands::trakt_mark_watched,
            trakt::commands::trakt_watchlist_add,
            trakt::commands::trakt_watchlist_remove,
            trakt::commands::trakt_get_watchlist,
            trakt::commands::trakt_get_show_seasons_progress,
            trakt::commands::trakt_get_calendar_my,
            trakt::commands::trakt_get_calendar_global,
            trakt::commands::trakt_get_global_calendar_filters,
            trakt::commands::trakt_set_global_calendar_filters,
            trakt::commands::trakt_match_titles,
            trakt::commands::trakt_find_epg_airings,
            trakt::commands::trakt_get_watchlist_epg_airings,
            trakt::commands::trakt_get_stats,
            trakt::commands::trakt_clear_cache,
            trakt::commands::trakt_resolve_for_title,
            trakt::commands::trakt_resolve_tmdb_poster,
            trakt::commands::get_tmdb_details_by_id,
            trakt::commands::trakt_on_window_focus
        ])
        .run(tauri::generate_context!())
        .expect("Hiba a Tauri indításakor");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_title_preserves_subtitles() {
        assert_eq!(
            clean_title("Jurassic World: Das gefallene Königreich"),
            "Jurassic World: Das gefallene Königreich"
        );
        assert_eq!(
            clean_title("G.I. Joe - Megtorlás"),
            "G.I. Joe - Megtorlás"
        );
        assert_eq!(
            clean_title("G.I. Joe - Geheimauftrag Cobra"),
            "G.I. Joe - Geheimauftrag Cobra"
        );
        assert_eq!(
            clean_title("CSI: Miami helyszínelők"),
            "CSI: Miami helyszínelők"
        );
        assert_eq!(
            clean_title("Mission: Impossible - Utóhatás"),
            "Mission: Impossible - Utóhatás"
        );
        assert_eq!(
            clean_title("Next Game: Calgary Wranglers @ Abbotsford Canucks on 2026-10-24 at 08:00PM EDT"),
            "Next Game: Calgary Wranglers @ Abbotsford Canucks on 2026-10-24 at 08:00PM EDT"
        );
    }

    #[test]
    fn test_clean_title_preserves_movie_sequels_and_roman_numerals() {
        assert_eq!(
            clean_title("Jurassic Park 2. - Az elveszett világ"),
            "Jurassic Park 2. - Az elveszett világ"
        );
        assert_eq!(
            clean_title("Jurassic Park 3."),
            "Jurassic Park 3"
        );
        assert_eq!(
            clean_title("Jurassic Park III."),
            "Jurassic Park III"
        );
        assert_eq!(
            clean_title("Rocky IV"),
            "Rocky IV"
        );
        assert_eq!(
            clean_title("A szállító 2."),
            "A szállító 2"
        );
    }

    #[test]
    fn test_clean_title_removes_series_season_and_episode() {
        assert_eq!(
            clean_title("A mi kis falunk 6. évad 1. rész"),
            "A mi kis falunk"
        );
        assert_eq!(
            clean_title("Gyurma srácok - Második évad"),
            "Gyurma srácok"
        );
        assert_eq!(
            clean_title("Baby Einstein: Új klasszikusok 1. évad"),
            "Baby Einstein: Új klasszikusok"
        );
        assert_eq!(
            clean_title("Linda - 3. évad 7."),
            "Linda"
        );
        assert_eq!(
            clean_title("Merész hétvége I./1."),
            "Merész hétvége"
        );
        assert_eq!(
            clean_title("Vissza Sherwoodba 8.rész"),
            "Vissza Sherwoodba"
        );
        assert_eq!(
            clean_title("Star Wars: A klónok háborúja 2. évad 5. rész - A hősök"),
            "Star Wars: A klónok háborúja"
        );
        assert_eq!(
            clean_title("Szökőár 1. rész"),
            "Szökőár"
        );
        assert_eq!(
            clean_title("The Big Bang Theory S03E14"),
            "The Big Bang Theory"
        );
        assert_eq!(
            clean_title("Trónok harca 8. évad"),
            "Trónok harca"
        );
    }

    #[test]
    fn test_clean_title_removes_tech_tags_and_years() {
        assert_eq!(
            clean_title("Matrix (1999) (16) (HD)"),
            "Matrix"
        );
        assert_eq!(
            clean_title("Híradó (élő)"),
            "Híradó"
        );
        assert_eq!(
            clean_title("Dr. Csont (Bones)"),
            "Dr. Csont"
        );
    }

    #[test]
    fn test_clean_channel_name_and_slugify() {
        assert_eq!(clean_channel_name("RTL Klub HD"), "RTL Klub");
        assert_eq!(slugify_channel_name(&clean_channel_name("RTL Klub HD")), "rtl-klub");

        assert_eq!(clean_channel_name("Cool TV (HD)"), "Cool TV");
        assert_eq!(slugify_channel_name(&clean_channel_name("Cool TV (HD)")), "cool-tv");

        assert_eq!(clean_channel_name("HU: Mozi+ [HUN]"), "Mozi+");
        assert_eq!(slugify_channel_name(&clean_channel_name("HU: Mozi+ [HUN]")), "mozi-plus");

        assert_eq!(clean_channel_name("Arena4 HD"), "Arena4");
        assert_eq!(slugify_channel_name(&clean_channel_name("Arena4 HD")), "arena4");

        assert_eq!(clean_channel_name("Duna World / M4 Sport+ (HD) (Telekom)"), "Duna World / M4 Sport+");
        assert_eq!(slugify_channel_name(&clean_channel_name("Duna World / M4 Sport+ (HD) (Telekom)")), "duna-world-m4-sport-plus");

        assert_eq!(clean_channel_name("TV2 Séf"), "TV2 Séf");
        assert_eq!(slugify_channel_name(&clean_channel_name("TV2 Séf")), "tv2-sef");

        assert_eq!(clean_channel_name("UniMas HD (Pacific)"), "UniMas");
        assert_eq!(clean_channel_name("The Discovery Channel HD (Pacific)"), "The Discovery Channel");
        assert_eq!(clean_channel_name("HBO East"), "HBO");
        assert_eq!(clean_channel_name("Showtime West (HD)"), "Showtime");
        assert_eq!(clean_channel_name("Starz (East)"), "Starz");
        assert_eq!(slugify_channel_name(&clean_channel_name("Animal Planet HD (Pacific)")), "animal-planet");

        assert_eq!(clean_channel_name("BBC One East.uk"), "BBC One");
        assert_eq!(clean_channel_name("Starz Encore Westerns.us"), "Starz Encore Westerns");
        assert_eq!(clean_channel_name("Cool TV.hu"), "Cool TV");
        assert_eq!(clean_channel_name("TotalDanceTV.hu@SD"), "TotalDanceTV");
        assert_eq!(clean_channel_name("Euronews.de HD"), "Euronews");
    }

    #[test]
    fn test_map_country_code() {
        assert_eq!(map_country_code("HU"), (Some("hungary"), "hu".to_string()));
        assert_eq!(map_country_code("de"), (Some("germany"), "de".to_string()));
        assert_eq!(map_country_code("at"), (Some("austria"), "at".to_string()));
        assert_eq!(map_country_code("US"), (Some("united-states"), "us".to_string()));
        assert_eq!(map_country_code("UK"), (Some("united-kingdom"), "uk".to_string()));
        assert_eq!(map_country_code("CA"), (Some("canada"), "ca".to_string()));
        assert_eq!(map_country_code("CH"), (Some("switzerland"), "ch".to_string()));
        assert_eq!(map_country_code("SE"), (Some("nordic"), "se".to_string()));
        assert_eq!(map_country_code("NO"), (Some("nordic"), "no".to_string()));
        assert_eq!(map_country_code("DK"), (Some("nordic"), "dk".to_string()));
        assert_eq!(map_country_code("FI"), (Some("nordic"), "fi".to_string()));
        // Unknown country code must NOT fall back to Hungary
        assert_eq!(map_country_code("ZZ"), (None, "zz".to_string()));
    }

    #[test]
    fn test_match_iptv_org_logos_with_fallback() {
        let logos = vec![
            IptvOrgLogoItem {
                channel: "AnimalPlanet.hu".to_string(),
                url: "https://example.com/animal-hu.svg".to_string(),
                format: Some("SVG".to_string()),
            },
            IptvOrgLogoItem {
                channel: "CartoonNetwork.int".to_string(),
                url: "https://example.com/cn-int.png".to_string(),
                format: Some("PNG".to_string()),
            },
            IptvOrgLogoItem {
                channel: "Eurosport1.fr".to_string(),
                url: "https://example.com/eurosport-fr.png".to_string(),
                format: Some("PNG".to_string()),
            },
        ];

        // 1. Exact country match
        let res_hu = match_iptv_org_logos(&logos, "animal-planet", "HU");
        assert!(res_hu.is_some());
        assert_eq!(res_hu.unwrap().url, "https://example.com/animal-hu.svg");

        // 2. International .int fallback
        let res_int = match_iptv_org_logos(&logos, "cartoon-network", "int");
        assert!(res_int.is_some());
        assert_eq!(res_int.unwrap().url, "https://example.com/cn-int.png");

        // 3. Global / without language fallback
        let res_global = match_iptv_org_logos_any(&logos, "eurosport1");
        assert!(res_global.is_some());
        assert_eq!(res_global.unwrap().url, "https://example.com/eurosport-fr.png");
    }

    #[test]
    fn test_match_tv_logo_from_tree() {
        let paths = vec![
            "countries/hungary/cool-tv-hu.png".to_string(),
            "countries/international/animal-planet-int.png".to_string(),
            "countries/france/eurosport-1-fr.png".to_string(),
            "countries/nordic/sweden/svt-1-se.png".to_string(),
        ];

        // 1. Country match
        let res1 = match_tv_logo_from_tree(&paths, Some("hungary"), "hu", "cool-tv");
        assert!(res1.is_some());
        assert_eq!(res1.unwrap().source_name, "tv-logo");

        // 2. International fallback
        let res2 = match_tv_logo_from_tree(&paths, Some("hungary"), "hu", "animal-planet");
        assert!(res2.is_some());
        assert_eq!(res2.unwrap().source_name, "tv-logo (int)");

        // 3. Global in any country fallback
        let res3 = match_tv_logo_from_tree(&paths, Some("hungary"), "hu", "eurosport-1");
        assert!(res3.is_some());
        assert_eq!(res3.unwrap().source_name, "tv-logo (global)");

        // 4. Fuzzy matching test
        let res_fuzzy = match_tv_logo_from_tree(&paths, Some("hungary"), "hu", "cool-tv-hd");
        assert!(res_fuzzy.is_some());
        assert_eq!(res_fuzzy.unwrap().source_name, "tv-logo");

        // 5. Nested Nordic directory match
        let res_nordic = match_tv_logo_from_tree(&paths, Some("nordic"), "se", "svt-1");
        assert!(res_nordic.is_some());
        assert_eq!(res_nordic.unwrap().source_name, "tv-logo");

        // 6. Unknown country (None) skips target country and still finds international/global
        let res_unknown = match_tv_logo_from_tree(&paths, None, "zz", "animal-planet");
        assert!(res_unknown.is_some());
        assert_eq!(res_unknown.unwrap().source_name, "tv-logo (int)");
    }

    #[test]
    fn test_fuzzy_match_iptv_org_logos() {
        let logos = vec![
            IptvOrgLogoItem {
                channel: "DiscoveryScience.hu".to_string(),
                url: "https://example.com/disc-sci.png".to_string(),
                format: Some("PNG".to_string()),
            },
        ];

        // Slugs with slight variations (e.g. extra character, typo)
        let res = match_iptv_org_logos(&logos, "discoverysciencee", "HU");
        assert!(res.is_some());
        assert_eq!(res.unwrap().url, "https://example.com/disc-sci.png");
    }

    #[test]
    fn test_images_db_direct_join() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sha256 TEXT NOT NULL UNIQUE,
                mime_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                data TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE images_db.image_urls (
                url TEXT PRIMARY KEY,
                image_id INTEGER NOT NULL,
                image_type TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        ).unwrap();

        conn.execute(
            "INSERT INTO images_db.images (sha256, mime_type, byte_size, data, created_at)
             VALUES ('hash123', 'image/png', 100, 'data:image/png;base64,sample', '2026-11-14')",
            [],
        ).unwrap();
        let img_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES ('https://example.com/logo.png', ?1, 'logo', '2026-11-14')",
            params![img_id],
        ).unwrap();

        conn.execute(
            "CREATE TABLE channels (id TEXT, source_id TEXT, name TEXT, icon TEXT);",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO channels VALUES ('ch1', 's1', 'Channel 1', 'https://example.com/logo.png');",
            [],
        ).unwrap();

        let data: String = conn.query_row(
            "SELECT img.data FROM channels c
             LEFT JOIN images_db.image_urls iu ON iu.url = c.icon
             LEFT JOIN images_db.images img ON img.id = iu.image_id
             WHERE c.id = 'ch1'",
            [],
            |r| r.get(0),
        ).unwrap();

        assert_eq!(data, "data:image/png;base64,sample");
    }

    #[test]
    fn test_wal_cleanup_to_delete_mode() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("epg_test_wal_cleanup.db");
        let wal_path = temp_dir.join("epg_test_wal_cleanup.db-wal");
        let shm_path = temp_dir.join("epg_test_wal_cleanup.db-shm");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (x TEXT); INSERT INTO t VALUES ('hello');").unwrap();

        // Checkpoint and switch back to DELETE
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode = DELETE;").unwrap();
        drop(conn);

        // WAL and SHM files should be gone or 0-byte, and DB has the data
        let conn2 = Connection::open(&db_path).unwrap();
        let val: String = conn2.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(val, "hello");
        drop(conn2);

        assert!(!wal_path.exists() || std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(1) == 0);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&wal_path);
        let _ = std::fs::remove_file(&shm_path);
    }

    #[test]
    fn test_xmltv_to_epoch() {
        assert_eq!(xmltv_to_epoch("19700101000000 +0000"), Some(0));
        assert_eq!(xmltv_to_epoch("20261118200000 +0100"), xmltv_to_epoch("20261118190000 +0000"));
        assert_eq!(xmltv_to_epoch("20261118200000 -0500"), xmltv_to_epoch("20261119010000 +0000"));
        assert_eq!(xmltv_to_epoch("202611182000"), xmltv_to_epoch("20261118200000 +0000"));
        assert_eq!(xmltv_to_epoch("20240229120000Z"), xmltv_to_epoch("20240229120000 +0000"));
        assert_eq!(xmltv_to_epoch("invalid"), None);
    }

    #[test]
    fn test_normalise_for_match_diacritics() {
        assert_eq!(normalise_for_match("A mi kis falunk"), "a mi kis falunk");
        assert_eq!(normalise_for_match("Őrült szerelem: 2. rész!"), "orult szerelem 2 resz");
        assert_eq!(normalise_for_match("  Dr. Csont (Bones)  "), "dr csont bones");
    }

    #[test]
    fn test_validate_http_url() {
        assert!(validate_http_url("https://trakt.tv/activate").is_ok());
        assert!(validate_http_url("http://localhost:8080").is_ok());
        assert!(validate_http_url("file:///C:/Windows/System32/calc.exe").is_err());
        assert!(validate_http_url("javascript:alert(1)").is_err());
        assert!(validate_http_url("cmd.exe").is_err());
    }

    #[test]
    fn test_get_cached_logo_query_without_view() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sha256 TEXT NOT NULL UNIQUE,
                mime_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                data TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE images_db.image_urls (
                url TEXT PRIMARY KEY,
                image_id INTEGER NOT NULL,
                image_type TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        ).unwrap();
        conn.execute(
            "INSERT INTO images_db.images (sha256, mime_type, byte_size, data, created_at)
             VALUES ('abc', 'image/jpeg', 10, 'data:image/jpeg;base64,xyz', '2026-11-18')",
            [],
        ).unwrap();
        let img_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES ('https://image.tmdb.org/t/p/original/test.jpg', ?1, 'poster', '2026-11-18')",
            params![img_id],
        ).unwrap();

        let res: Option<String> = conn.query_row(
            "SELECT img.data FROM images_db.image_urls iu
             JOIN images_db.images img ON img.id = iu.image_id
             WHERE iu.url = ?1",
            params!["https://image.tmdb.org/t/p/original/test.jpg"],
            |row| row.get(0),
        ).ok();
        assert_eq!(res.as_deref(), Some("data:image/jpeg;base64,xyz"));
    }

    #[test]
    fn test_import_xmltv_deduplicates_programmes_and_keeps_best() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tv>
  <channel id="398"><display-name>SkyShowtime 1</display-name></channel>
  <channel id="399"><display-name>SkyShowtime 2</display-name></channel>
  <programme start="20261124200000 +0000" stop="20261124205000 +0000" channel="398">
    <title lang="hu">Tulsa királya</title>
    <desc lang="hu">IV / 5. rész</desc>
  </programme>
  <programme start="20261125232000 +0000" stop="20261126001000 +0000" channel="398">
    <title lang="hu">Tulsa királya</title>
    <desc lang="hu">IV / 5. rész</desc>
  </programme>
  <programme start="20261125232000 +0000" stop="20261126001000 +0000" channel="398">
    <title lang="hu">Tulsa királya</title>
    <desc lang="hu">IV / 5. rész</desc>
  </programme>
  <programme start="20261128234500 +0000" stop="20261128234500 +0000" channel="398">
    <title lang="hu">Tulsa királya</title>
    <desc lang="hu">Rövid</desc>
  </programme>
  <programme start="20261129004500 +0100" stop="20261129013500 +0100" channel="398">
    <title lang="hu">Tulsa királya</title>
    <desc lang="hu"></desc>
  </programme>
  <programme start="20261125232000 +0000" stop="20261126001000 +0000" channel="399">
    <title lang="hu">Tulsa királya</title>
    <desc lang="hu">Másik csatorna</desc>
  </programme>
</tv>"#;

        let tx = conn.transaction().unwrap();
        import_xmltv_into_tx(&tx, "porthu", xml).unwrap();
        tx.commit().unwrap();

        let ch398_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM programmes WHERE source_id = 'porthu' AND channel_id = '398'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ch398_count, 3, "5 XML entries on channel 398 must deduplicate to 3 distinct airings");

        // Nov 29 entry had a zero-duration entry with desc "Rövid" followed by a valid-duration entry with empty desc:
        // valid duration must win while preserving the non-empty description.
        let (stop_ts, desc): (i64, String) = conn
            .query_row(
                "SELECT stop_ts, desc FROM programmes WHERE source_id = 'porthu' AND channel_id = '398' AND start_ts = ?1",
                params![xmltv_to_epoch("20261128234500 +0000").unwrap()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stop_ts, xmltv_to_epoch("20261129013500 +0100").unwrap());
        assert_eq!(desc, "Rövid");

        let ch399_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM programmes WHERE source_id = 'porthu' AND channel_id = '399'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ch399_count, 1);
    }

    #[test]
    fn test_import_xmltv_repairs_30m_dummy_stops_duplicates_and_zero_durations() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tv>
  <channel id="5-star-max"><display-name>5-Star MAX</display-name></channel>
  <channel id="bbc-news"><display-name>BBC News</display-name></channel>
  <channel id="amc"><display-name>AMC</display-name></channel>
  <channel id="auto"><display-name>Auto Motor</display-name></channel>
  <channel id="m4plus"><display-name>M4 Sport+</display-name></channel>
  <channel id="dazn1"><display-name>DAZN 1</display-name></channel>

  <!-- 30m dummy stop leaving a gap across midnight/day boundary -->
  <programme start="20261126030800 +0000" stop="20261126033800 +0000" channel="5-star-max">
    <title>The Hangover Part II</title>
  </programme>
  <programme start="20261126045000 +0000" stop="20261126064700 +0000" channel="5-star-max">
    <title>Time Bandits</title>
  </programme>

  <!-- 30m dummy stop overlapping next day's first show -->
  <programme start="20261126034500 +0000" stop="20261126041500 +0000" channel="bbc-news">
    <title>BBC Sport</title>
  </programme>
  <programme start="20261126040000 +0000" stop="20261126043000 +0000" channel="bbc-news">
    <title>BBC News</title>
  </programme>

  <!-- Midnight-spanning show truncated to 30m (23:00 -> 23:30 instead of 00:55 next day) -->
  <programme start="20261125220000 +0000" stop="20261125223000 +0000" channel="amc">
    <title>Az árnyékember</title>
  </programme>
  <programme start="20261125235500 +0000" stop="20261126015000 +0000" channel="amc">
    <title>Cliffhanger</title>
  </programme>

  <!-- Duplicate start where first entry has 30m dummy stop and second has real 15m stop -->
  <programme start="20261125044500 +0000" stop="20261125051500 +0000" channel="auto">
    <title>Vorfahrt</title>
    <desc>Same desc</desc>
  </programme>
  <programme start="20261125044500 +0000" stop="20261125050000 +0000" channel="auto">
    <title>Vorfahrt</title>
    <desc>Same desc</desc>
  </programme>
  <programme start="20261125050000 +0000" stop="20261125051500 +0000" channel="auto">
    <title>Die 7</title>
  </programme>

  <!-- Multi-day off-air gap (>12h) must NOT be stretched -->
  <programme start="20261129190000 +0000" stop="20261129193000 +0000" channel="m4plus">
    <title>Skating</title>
  </programme>
  <programme start="20261205130000 +0000" stop="20261205140000 +0000" channel="m4plus">
    <title>F2</title>
  </programme>

  <!-- Zero-duration programme with simultaneous start tie followed by next show -->
  <programme start="20261127192500 +0000" stop="20261127192500 +0000" channel="dazn1">
    <title>NCAA</title>
  </programme>
  <programme start="20261127192500 +0000" stop="20261127192500 +0000" channel="dazn1">
    <title>Ligue 1</title>
  </programme>
  <programme start="20261127203000 +0000" stop="20261127220000 +0000" channel="dazn1">
    <title>NHL</title>
  </programme>

  <!-- Empty title must be skipped -->
  <programme start="20261124073548 +0000" stop="20261124073548 +0000" channel="dazn1">
    <title></title>
  </programme>
</tv>"#;

        let tx = conn.transaction().unwrap();
        import_xmltv_into_tx(&tx, "test_src", xml).unwrap();
        tx.commit().unwrap();

        let get_stop = |ch: &str, title: &str| -> (String, i64) {
            conn.query_row(
                "SELECT stop, stop_ts FROM programmes WHERE source_id = 'test_src' AND channel_id = ?1 AND title = ?2",
                params![ch, title],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };

        let (s, ts) = get_stop("5-star-max", "The Hangover Part II");
        assert_eq!(s, "20261126045000 +0000");
        assert_eq!(ts, xmltv_to_epoch("20261126045000 +0000").unwrap());

        let (s, ts) = get_stop("bbc-news", "BBC Sport");
        assert_eq!(s, "20261126040000 +0000");
        assert_eq!(ts, xmltv_to_epoch("20261126040000 +0000").unwrap());

        let (s, ts) = get_stop("amc", "Az árnyékember");
        assert_eq!(s, "20261125235500 +0000");
        assert_eq!(ts, xmltv_to_epoch("20261125235500 +0000").unwrap());

        let (s, ts) = get_stop("auto", "Vorfahrt");
        assert_eq!(s, "20261125050000 +0000");
        assert_eq!(ts, xmltv_to_epoch("20261125050000 +0000").unwrap());

        let (s, _) = get_stop("m4plus", "Skating");
        assert_eq!(s, "20261129193000 +0000", "multi-day off-air gap must not be stretched");

        let (s, _) = get_stop("dazn1", "NCAA");
        assert_eq!(s, "20261127203000 +0000");
        let (s, _) = get_stop("dazn1", "Ligue 1");
        assert_eq!(s, "20261127203000 +0000");

        let empty_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM programmes WHERE source_id = 'test_src' AND TRIM(title) = ''",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(empty_count, 0);
    }

    #[test]
    fn test_import_xmltv_extracts_self_closing_channel_icons() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<tv>
  <channel id="ch1">
    <display-name>M1 HD</display-name>
    <icon src="https://example.com/m1.png?a=1&amp;b=2" />
  </channel>
  <channel id="ch2">
    <icon src="https://example.com/rtl.png"></icon>
    <display-name>RTL</display-name>
  </channel>
  <programme start="20261125100000 +0000" stop="20261125110000 +0000" channel="ch1">
    <title>Híradó</title>
    <icon src="https://example.com/prog-icon.png" />
    <desc>Esti hírek</desc>
  </programme>
</tv>"#;

        let tx = conn.transaction().unwrap();
        import_xmltv_into_tx(&tx, "src1", xml).unwrap();
        tx.commit().unwrap();

        let (name1, icon1): (String, String) = conn
            .query_row(
                "SELECT name, icon FROM channels WHERE source_id = 'src1' AND id = 'ch1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name1, "M1 HD");
        assert_eq!(icon1, "https://example.com/m1.png?a=1&b=2");

        let (name2, icon2): (String, String) = conn
            .query_row(
                "SELECT name, icon FROM channels WHERE source_id = 'src1' AND id = 'ch2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(name2, "RTL");
        assert_eq!(icon2, "https://example.com/rtl.png");

        let (prog_title, prog_desc): (String, String) = conn
            .query_row(
                "SELECT title, desc FROM programmes WHERE source_id = 'src1' AND channel_id = 'ch1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(prog_title, "Híradó");
        assert_eq!(prog_desc, "Esti hírek");
    }

    #[test]
    fn test_decode_xmltv_bytes_non_utf8_plain_and_gzip() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let sample = "<?xml version=\"1.0\" encoding=\"ISO-8859-2\"?><tv><channel id=\"1\"><display-name>Árvíztűrő tükörfúrógép</display-name></channel></tv>";
        let (iso_bytes, _, had_errors) = encoding_rs::ISO_8859_2.encode(sample);
        assert!(!had_errors);

        // Plain ISO-8859-2
        let decoded_plain = decode_xmltv_bytes(&iso_bytes).unwrap();
        assert!(decoded_plain.contains("Árvíztűrő tükörfúrógép"));

        // Gzipped ISO-8859-2
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&iso_bytes).unwrap();
        let gz_bytes = gz.finish().unwrap();
        let decoded_gz = decode_xmltv_bytes(&gz_bytes).unwrap();
        assert!(decoded_gz.contains("Árvíztűrő tükörfúrógép"));

        // Windows-1250
        let win_sample = "<?xml version='1.0' encoding='windows-1250'?><tv><title>Műsorfüzet ŐŰ</title></tv>";
        let (win_bytes, _, had_errors2) = encoding_rs::WINDOWS_1250.encode(win_sample);
        assert!(!had_errors2);
        let decoded_win = decode_xmltv_bytes(&win_bytes).unwrap();
        assert!(decoded_win.contains("Műsorfüzet ŐŰ"));

        // Invalid UTF-8 without encoding header must fail cleanly
        assert!(decode_xmltv_bytes(&[0x3c, 0x74, 0x76, 0x3e, 0xff, 0xfe]).is_err());
    }

    #[test]
    fn test_source_sort_position_serde_and_all_favorites_ordering() {
        // Legacy backup JSON without sortPosition must deserialize cleanly
        let legacy_json = r#"{"id":"s1","name":"Src 1","url":"https://a.test/epg.xml","lastDownloaded":null,"countryCode":"HU","timeOffset":0}"#;
        let parsed: Source = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(parsed.sort_position, None);

        // New JSON with sortPosition preserves it
        let new_json = r#"{"id":"s2","name":"Src 2","url":"https://b.test/epg.xml","lastDownloaded":null,"countryCode":"HU","timeOffset":0,"sortPosition":2}"#;
        let parsed2: Source = serde_json::from_str(new_json).unwrap();
        assert_eq!(parsed2.sort_position, Some(2));

        // Verify COALESCE(s.sort_position, s.rowid) ordering
        let conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();
        conn.execute(
            "INSERT INTO sources (id, name, url, sort_position) VALUES ('s_first_inserted', 'First', 'https://a', 10)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO sources (id, name, url, sort_position) VALUES ('s_second_inserted', 'Second', 'https://b', 1)",
            [],
        ).unwrap();
        conn.execute("INSERT INTO channels (id, source_id, name, icon) VALUES ('c1', 's_first_inserted', 'Ch1', '')", []).unwrap();
        conn.execute("INSERT INTO channels (id, source_id, name, icon) VALUES ('c2', 's_second_inserted', 'Ch2', '')", []).unwrap();
        conn.execute("INSERT INTO channel_user_settings (source_id, channel_id, is_favorite) VALUES ('s_first_inserted', 'c1', 1)", []).unwrap();
        conn.execute("INSERT INTO channel_user_settings (source_id, channel_id, is_favorite) VALUES ('s_second_inserted', 'c2', 1)", []).unwrap();

        let mut stmt = conn.prepare(
            "SELECT c.source_id
             FROM channels c
             JOIN sources s ON s.id = c.source_id
             JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
             WHERE cus.is_favorite = 1 AND COALESCE(cus.is_hidden, 0) = 0
             ORDER BY COALESCE(s.sort_position, s.rowid) ASC"
        ).unwrap();
        let ordered: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().flatten().collect();
        assert_eq!(ordered, vec!["s_second_inserted", "s_first_inserted"]);
    }

    #[test]
    fn test_deduplicate_logos_protects_channel_icons_and_cleans_orphans() {
        let conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();
        conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                sha256 TEXT NOT NULL UNIQUE,
                mime_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                data TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE images_db.image_urls (
                url TEXT PRIMARY KEY,
                image_id INTEGER NOT NULL,
                image_type TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (image_id) REFERENCES images(id) ON DELETE CASCADE
            );"
        ).unwrap();

        // Image 1: shared by active channels.icon (url_ch1) and custom_icon (url_custom1)
        conn.execute("INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at) VALUES (1, 'h1', 'image/png', 5000, 'data:image/png;base64,A', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES ('https://xmltv.test/ch1-very-long-url.png', 1, 'logo', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES ('https://c.test/1.png', 1, 'logo', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO channels (id, source_id, name, icon) VALUES ('c1', 's1', 'Ch1', 'https://xmltv.test/ch1-very-long-url.png')", []).unwrap();
        conn.execute("INSERT INTO channel_user_settings (source_id, channel_id, custom_icon) VALUES ('s1', 'c2', 'https://c.test/1.png')", []).unwrap();

        // Image 2: two active channels.icon URLs -> BOTH must be protected!
        conn.execute("INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at) VALUES (2, 'h2', 'image/png', 4000, 'data:image/png;base64,B', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES ('https://a.test/2.png', 2, 'logo', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES ('https://b.test/2.png', 2, 'logo', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO channels (id, source_id, name, icon) VALUES ('c3', 's1', 'Ch3', 'https://a.test/2.png')", []).unwrap();
        conn.execute("INSERT INTO channels (id, source_id, name, icon) VALUES ('c4', 's2', 'Ch4', 'https://b.test/2.png')", []).unwrap();

        // Image 3: completely orphaned logo (no channel or custom_icon references it)
        conn.execute("INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at) VALUES (3, 'h3', 'image/png', 3000, 'data:image/png;base64,C', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES ('https://orphan.test/3.png', 3, 'logo', datetime('now'))", []).unwrap();

        // Image 4: poster -> must never be touched
        conn.execute("INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at) VALUES (4, 'h4', 'image/jpeg', 9000, 'data:image/jpeg;base64,P', datetime('now'))", []).unwrap();
        conn.execute("INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES ('https://image.tmdb.org/p4.jpg', 4, 'poster', datetime('now'))", []).unwrap();

        let report = find_duplicate_logos_inner(&conn).unwrap();
        let expected_saved = "https://c.test/1.png".len() as i64
            + "https://orphan.test/3.png".len() as i64
            + 3000;
        assert_eq!(report.redundant_count, 2);
        assert_eq!(report.potential_saved_bytes, expected_saved);

        let res = deduplicate_logos_inner(&conn).unwrap();
        assert_eq!(res.removed_urls, 2);
        assert_eq!(res.removed_images, 1);
        assert_eq!(res.freed_bytes, expected_saved);

        // Active channels.icon URL for image 1 MUST still exist, and custom_icon was repointed to it
        let ch1_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM images_db.image_urls WHERE url = 'https://xmltv.test/ch1-very-long-url.png'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(ch1_exists, 1);

        let updated_custom: String = conn.query_row(
            "SELECT custom_icon FROM channel_user_settings WHERE source_id = 's1' AND channel_id = 'c2'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(updated_custom, "https://xmltv.test/ch1-very-long-url.png");

        // Both active channels.icon URLs for image 2 MUST still exist
        let img2_urls: i64 = conn.query_row(
            "SELECT COUNT(*) FROM images_db.image_urls WHERE image_id = 2",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(img2_urls, 2);

        // Orphaned image 3 is gone, poster image 4 is intact
        let img3_exists: i64 = conn.query_row("SELECT COUNT(*) FROM images_db.images WHERE id = 3", [], |r| r.get(0)).unwrap();
        let img4_exists: i64 = conn.query_row("SELECT COUNT(*) FROM images_db.images WHERE id = 4", [], |r| r.get(0)).unwrap();
        assert_eq!(img3_exists, 0);
        assert_eq!(img4_exists, 1);
    }

    #[cfg(windows)]
    #[test]
    fn test_tmdb_key_dpapi_migration_and_export_exclusion() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();
        let store = trakt::secret::default_secret_store();

        let secret = "tmdb_secret_bearer_token_987654";
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('tmdb_api_key', ?1)",
            params![secret],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('language', 'hungarian')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('trakt_secure_foo', 'trakt_blob')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('google_secure_bar', 'google_blob')",
            [],
        )
        .unwrap();

        migrate_plaintext_tmdb_api_key(&mut conn, store.as_ref()).unwrap();

        // Plaintext row must be gone; encrypted blob must differ from plaintext and decrypt cleanly
        let legacy_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM settings WHERE key = 'tmdb_api_key'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(legacy_count, 0);

        let raw_blob: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'tmdb_secure_key'", [], |r| r.get(0))
            .unwrap();
        assert_ne!(raw_blob, secret);
        assert_eq!(
            load_tmdb_api_key_with_store(&conn, store.as_ref()).as_deref(),
            Some(secret)
        );

        // Export JSON must never contain the TMDB key or any protected/internal setting keys
        let backup = export_settings_from_conn(&conn).unwrap();
        let json = serde_json::to_string(&backup).unwrap();
        assert!(!json.contains(secret), "Export JSON leaked plaintext TMDB key");
        assert!(!json.contains(&raw_blob), "Export JSON leaked encrypted TMDB blob");
        assert!(!backup.settings.contains_key("tmdb_api_key"));
        assert!(!backup.settings.contains_key("tmdb_secure_key"));
        assert!(!backup.settings.contains_key("trakt_secure_foo"));
        assert!(!backup.settings.contains_key("google_secure_bar"));
        assert_eq!(backup.settings.get("language").map(String::as_str), Some("hungarian"));

        // get_settings must also hide protected/internal keys
        let visible = get_settings_from_conn(&conn).unwrap();
        assert!(!visible.contains_key("tmdb_api_key"));
        assert!(!visible.contains_key("tmdb_secure_key"));
        assert!(!visible.contains_key("trakt_secure_foo"));
        assert!(!visible.contains_key("google_secure_bar"));
        assert_eq!(visible.get("language").map(String::as_str), Some("hungarian"));

        // set_setting must block all sensitive keys
        assert!(set_setting_in_conn(&conn, "tmdb_api_key", "x").is_err());
        assert!(set_setting_in_conn(&conn, "tmdb_secure_key", "x").is_err());
        assert!(set_setting_in_conn(&conn, "trakt_secure_tok", "x").is_err());
        assert!(set_setting_in_conn(&conn, "google_secure_tok", "x").is_err());
        assert!(set_setting_in_conn(&conn, "theme", "dark").is_ok());
    }

    #[test]
    fn test_import_settings_filters_sensitive_keys_and_rolls_back_on_error() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        conn.execute("INSERT INTO favorites (title) VALUES ('Existing Fav')", []).unwrap();
        conn.execute("INSERT INTO settings (key, value) VALUES ('theme', 'dark')", []).unwrap();

        // 1. Valid import ignores all sensitive/internal keys
        let mut incoming_settings = HashMap::new();
        incoming_settings.insert("language".to_string(), "english".to_string());
        incoming_settings.insert("tmdb_api_key".to_string(), "leaked".to_string());
        incoming_settings.insert("tmdb_secure_key".to_string(), "blob1".to_string());
        incoming_settings.insert("trakt_secure_tok".to_string(), "blob2".to_string());
        incoming_settings.insert("google_secure_tok".to_string(), "blob3".to_string());
        incoming_settings.insert("lng_hash_english".to_string(), "fakehash".to_string());

        let valid_backup = BackupData {
            version: 1,
            exported_at: "2026-11-25".to_string(),
            sources: vec![],
            favorites: vec!["New Fav".to_string()],
            channel_user_settings: vec![],
            channel_orders: vec![],
            channel_sort_presets: vec![],
            settings: incoming_settings,
        };
        import_settings_into_conn(&mut conn, valid_backup).unwrap();

        let lang: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'language'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(lang, "english");
        for blocked in ["tmdb_api_key", "tmdb_secure_key", "trakt_secure_tok", "google_secure_tok"] {
            let cnt: i64 = conn
                .query_row("SELECT COUNT(*) FROM settings WHERE key = ?1", params![blocked], |r| r.get(0))
                .unwrap();
            assert_eq!(cnt, 0, "Blocked key {} must not be imported", blocked);
        }

        // 2. Failing SQL inside import_settings (duplicate preset id) must return Err and roll back favorites & settings
        let bad_backup = BackupData {
            version: 1,
            exported_at: "2026-11-25".to_string(),
            sources: vec![],
            favorites: vec!["Should Rollback".to_string()],
            channel_user_settings: vec![],
            channel_orders: vec![],
            channel_sort_presets: vec![
                SortPreset {
                    id: 1,
                    name: "P1".to_string(),
                    source_id: "s1".to_string(),
                    channel_order: vec!["c1".to_string()],
                    created_at: Some("2026-11-25".to_string()),
                },
                SortPreset {
                    id: 1, // Duplicate PRIMARY KEY triggers SQLite constraint error
                    name: "P2".to_string(),
                    source_id: "s1".to_string(),
                    channel_order: vec!["c2".to_string()],
                    created_at: Some("2026-11-25".to_string()),
                },
            ],
            settings: HashMap::from([("language".to_string(), "should_rollback".to_string())]),
        };
        assert!(import_settings_into_conn(&mut conn, bad_backup).is_err());

        // Favorites and settings must still reflect the previous committed state
        let fav: String = conn
            .query_row("SELECT title FROM favorites LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fav, "New Fav");
        let lang_after: String = conn
            .query_row("SELECT value FROM settings WHERE key = 'language'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(lang_after, "english");
    }

    #[test]
    fn test_maintain_database_prunes_and_vacuums_preserving_manual_rows() {
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join("epg_test_maintain_epg.db");
        let img_path = temp_dir.join("epg_test_maintain_images.db");
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&img_path);

        let conn = Connection::open(&db_path).unwrap();
        migrations::run_migrations(&conn, None).unwrap();
        conn.execute(
            &format!("ATTACH DATABASE '{}' AS images_db", img_path.to_string_lossy().replace('\'', "''")),
            [],
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (id INTEGER PRIMARY KEY AUTOINCREMENT, sha256 TEXT NOT NULL UNIQUE, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL, data TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE images_db.image_urls (url TEXT PRIMARY KEY, image_id INTEGER NOT NULL, image_type TEXT NOT NULL, created_at TEXT NOT NULL);",
        )
        .unwrap();

        // Populate large expired trakt_http_cache row so VACUUM visibly shrinks the file
        let big_body = "x".repeat(250_000);
        conn.execute(
            "INSERT INTO trakt_http_cache (key, etag, body, fetched_at, ttl_seconds, page_count)
             VALUES ('/calendars/my/shows/2020-01-01/32', NULL, ?1, '2020-01-01T00:00:00Z', 60, 0)",
            params![big_body],
        )
        .unwrap();
        // Expired automatic negative vs permanent manual negative
        conn.execute(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at)
             VALUES ('auto_expired', 'hu-HU', 'none', 0, '2020-01-01 00:00:00'),
                    ('manual_keep', 'hu-HU', 'none', 1, '2020-01-01 00:00:00')",
            [],
        )
        .unwrap();
        // One orphan image + one referenced image
        conn.execute(
            "INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at)
             VALUES (1, 'keep_h', 'image/png', 10, 'data:image/png;base64,AAAA', datetime('now')),
                    (2, 'orphan_h', 'image/png', 100000, ?1, datetime('now'))",
            params![format!("data:image/png;base64,{}", "A".repeat(100_000))],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES ('https://example.com/a.png', 1, 'logo', datetime('now'))",
            [],
        )
        .unwrap();

        let stats_before = get_database_stats_inner(&conn, Some(&db_path), Some(&img_path));
        assert_eq!(stats_before.expired_http_cache_rows, 1);
        assert_eq!(stats_before.expired_tmdb_negative_rows, 1);
        assert_eq!(stats_before.orphaned_image_rows, 1);

        let res = maintain_database_inner(&conn, Some(&db_path), Some(&img_path)).unwrap();
        assert_eq!(res.pruned_http_cache, 1);
        assert_eq!(res.pruned_tmdb_negatives, 1);
        assert_eq!(res.pruned_orphan_images, 1);
        assert_eq!(res.converted_blob_images, 1);
        assert!(res.after_bytes < res.before_bytes, "VACUUM must shrink disk size");
        assert!(res.freed_bytes > 100_000);

        let manual_still_exists: i64 = conn
            .query_row("SELECT COUNT(*) FROM tmdb_cache WHERE query_title = 'manual_keep'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(manual_still_exists, 1, "Manual negative marker must survive maintenance");

        drop(conn);
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(&img_path);
    }

    #[test]
    fn test_images_db_blob_storage_and_legacy_text_conversion() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (id INTEGER PRIMARY KEY AUTOINCREMENT, sha256 TEXT NOT NULL UNIQUE, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL, data BLOB NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE images_db.image_urls (url TEXT PRIMARY KEY, image_id INTEGER NOT NULL, image_type TEXT NOT NULL, created_at TEXT NOT NULL);",
        )
        .unwrap();

        let raw_png: &[u8] = b"\x89PNG\r\n\x1a\nhello";
        use base64::Engine;
        let expected_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(raw_png)
        );

        // 1. Native BLOB row
        conn.execute(
            "INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at)
             VALUES (1, 'blob_h', 'image/png', ?1, ?2, datetime('now'))",
            params![raw_png.len() as i64, raw_png],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES ('https://example.com/blob.png', 1, 'logo', datetime('now'))",
            [],
        )
        .unwrap();

        // 2. Legacy base64 TEXT row
        conn.execute(
            "INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at)
             VALUES (2, 'text_h', 'image/png', 999, ?1, datetime('now'))",
            params![expected_uri],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES ('https://example.com/text.png', 2, 'poster', datetime('now'))",
            [],
        )
        .unwrap();

        // Both read identically as data URIs
        assert_eq!(
            read_cached_image_data_uri(&conn, "https://example.com/blob.png").as_deref(),
            Some(expected_uri.as_str())
        );
        assert_eq!(
            read_cached_image_data_uri(&conn, "https://example.com/text.png").as_deref(),
            Some(expected_uri.as_str())
        );

        // Converting legacy TEXT rows turns id=2 into raw BLOB and fixes byte_size
        let converted = convert_legacy_text_images_to_blobs(&conn);
        assert_eq!(converted, 1);
        let (kind, size): (String, i64) = conn
            .query_row(
                "SELECT typeof(data), byte_size FROM images_db.images WHERE id = 2",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "blob");
        assert_eq!(size, raw_png.len() as i64);
        assert_eq!(
            read_cached_image_data_uri(&conn, "https://example.com/text.png").as_deref(),
            Some(expected_uri.as_str())
        );
        // Second conversion pass is a no-op
        assert_eq!(convert_legacy_text_images_to_blobs(&conn), 0);
    }

    #[test]
    fn test_channel_export_row_deserializes_bool_and_int() {
        let json_bool = r#"{
            "sourceId": "s1", "sourceName": "Src", "sourceUrl": "https://a.test",
            "channelId": "c1", "channelName": "Ch1",
            "isFavorite": true, "isHidden": false,
            "position": 0, "customIcon": null, "customName": null
        }"#;
        let row_bool: ChannelExportRow = serde_json::from_str(json_bool).unwrap();
        assert_eq!(row_bool.is_favorite, 1);
        assert_eq!(row_bool.is_hidden, 0);

        let json_int = r#"{
            "sourceId": "s1", "sourceName": "Src", "sourceUrl": "https://a.test",
            "channelId": "c1", "channelName": "Ch1",
            "isFavorite": 1, "isHidden": 1,
            "position": 2, "customIcon": null, "customName": null
        }"#;
        let row_int: ChannelExportRow = serde_json::from_str(json_int).unwrap();
        assert_eq!(row_int.is_favorite, 1);
        assert_eq!(row_int.is_hidden, 1);
    }

    #[test]
    fn test_import_xmltv_handles_cdata_and_rolls_back_on_malformed_xml() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        let valid_xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <tv>
          <channel id="ch1">
            <display-name><![CDATA[RTL Klub & HD]]></display-name>
            <display-name>101</display-name>
          </channel>
          <programme start="20261125200000 +0100" stop="20261125210000 +0100" channel="ch1">
            <title><![CDATA[Film & Barátai]]></title>
            <title lang="en">Movie &amp; Friends</title>
            <desc><![CDATA[Izgalmas <akció> leírás]]></desc>
          </programme>
        </tv>"#;

        {
            let tx = conn.transaction().unwrap();
            import_xmltv_into_tx(&tx, "src1", valid_xml).unwrap();
            tx.commit().unwrap();
        }

        let ch_name: String = conn
            .query_row("SELECT name FROM channels WHERE id = 'ch1' AND source_id = 'src1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ch_name, "RTL Klub & HD");

        let (title, desc): (String, String) = conn
            .query_row("SELECT title, desc FROM programmes WHERE source_id = 'src1'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(title, "Film & Barátai");
        assert_eq!(desc, "Izgalmas <akció> leírás");

        // Malformed XML must return Err and preserve existing channels/programmes
        let broken_xml = r#"<?xml version="1.0"?><tv><channel id="ch2"><display-name>Broken</channel></tv>"#;
        {
            let tx = conn.transaction().unwrap();
            assert!(import_xmltv_into_tx(&tx, "src1", broken_xml).is_err());
        }
        let preserved_cnt: i64 = conn
            .query_row("SELECT COUNT(*) FROM programmes WHERE source_id = 'src1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(preserved_cnt, 1, "Failed XML import must not wipe existing EPG data");
    }

    #[test]
    fn test_detect_and_validate_image_mime() {
        assert_eq!(
            detect_and_validate_image_mime(Some("application/octet-stream"), b"\x89PNG\r\n\x1a\n1234").unwrap(),
            "image/png"
        );
        assert_eq!(
            detect_and_validate_image_mime(Some("text/plain"), b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").unwrap(),
            "image/svg+xml"
        );
        assert!(detect_and_validate_image_mime(Some("text/html; charset=utf-8"), b"<!DOCTYPE html><html>404</html>").is_err());
        assert!(detect_and_validate_image_mime(Some("image/png"), b"<html>not an image</html>").is_err());
        assert!(detect_and_validate_image_mime(Some("application/json"), b"{\"error\":\"not found\"}").is_err());
        assert!(detect_and_validate_image_mime(Some("image/png"), &[]).is_err());
    }

    #[test]
    fn test_get_image_cache_stats_does_not_double_count_deduped_images() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (id INTEGER PRIMARY KEY AUTOINCREMENT, sha256 TEXT NOT NULL UNIQUE, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL, data BLOB NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE images_db.image_urls (url TEXT PRIMARY KEY, image_id INTEGER NOT NULL, image_type TEXT NOT NULL, created_at TEXT NOT NULL);
             INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at) VALUES (1, 'h1', 'image/png', 1000, X'89504E47', datetime('now'));
             INSERT INTO images_db.image_urls (url, image_id, image_type, created_at) VALUES
                ('https://a.test/1.png', 1, 'logo', datetime('now')),
                ('https://b.test/1.png', 1, 'logo', datetime('now'));",
        )
        .unwrap();

        let stats = get_image_cache_stats_from_conn(&conn);
        assert_eq!(stats.logo_count, 2);
        assert_eq!(stats.logo_bytes, 1000, "Shared deduplicated image must only be counted once");
        assert_eq!(stats.total_bytes, 1000);
    }

    #[test]
    fn test_delete_source_cleans_all_favorites_orders_without_wildcard_collision() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();
        conn.execute_batch(
            "INSERT INTO sources (id, name, url) VALUES ('src_1', 'S1', 'https://a'), ('srcX1', 'SX', 'https://b');
             INSERT INTO channel_orders (source_id, channel_id, position) VALUES
                ('src_1', 'c1', 0),
                ('__ALL_FAVORITES__', 'src_1::c1', 0),
                ('__ALL_FAVORITES__', 'srcX1::c1', 1);",
        )
        .unwrap();

        delete_source_in_conn(&mut conn, "src_1").unwrap();

        let remaining: Vec<String> = conn
            .prepare("SELECT channel_id FROM channel_orders WHERE source_id = '__ALL_FAVORITES__'")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(remaining, vec!["srcX1::c1".to_string()]);
    }

    #[test]
    fn test_epgimg_protocol_url_and_raw_bytes_lookup() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE images (id INTEGER PRIMARY KEY AUTOINCREMENT, sha256 TEXT NOT NULL UNIQUE, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL, data BLOB NOT NULL, created_at TEXT NOT NULL);",
        )
        .unwrap();

        let raw_png: &[u8] = b"\x89PNG\r\n\x1a\nraw";
        let sha_blob = "a".repeat(64);
        let sha_text = "b".repeat(64);
        use base64::Engine;
        let legacy_uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(raw_png)
        );
        conn.execute(
            "INSERT INTO images (sha256, mime_type, byte_size, data, created_at) VALUES (?1, 'image/png', ?2, ?3, datetime('now'))",
            params![sha_blob, raw_png.len() as i64, raw_png],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images (sha256, mime_type, byte_size, data, created_at) VALUES (?1, 'image/png', ?2, ?3, datetime('now'))",
            params![sha_text, legacy_uri.len() as i64, legacy_uri],
        )
        .unwrap();

        assert_eq!(read_image_bytes_by_sha256(&conn, &sha_blob), Some(("image/png".to_string(), raw_png.to_vec())));
        assert_eq!(read_image_bytes_by_sha256(&conn, &sha_text), Some(("image/png".to_string(), raw_png.to_vec())));
        assert_eq!(read_image_bytes_by_sha256(&conn, &"c".repeat(64)), None);
        assert!(image_protocol_url(&sha_blob).ends_with(&sha_blob));
    }

    #[test]
    fn test_deduplicate_logos_protects_trakt_profile_avatar() {
        let conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();
        conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
        conn.execute_batch(
            "CREATE TABLE images_db.images (id INTEGER PRIMARY KEY AUTOINCREMENT, sha256 TEXT NOT NULL UNIQUE, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL, data BLOB NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE images_db.image_urls (url TEXT PRIMARY KEY, image_id INTEGER NOT NULL, image_type TEXT NOT NULL, created_at TEXT NOT NULL);",
        )
        .unwrap();

        let avatar_url = "https://secure.gravatar.com/avatar/d9c5857db48d3484721af9a3c802b294?s=256&r=pg";
        let profile = trakt::auth::TraktProfile {
            username: "tester".to_string(),
            slug: Some("tester".to_string()),
            name: Some("Tester".to_string()),
            vip: false,
            vip_ep: false,
            joined_at: None,
            avatar_url: Some(avatar_url.to_string()),
            watchlist_limit: None,
        };
        trakt::auth::save_cached_profile(&conn, &profile).unwrap();

        // Simulate legacy avatar cached with image_type = 'logo'
        conn.execute(
            "INSERT INTO images_db.images (id, sha256, mime_type, byte_size, data, created_at)
             VALUES (1, 'b2ac208e1ab0920a69485ecfae1f85cbc2d207855c6cc3aae453e488d8c7cd98', 'image/png', 40500, X'89504E47', datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
             VALUES (?1, 1, 'logo', datetime('now'))",
            params![avatar_url],
        )
        .unwrap();

        let report = find_duplicate_logos_inner(&conn).unwrap();
        assert_eq!(report.duplicate_groups.len(), 0, "Trakt avatar must not be reported as duplicate/orphan");
        assert_eq!(report.redundant_count, 0);

        let res = deduplicate_logos_inner(&conn).unwrap();
        assert_eq!(res.removed_urls, 0);
        assert_eq!(res.removed_images, 0);

        let new_type: String = conn
            .query_row(
                "SELECT image_type FROM images_db.image_urls WHERE url = ?1",
                params![avatar_url],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(new_type, "avatar", "Legacy avatar row should be reclassified as 'avatar'");
    }

    #[tokio::test]
    async fn test_update_source_epg_etag_304_skips_redownload_and_reparse() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let hits_srv = hits.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let h = hits_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                    h.fetch_add(1, Ordering::SeqCst);
                    if req.contains("if-none-match: \"epg-v1\"") {
                        let resp = "HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = socket.write_all(resp.as_bytes()).await;
                    } else {
                        let xml = r#"<?xml version="1.0"?><tv><channel id="c1"><display-name>Ch 1</display-name></channel><programme start="20261125200000 +0100" stop="20261125210000 +0100" channel="c1"><title>Show</title></programme></tv>"#;
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nETag: \"epg-v1\"\r\nLast-Modified: Wed, 25 Nov 2026 12:00:00 GMT\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            xml.len(),
                            xml
                        );
                        let _ = socket.write_all(resp.as_bytes()).await;
                    }
                });
            }
        });

        let state = trakt::tests::make_test_state("http://127.0.0.1:1".into(), "http://127.0.0.1:1".into());
        let url = format!("http://{}/epg.xml", addr);
        {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT INTO sources (id, name, url) VALUES ('s1', 'Test', ?1)",
                params![url],
            )
            .unwrap();
        }

        // First download: 200 OK -> imports channel and stores ETag
        update_source_epg_inner(&state, "s1", &url).await.unwrap();
        {
            let conn = state.db.lock().await;
            let (etag, lm): (Option<String>, Option<String>) = conn
                .query_row("SELECT etag, last_modified FROM sources WHERE id = 's1'", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(etag.as_deref(), Some("\"epg-v1\""));
            assert_eq!(lm.as_deref(), Some("Wed, 25 Nov 2026 12:00:00 GMT"));
        }

        // Second download: sends If-None-Match -> receives 304 Not Modified and preserves data
        update_source_epg_inner(&state, "s1", &url).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        {
            let conn = state.db.lock().await;
            let ch_cnt: i64 = conn
                .query_row("SELECT COUNT(*) FROM channels WHERE source_id = 's1'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(ch_cnt, 1, "304 Not Modified must keep existing imported channels intact");
        }
    }

    #[tokio::test]
    async fn test_cache_logo_single_flight_and_negative_cache() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let ok_hits = std::sync::Arc::new(AtomicUsize::new(0));
        let err_hits = std::sync::Arc::new(AtomicUsize::new(0));
        let ok_srv = ok_hits.clone();
        let err_srv = err_hits.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let ok_h = ok_srv.clone();
                let err_h = err_srv.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 2048];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");
                    if path == "/ok.png" {
                        ok_h.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                        let body: &[u8] = b"\x89PNG\r\n\x1a\n123456";
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = socket.write_all(hdr.as_bytes()).await;
                        let _ = socket.write_all(body).await;
                    } else {
                        err_h.fetch_add(1, Ordering::SeqCst);
                        let resp = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = socket.write_all(resp.as_bytes()).await;
                    }
                });
            }
        });

        let state = trakt::tests::make_test_state("http://127.0.0.1:1".into(), "http://127.0.0.1:1".into());
        {
            let conn = state.db.lock().await;
            conn.execute("ATTACH DATABASE ':memory:' AS images_db", []).unwrap();
            conn.execute_batch(
                "CREATE TABLE images_db.images (id INTEGER PRIMARY KEY AUTOINCREMENT, sha256 TEXT NOT NULL UNIQUE, mime_type TEXT NOT NULL, byte_size INTEGER NOT NULL, data BLOB NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE images_db.image_urls (url TEXT PRIMARY KEY, image_id INTEGER NOT NULL, image_type TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE images_db.failed_logo_cache (url TEXT PRIMARY KEY, status_code INTEGER, error TEXT NOT NULL, failed_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);",
            )
            .unwrap();
        }

        // 3 concurrent calls for the same valid URL -> only 1 HTTP request via single_flight_shared
        let ok_url = format!("http://{}/ok.png", addr);
        let (r1, r2, r3) = tokio::join!(
            cache_logo_inner(&state, ok_url.clone()),
            cache_logo_inner(&state, ok_url.clone()),
            cache_logo_inner(&state, ok_url.clone()),
        );
        assert!(r1.is_ok() && r2.is_ok() && r3.is_ok());
        assert_eq!(ok_hits.load(Ordering::SeqCst), 1, "Concurrent identical logo downloads must coalesce");

        // 403 error is stored in negative cache -> second call skips HTTP completely
        let bad_url = format!("http://{}/missing.png", addr);
        assert!(cache_logo_inner(&state, bad_url.clone()).await.is_err());
        assert!(cache_logo_inner(&state, bad_url.clone()).await.is_err());
        assert_eq!(err_hits.load(Ordering::SeqCst), 1, "403 logo must be served from negative cache on retry");
    }

    #[test]
    fn test_update_setting_key_is_protected() {
        assert!(is_internal_or_protected_setting_key(updater::SETTING_LAST_CHECKED_AT));
        assert!(!is_internal_or_protected_setting_key("language"));
    }

    #[test]
    fn test_resolve_initial_window_title_follows_saved_language_and_falls_back() {
        let conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        // No saved language -> defaults to Hungarian
        assert_eq!(
            resolve_initial_window_title(&conn),
            extract_app_title(EMBEDDED_HUNGARIAN_LNG).unwrap()
        );

        // Saved English language -> English title
        set_setting_in_conn(&conn, "language", "english").unwrap();
        assert_eq!(
            resolve_initial_window_title(&conn),
            extract_app_title(EMBEDDED_ENGLISH_LNG).unwrap()
        );

        // Custom app_title in DB is respected
        conn.execute(
            "UPDATE app_languages SET content = ?1 WHERE lang = 'english'",
            params![r#"{"app_title": "Custom Guide Title"}"#],
        )
        .unwrap();
        assert_eq!(resolve_initial_window_title(&conn), "Custom Guide Title");

        // Corrupted JSON in DB falls back to embedded language title
        conn.execute(
            "UPDATE app_languages SET content = 'not-json' WHERE lang = 'english'",
            [],
        )
        .unwrap();
        assert_eq!(
            resolve_initial_window_title(&conn),
            extract_app_title(EMBEDDED_ENGLISH_LNG).unwrap()
        );
    }

    #[test]
    fn test_query_favorite_programmes_airings_filters_sources_hidden_and_past() {
        let conn = Connection::open_in_memory().unwrap();
        migrations::run_migrations(&conn, None).unwrap();

        conn.execute_batch(
            "INSERT INTO sources (id, name, url, country_code, time_offset, sort_position) VALUES
                ('s1', 'Forrás 1', 'https://a', 'HU', 0, 0),
                ('s2', 'Forrás 2', 'https://b', 'EN', 1, 1);
             INSERT INTO channels (id, source_id, name, icon) VALUES
                ('c1', 's1', 'M1', ''),
                ('c2', 's1', 'Rejtett', ''),
                ('c3', 's2', 'HBO', '');
             INSERT INTO channel_user_settings (source_id, channel_id, is_favorite, is_hidden, custom_name) VALUES
                ('s1', 'c1', 1, 0, 'M1 HD'),
                ('s1', 'c2', 0, 1, NULL),
                ('s2', 'c3', 0, 0, NULL);
             INSERT INTO favorites (title) VALUES ('Kedvenc Film'), ('Másik Műsor');
             INSERT INTO programmes (channel_id, source_id, start, stop, title, desc, title_norm, start_ts, stop_ts) VALUES
                ('c1', 's1', '20261125100000 +0000', '20261125110000 +0000', 'Kedvenc Film', 'Múltbeli', 'kedvenc film', 1000, 1100),
                ('c1', 's1', '20261126200000 +0000', '20261126210000 +0000', 'Kedvenc Film', 'Jövőbeli 1', 'kedvenc film', 2000, 2100),
                ('c1', 's1', '20261126200000 +0000', '20261126210000 +0000', 'Kedvenc Film', 'Duplikátum', 'kedvenc film', 2000, 2100),
                ('c2', 's1', '20261126200000 +0000', '20261126210000 +0000', 'Kedvenc Film', 'Rejtett csatorna', 'kedvenc film', 2000, 2100),
                ('c3', 's2', '20261126220000 +0000', '20261126230000 +0000', 'Másik Műsor', 'S2 adás', 'masik musor', 2200, 2300),
                ('c1', 's1', '20261126230000 +0000', '20261126233000 +0000', 'Nem Kedvenc', 'Egyéb', 'nem kedvenc', 2400, 2500);",
        )
        .unwrap();

        let now_ts = 1500;

        // Upcoming across all sources: excludes past (1100 < 1500), hidden (c2), non-favorite, and deduplicates c1/2000
        let all_upcoming = query_favorite_programmes_airings(&conn, None, false, now_ts).unwrap();
        assert_eq!(all_upcoming.len(), 2);
        assert_eq!(all_upcoming[0].channel_name, "M1 HD");
        assert_eq!(all_upcoming[0].source_name, "Forrás 1");
        assert_eq!(all_upcoming[1].source_id, "s2");
        assert_eq!(all_upcoming[1].time_offset, 1);

        // Include past: adds the 1000..1100 airing
        let all_with_past = query_favorite_programmes_airings(&conn, Some("__ALL_SOURCES__"), true, now_ts).unwrap();
        assert_eq!(all_with_past.len(), 3);

        // Filter by specific source s2
        let s2_only = query_favorite_programmes_airings(&conn, Some("s2"), false, now_ts).unwrap();
        assert_eq!(s2_only.len(), 1);
        assert_eq!(s2_only[0].title, "Másik Műsor");

        // Filter by favorite channels only (__ALL_FAVORITES__)
        let fav_ch_only = query_favorite_programmes_airings(&conn, Some("__ALL_FAVORITES__"), false, now_ts).unwrap();
        assert_eq!(fav_ch_only.len(), 1);
        assert_eq!(fav_ch_only[0].channel_id, "c1");
    }
}

