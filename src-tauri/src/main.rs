#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use tokio::sync::Mutex;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use flate2::read::GzDecoder;
use regex::Regex;

struct AppState {
    db: Mutex<Connection>,
    http: reqwest::Client,
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

const EMBEDDED_ENGLISH_LNG: &str = include_str!("../../public/languages/english.lng");
const EMBEDDED_HUNGARIAN_LNG: &str = include_str!("../../public/languages/hungarian.lng");

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
    channel_count: Option<i64>,
    #[serde(default)]
    favorite_count: Option<i64>,
    #[serde(default)]
    hidden_count: Option<i64>,
}

fn get_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("epg-viewer");
    fs::create_dir_all(&path).ok();
    path.push("epg.db");
    path
}

fn get_images_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("epg-viewer");
    fs::create_dir_all(&path).ok();
    path.push("images.db");
    path
}

fn open_and_init_db() -> Connection {
    let conn = Connection::open(get_db_path()).expect("DB megnyitási hiba");

    // images.db csatolása külön adatbázisként
    let images_db_path = get_images_db_path();
    let attach_sql = format!(
        "ATTACH DATABASE '{}' AS images_db",
        images_db_path.to_string_lossy().replace('\'', "''")
    );
    conn.execute(&attach_sql, []).expect("images.db csatolási hiba");

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
        CREATE INDEX IF NOT EXISTS images_db.idx_image_urls_image_id ON image_urls (image_id);",
    ).expect("images_db táblák inicializálási hiba");

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
                    let cr_time = if created_at.is_empty() { "datetime('now')" } else { &created_at };

                    let _ = conn.execute(
                        "INSERT OR IGNORE INTO images_db.images (sha256, mime_type, byte_size, data, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![hash_hex, mime, byte_size, data, cr_time],
                    );

                    let img_id: Option<i64> = conn.query_row(
                        "SELECT id FROM images_db.images WHERE sha256 = ?1",
                        params![hash_hex],
                        |r| r.get(0),
                    ).ok();

                    if let Some(id) = img_id {
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO images_db.image_urls (url, image_id, image_type, created_at)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![url, id, img_type, cr_time],
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

    // WAL mód és teljesítmény-optimalizáló PRAGMA beállítások mindkét adatbázishoz
    let _ = conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA cache_size = -64000;
         PRAGMA temp_store = MEMORY;
         PRAGMA images_db.journal_mode = WAL;
         PRAGMA images_db.synchronous = NORMAL;
         PRAGMA images_db.cache_size = -64000;",
    );

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
    ).unwrap();

    // Meglévő adatbázis sémakorrekció, ha hiányoznának az új oszlopok
    let _ = conn.execute("ALTER TABLE sources ADD COLUMN country_code TEXT", []);
    let _ = conn.execute("ALTER TABLE sources ADD COLUMN time_offset INTEGER", []);
    let _ = conn.execute("ALTER TABLE sources ADD COLUMN sort_position INTEGER", []);
    let _ = conn.execute("ALTER TABLE channel_user_settings ADD COLUMN custom_name TEXT", []);
    let _ = conn.execute("ALTER TABLE tmdb_cache ADD COLUMN hu_title TEXT", []);
    let _ = conn.execute("ALTER TABLE tmdb_cache ADD COLUMN en_title TEXT", []);
    let _ = conn.execute("ALTER TABLE tmdb_cache ADD COLUMN original_language TEXT", []);

    // Tisztítsuk meg a korábban hibásan felülírt franchise gyorsítótár bejegyzéseket (pl. Jurassic World -> Fallen Kingdom)
    conn.execute("DELETE FROM tmdb_cache WHERE query_title = 'Jurassic World' AND (title LIKE '%gefallene%' OR title LIKE '%Fallen Kingdom%')", []).ok();
    conn.execute("DELETE FROM tmdb_cache WHERE query_title = 'G.I. Joe' AND (title LIKE '%Megtorlás%' OR title LIKE '%Retaliation%')", []).ok();

    // Korábbi alapértelmezett források törlése a tiszta induláshoz
    conn.execute("DELETE FROM sources WHERE id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM channels WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM programmes WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM channel_orders WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();

    // Kezdeti nyelvi sablonok feltöltése, ha üres lenne
    let lang_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM app_languages", [], |r| r.get(0))
        .unwrap_or(0);
    if lang_count == 0 {
        let _ = conn.execute(
            "INSERT OR IGNORE INTO app_languages (lang, content, updated_at) VALUES ('english', ?1, datetime('now', 'localtime'))",
            params![EMBEDDED_ENGLISH_LNG],
        );
        let _ = conn.execute(
            "INSERT OR IGNORE INTO app_languages (lang, content, updated_at) VALUES ('hungarian', ?1, datetime('now', 'localtime'))",
            params![EMBEDDED_HUNGARIAN_LNG],
        );
    }

    conn
}

async fn get_all_favorites_epg_data(state: &AppState) -> Result<EpgResponse, String> {
    let conn = state.db.lock().await;

    let mut stmt = conn.prepare(
        "SELECT c.id, 
                c.source_id,
                COALESCE(s.name, '') AS source_name,
                COALESCE(NULLIF(cus.custom_name, ''), c.name) AS display_name,
                c.name AS original_name,
                cus.custom_name,
                c.icon,
                cus.custom_icon,
                img.data AS cached_icon,
                COALESCE(cus.is_favorite, 0),
                COALESCE(cus.is_hidden, 0),
                COALESCE(s.country_code, 'HU') AS country_code,
                COALESCE(s.time_offset, 0) AS time_offset
         FROM channels c 
         JOIN sources s ON s.id = c.source_id
         JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id 
         LEFT JOIN channel_orders co_fav ON co_fav.source_id = '__ALL_FAVORITES__' AND co_fav.channel_id = (c.source_id || '::' || c.id)
         LEFT JOIN images_db.image_urls iu ON iu.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
         LEFT JOIN images_db.images img ON img.id = iu.image_id
         WHERE cus.is_favorite = 1 AND COALESCE(cus.is_hidden, 0) = 0
         ORDER BY 
            CASE WHEN co_fav.position IS NOT NULL THEN 0 ELSE 1 END,
            co_fav.position ASC,
            s.rowid ASC,
            CASE WHEN co.position IS NOT NULL THEN 0 ELSE 1 END, 
            co.position ASC, 
            display_name ASC"
    ).map_err(|e| e.to_string())?;

    let channel_iter = stmt.query_map([], |row| {
        let raw_id: String = row.get(0)?;
        let src_id: String = row.get(1)?;
        let src_name: String = row.get(2)?;
        let composite_id = format!("{}::{}", src_id, raw_id);
        let is_fav: i32 = row.get(9)?;
        let is_hid: i32 = row.get(10)?;
        let country_code: String = row.get(11)?;
        Ok(Channel {
            id: composite_id,
            name: row.get(3)?,
            original_name: row.get(4)?,
            custom_name: row.get(5)?,
            icon: row.get(6)?,
            custom_icon: row.get(7)?,
            cached_icon: row.get(8)?,
            is_favorite: is_fav == 1,
            is_hidden: is_hid == 1,
            source_id: Some(src_id),
            source_name: Some(src_name),
            country_code: Some(country_code),
        })
    }).map_err(|e| e.to_string())?;

    let mut channels = Vec::new();
    for ch in channel_iter {
        if let Ok(c) = ch { channels.push(c); }
    }

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
    for pr in prog_iter {
        if let Ok(p) = pr { programmes.push(p); }
    }

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

    let mut stmt = conn.prepare(
        "SELECT c.id, 
                COALESCE(NULLIF(cus.custom_name, ''), c.name) AS display_name,
                c.name AS original_name,
                cus.custom_name,
                c.icon,
                cus.custom_icon,
                img.data AS cached_icon,
                COALESCE(cus.is_favorite, 0),
                COALESCE(cus.is_hidden, 0),
                COALESCE(s.country_code, 'HU') AS country_code,
                COALESCE(s.name, '') AS source_name
         FROM channels c 
         JOIN sources s ON s.id = c.source_id
         LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id 
         LEFT JOIN images_db.image_urls iu ON iu.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
         LEFT JOIN images_db.images img ON img.id = iu.image_id
         WHERE c.source_id = ?1 
         ORDER BY 
            COALESCE(cus.is_favorite, 0) DESC,
            CASE WHEN co.position IS NOT NULL THEN 0 ELSE 1 END, 
            co.position ASC, 
            display_name ASC"
    ).map_err(|e| e.to_string())?;

    let channel_iter = stmt.query_map(params![source_id], |row| {
        let is_fav: i32 = row.get(7)?;
        let is_hid: i32 = row.get(8)?;
        let country_code: String = row.get(9)?;
        let src_name: String = row.get(10)?;
        Ok(Channel {
            id: row.get(0)?,
            name: row.get(1)?,
            original_name: row.get(2)?,
            custom_name: row.get(3)?,
            icon: row.get(4)?,
            custom_icon: row.get(5)?,
            cached_icon: row.get(6)?,
            is_favorite: is_fav == 1,
            is_hidden: is_hid == 1,
            source_id: Some(source_id.clone()),
            source_name: Some(src_name),
            country_code: Some(country_code),
        })
    }).map_err(|e| e.to_string())?;

    let mut channels = Vec::new();
    for ch in channel_iter {
        if let Ok(c) = ch { channels.push(c); }
    }

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
    for pr in prog_iter {
        if let Ok(p) = pr { programmes.push(p); }
    }

    Ok(EpgResponse { channels, programmes })
}

#[tauri::command]
async fn get_sources(state: tauri::State<'_, AppState>) -> Result<Vec<Source>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.url, s.last_downloaded, s.country_code, s.time_offset,
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
            channel_count: Some(row.get(6)?),
            favorite_count: Some(row.get(7)?),
            hidden_count: Some(row.get(8)?),
        })
    }).map_err(|e| e.to_string())?;

    let mut sources = Vec::new();
    for s in rows {
        if let Ok(src) = s { sources.push(src); }
    }
    Ok(sources)
}

#[tauri::command]
async fn save_source(state: tauri::State<'_, AppState>, source: Source) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO sources (id, name, url, last_downloaded, country_code, time_offset) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET 
            name = excluded.name, 
            url = excluded.url, 
            last_downloaded = COALESCE(excluded.last_downloaded, sources.last_downloaded),
            country_code = excluded.country_code,
            time_offset = excluded.time_offset",
        params![source.id, source.name, source.url, source.last_downloaded, source.country_code, source.time_offset],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn delete_source(state: tauri::State<'_, AppState>, source_id: String) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM sources WHERE id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channels WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM programmes WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_orders WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_sort_presets WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_user_settings WHERE source_id = ?1", params![source_id]).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
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
    tx.execute("UPDATE sources SET last_downloaded = NULL", []).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn get_favorites(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT title FROM favorites ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut favs = Vec::new();
    for f in rows {
        if let Ok(title) = f { favs.push(title); }
    }
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
async fn set_favorites(state: tauri::State<'_, AppState>, favorites: Vec<String>) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM favorites", []).ok();
    for f in favorites {
        tx.execute("INSERT INTO favorites (title) VALUES (?1)", params![f]).ok();
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_channel_order(state: tauri::State<'_, AppState>, source_id: String) -> Result<Vec<String>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare(
        "SELECT channel_id FROM channel_orders WHERE source_id = ?1 ORDER BY position ASC"
    ).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![source_id], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut order = Vec::new();
    for o in rows {
        if let Ok(ch_id) = o { order.push(ch_id); }
    }
    Ok(order)
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
    for p in rows {
        if let Ok(item) = p { presets.push(item); }
    }
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

#[tauri::command]
async fn get_cached_logo(state: tauri::State<'_, AppState>, url: String) -> Result<Option<String>, String> {
    let clean_url = url.trim();
    if clean_url.is_empty() {
        return Ok(None);
    }
    let conn = state.db.lock().await;
    let res: Option<String> = conn.query_row(
        "SELECT data FROM images_db.logo_cache WHERE url = ?1",
        params![clean_url],
        |row| row.get(0),
    ).ok();
    Ok(res)
}

async fn cache_logo_inner(state: &AppState, url: String) -> Result<String, String> {
    let clean_url = url.trim().to_string();
    if clean_url.is_empty() {
        return Err("Üres URL".to_string());
    }
    if clean_url.starts_with("data:") {
        return Ok(clean_url);
    }

    // 1. Előzetes gyorsítótár ellenőrzés (a zár azonnal feloldódik a hálózati kérés előtt)
    {
        let conn = state.db.lock().await;
        let cached: Option<String> = conn.query_row(
            "SELECT img.data FROM images_db.image_urls iu
             JOIN images_db.images img ON img.id = iu.image_id
             WHERE iu.url = ?1",
            params![clean_url],
            |row| row.get(0),
        ).ok();

        if let Some(data) = cached {
            if !data.is_empty() {
                return Ok(data);
            }
        }
    }

    if !clean_url.starts_with("http://") && !clean_url.starts_with("https://") {
        return Err("Nem támogatott URL protokoll".to_string());
    }

    // 2. Hálózati letöltés DB zárolás nélkül
    let resp = state.http.get(&clean_url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP hiba: {}", resp.status()));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Err("Üres válasz az ikonkérésre".to_string());
    }

    let mime = if let Some(ct) = content_type {
        let ct_lower = ct.to_lowercase();
        if ct_lower.contains("svg") {
            "image/svg+xml".to_string()
        } else if ct_lower.contains("png") {
            "image/png".to_string()
        } else if ct_lower.contains("jpeg") || ct_lower.contains("jpg") {
            "image/jpeg".to_string()
        } else if ct_lower.contains("webp") {
            "image/webp".to_string()
        } else if ct_lower.contains("gif") {
            "image/gif".to_string()
        } else {
            ct.split(';').next().unwrap_or("image/png").trim().to_string()
        }
    } else if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        "image/png".to_string()
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg".to_string()
    } else if bytes.starts_with(&[0x47, 0x49, 0x46]) {
        "image/gif".to_string()
    } else if bytes.starts_with(b"<svg") || clean_url.ends_with(".svg") {
        "image/svg+xml".to_string()
    } else {
        "image/png".to_string()
    };

    // 3. Tartalom-alapú SHA-256 hash generálás a nyers bájtokból
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash_hex = format!("{:x}", hasher.finalize());

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_uri = format!("data:{};base64,{}", mime, b64);
    let byte_size = bytes.len() as i64;
    let img_type = if clean_url.contains("image.tmdb.org") { "poster" } else { "logo" };

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
                params![hash_hex, mime, byte_size, data_uri],
            ).map_err(|e| e.to_string())?;
            conn.last_insert_rowid()
        }
    };

    conn.execute(
        "INSERT INTO images_db.image_urls (url, image_id, image_type, created_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(url) DO UPDATE SET image_id = excluded.image_id, created_at = excluded.created_at",
        params![clean_url, image_id, img_type],
    ).map_err(|e| e.to_string())?;

    Ok(data_uri)
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

#[tauri::command]
async fn find_duplicate_logos(state: tauri::State<'_, AppState>) -> Result<DuplicateReport, String> {
    let conn = state.db.lock().await;

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

    let mut stmt = conn.prepare(
        "SELECT iu.image_id, img.sha256, img.mime_type, img.byte_size, img.data, COUNT(iu.url) as cnt
         FROM images_db.image_urls iu
         JOIN images_db.images img ON img.id = iu.image_id
         WHERE iu.image_type = 'logo'
         GROUP BY iu.image_id
         HAVING cnt > 1
         ORDER BY cnt DESC, img.byte_size DESC"
    ).map_err(|e| e.to_string())?;

    let group_rows = stmt.query_map([], |row| {
        let image_id: i64 = row.get(0)?;
        let sha256: String = row.get(1)?;
        let mime_type: String = row.get(2)?;
        let byte_size: i64 = row.get(3)?;
        let data: String = row.get(4)?;
        Ok((image_id, sha256, mime_type, byte_size, data))
    }).map_err(|e| e.to_string())?;

    let mut duplicate_groups = Vec::new();
    let mut redundant_count = 0;
    let mut potential_saved_bytes = 0;

    for g in group_rows.flatten() {
        let (image_id, sha256, mime_type, byte_size, data) = g;

        let mut url_stmt = conn.prepare(
            "SELECT url FROM images_db.image_urls WHERE image_id = ?1 AND image_type = 'logo' ORDER BY url ASC"
        ).map_err(|e| e.to_string())?;

        let urls: Vec<String> = url_stmt.query_map(params![image_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();

        let mut ch_names = Vec::new();
        for u in &urls {
            let mut ch_stmt = conn.prepare(
                "SELECT DISTINCT COALESCE(NULLIF(cus.custom_name, ''), c.name) AS ch_name
                 FROM channels c
                 LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
                 WHERE COALESCE(NULLIF(cus.custom_icon, ''), c.icon) = ?1"
            ).map_err(|e| e.to_string())?;

            let names: Vec<String> = ch_stmt.query_map(params![u], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .flatten()
                .collect();
            ch_names.extend(names);
        }
        ch_names.sort();
        ch_names.dedup();

        let count = urls.len();
        if count > 1 {
            redundant_count += count - 1;
            potential_saved_bytes += (count - 1) as i64 * byte_size;

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
    }

    Ok(DuplicateReport {
        total_unique_images,
        total_urls,
        duplicate_groups,
        redundant_count,
        potential_saved_bytes,
    })
}

#[tauri::command]
async fn deduplicate_logos(state: tauri::State<'_, AppState>) -> Result<DeduplicationResult, String> {
    let conn = state.db.lock().await;

    let mut stmt = conn.prepare(
        "SELECT iu.image_id, img.byte_size, COUNT(iu.url) as cnt
         FROM images_db.image_urls iu
         JOIN images_db.images img ON img.id = iu.image_id
         WHERE iu.image_type = 'logo'
         GROUP BY iu.image_id
         HAVING cnt > 1"
    ).map_err(|e| e.to_string())?;

    let groups: Vec<(i64, i64, usize)> = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    }).map_err(|e| e.to_string())?
      .flatten()
      .collect();

    let mut cleaned_groups = 0;
    let mut removed_urls = 0;
    let mut freed_bytes = 0;

    for (image_id, byte_size, _) in groups {
        let mut url_stmt = conn.prepare(
            "SELECT url FROM images_db.image_urls WHERE image_id = ?1 AND image_type = 'logo' ORDER BY length(url) ASC, url ASC"
        ).map_err(|e| e.to_string())?;

        let urls: Vec<String> = url_stmt.query_map(params![image_id], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();

        if urls.len() <= 1 {
            continue;
        }

        let canonical_url = &urls[0];
        let redundant_urls = &urls[1..];

        for red_url in redundant_urls {
            let _ = conn.execute(
                "UPDATE channel_user_settings SET custom_icon = ?1 WHERE custom_icon = ?2",
                params![canonical_url, red_url],
            );
            let _ = conn.execute(
                "DELETE FROM images_db.image_urls WHERE url = ?1",
                params![red_url],
            );
            removed_urls += 1;
            freed_bytes += byte_size;
        }
        cleaned_groups += 1;
    }

    let removed_images = conn.execute(
        "DELETE FROM images_db.images WHERE id NOT IN (SELECT DISTINCT image_id FROM images_db.image_urls)",
        [],
    ).unwrap_or(0);

    let _ = conn.execute("VACUUM images_db", []);

    Ok(DeduplicationResult {
        cleaned_groups,
        removed_urls,
        removed_images,
        freed_bytes,
    })
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
struct AutoMatchSummary {
    total_channels: usize,
    with_logo_count: usize,
    missing_logos: usize,
    matched_count: usize,
    failed_count: usize,
    skipped_existing: usize,
    matches: Vec<MatchedChannelItem>,
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

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TvLogoFileItem {
    name: String,
    download_url: Option<String>,
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

static TV_LOGO_TREE_CACHE: RwLock<Option<Vec<String>>> = RwLock::new(None);
static IPTV_LOGOS_MEM_CACHE: RwLock<Option<Vec<IptvOrgLogoItem>>> = RwLock::new(None);

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IptvOrgChannel {
    id: String,
    name: String,
    #[serde(default)]
    alt_names: Vec<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    logo: Option<String>,
}

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

fn clean_channel_name(raw_name: &str) -> String {
    let re_at = Regex::new(r"@[A-Za-z0-9]+").unwrap();
    let mut name = re_at.replace_all(raw_name, "").to_string();

    let re_prefix = Regex::new(r"(?i)^(HU|HUN|RO|DE|AT|SK|CZ|UK|US|PT|PL):\s*").unwrap();
    name = re_prefix.replace(&name, "").to_string();

    let re_lang = Regex::new(r"(?i)\s*[\(\[]\s*(HU|HUN|RO|DE|AT|SK|CZ|UK|US|PT|PL|ENG|HUN/ENG)\s*[\)\]]").unwrap();
    name = re_lang.replace_all(&name, "").to_string();

    let re_provider = Regex::new(r"(?i)\s*[\(\[]\s*(Telekom|Vodafone|Digi|Yettel|Direct\s*One|UPC|backup|test|\d+)\s*[\)\]]").unwrap();
    name = re_provider.replace_all(&name, "").to_string();

    let re_quality_bracket = Regex::new(r"(?i)\s*[\(\[]\s*(HD|FHD|4K|UHD|SD|HEVC|H\.?265|1080[pi]|720[pi]|50fps|60fps|100fps)\s*[\)\]]").unwrap();
    name = re_quality_bracket.replace_all(&name, "").to_string();

    let re_quality_word = Regex::new(r"(?i)\b(HD|FHD|4K|UHD|SD|HEVC|H\.?265|1080[pi]|720[pi]|50fps|60fps|100fps)\b").unwrap();
    name = re_quality_word.replace_all(&name, "").to_string();

    let re_region_bracket = Regex::new(r"(?i)\s*[\(\[]\s*(pacific|east|west)\s*[\)\]]").unwrap();
    name = re_region_bracket.replace_all(&name, "").to_string();

    let re_region_word = Regex::new(r"(?i)\b(pacific|east|west)\b").unwrap();
    name = re_region_word.replace_all(&name, "").to_string();

    // Domain extensions: .hu, .pt, .us, .uk, .de, .at, .ro, etc.
    let re_domains = Regex::new(r"(?i)\.(hu|pt|us|uk|de|at|ro|sk|cz|pl|fr|it|es|nl|com|net|org|tv|ca|ch|se|no|dk|fi|eu|int)\b").unwrap();
    name = re_domains.replace_all(&name, "").to_string();

    let re_spaces = Regex::new(r"\s+").unwrap();
    name = re_spaces.replace_all(&name, " ").to_string();
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

fn map_country_code(country_code: &str) -> (&'static str, &'static str) {
    match country_code.trim().to_uppercase().as_str() {
        "HU" => ("hungary", "hu"),
        "DE" => ("germany", "de"),
        "AT" => ("austria", "at"),
        "RO" => ("romania", "ro"),
        "SK" => ("slovakia", "sk"),
        "CZ" => ("czech-republic", "cz"),
        "US" => ("united-states", "us"),
        "GB" | "UK" | "EN" => ("united-kingdom", "uk"),
        "PT" => ("portugal", "pt"),
        "IT" => ("italy", "it"),
        "FR" => ("france", "fr"),
        "ES" => ("spain", "es"),
        "PL" => ("poland", "pl"),
        "NL" => ("netherlands", "nl"),
        "HR" => ("croatia", "hr"),
        "RS" => ("serbia", "rs"),
        _ => ("hungary", "hu"),
    }
}


#[allow(dead_code)]
fn match_tv_logo(items: &[TvLogoFileItem], country_dir: &str, country_code: &str, slug: &str) -> Option<LogoSearchResult> {
    let target_exact = format!("{}-{}.png", slug, country_code);
    let target_simple = format!("{}.png", slug);

    for item in items {
        if item.name.eq_ignore_ascii_case(&target_exact) {
            let dl = item.download_url.clone().unwrap_or_else(|| {
                format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/countries/{}/{}", country_dir, item.name)
            });
            return Some(LogoSearchResult {
                url: dl.clone(),
                thumb_url: dl,
                source_name: "tv-logo".to_string(),
                format: "PNG".to_string(),
                title: item.name.trim_end_matches(".png").to_string(),
            });
        }
    }

    for item in items {
        if item.name.eq_ignore_ascii_case(&target_simple) {
            let dl = item.download_url.clone().unwrap_or_else(|| {
                format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/countries/{}/{}", country_dir, item.name)
            });
            return Some(LogoSearchResult {
                url: dl.clone(),
                thumb_url: dl,
                source_name: "tv-logo".to_string(),
                format: "PNG".to_string(),
                title: item.name.trim_end_matches(".png").to_string(),
            });
        }
    }

    for item in items {
        let name_no_ext = item.name.trim_end_matches(".png");
        let name_clean = if let Some(stripped) = name_no_ext.strip_suffix(&format!("-{}", country_code)) {
            stripped
        } else {
            name_no_ext
        };
        if name_clean.eq_ignore_ascii_case(slug) {
            let dl = item.download_url.clone().unwrap_or_else(|| {
                format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/countries/{}/{}", country_dir, item.name)
            });
            return Some(LogoSearchResult {
                url: dl.clone(),
                thumb_url: dl,
                source_name: "tv-logo".to_string(),
                format: "PNG".to_string(),
                title: item.name.trim_end_matches(".png").to_string(),
            });
        }
    }

    None
}


async fn fetch_tv_logo_full_tree(state: &AppState) -> Result<Vec<String>, String> {
    let cache_key = "tv_logo_full_tree_paths";
    let conn = state.db.lock().await;

    let cached: Option<String> = conn.query_row(
        "SELECT content FROM external_logo_cache WHERE source_key = ?1 AND updated_at > datetime('now', '-7 days')",
        params![cache_key],
        |row| row.get(0),
    ).ok();

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
        _ => {
            let fallback: Option<String> = conn.query_row(
                "SELECT content FROM external_logo_cache WHERE source_key = ?1",
                params![cache_key],
                |row| row.get(0),
            ).ok();
            if let Some(json_str) = fallback {
                if let Ok(paths) = serde_json::from_str::<Vec<String>>(&json_str) {
                    if !paths.is_empty() {
                        return Ok(paths);
                    }
                }
            }
        }
    }

    Ok(Vec::new())
}

fn match_tv_logo_from_tree(
    paths: &[String],
    country_dir: &str,
    country_code: &str,
    slug: &str,
) -> Option<LogoSearchResult> {
    let target_exact = format!("{}-{}.png", slug, country_code);
    let target_simple = format!("{}.png", slug);
    let country_prefix = format!("countries/{}/", country_dir);

    // 1. Target country folder
    for p in paths {
        if p.starts_with(&country_prefix) {
            let fname = &p[country_prefix.len()..];
            if fname.eq_ignore_ascii_case(&target_exact) || fname.eq_ignore_ascii_case(&target_simple) {
                let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
                return Some(LogoSearchResult {
                    url: url.clone(),
                    thumb_url: url,
                    source_name: "tv-logo".to_string(),
                    format: "PNG".to_string(),
                    title: fname.trim_end_matches(".png").to_string(),
                });
            }
        }
    }

    // 2. International folder
    let int_prefix = "countries/international/";
    let target_int = format!("{}-int.png", slug);
    for p in paths {
        if p.starts_with(int_prefix) {
            let fname = &p[int_prefix.len()..];
            if fname.eq_ignore_ascii_case(&target_int) || fname.eq_ignore_ascii_case(&target_simple) {
                let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
                return Some(LogoSearchResult {
                    url: url.clone(),
                    thumb_url: url,
                    source_name: "tv-logo (int)".to_string(),
                    format: "PNG".to_string(),
                    title: fname.trim_end_matches(".png").to_string(),
                });
            }
        }
    }

    // 3. ANY country folder in the entire tree (without language restriction)
    for p in paths {
        let fname = p.rsplit('/').next().unwrap_or(p.as_str());
        let name_no_ext = fname.trim_end_matches(".png");
        let matches = if let Some(stripped) = name_no_ext.rsplit_once('-') {
            stripped.0.eq_ignore_ascii_case(slug)
        } else {
            name_no_ext.eq_ignore_ascii_case(slug)
        };
        if matches {
            let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
            return Some(LogoSearchResult {
                url: url.clone(),
                thumb_url: url,
                source_name: "tv-logo (global)".to_string(),
                format: "PNG".to_string(),
                title: name_no_ext.to_string(),
            });
        }
    }

    // 4. Fuzzy egyezés a célország könyvtárában (ha pontos egyezés nem volt, Jaro-Winkler >= 0.85)
    let country_prefix = format!("countries/{}/", country_dir);
    let mut best_target: Option<(f64, LogoSearchResult)> = None;

    for p in paths {
        if p.starts_with(&country_prefix) {
            let fname = &p[country_prefix.len()..];
            let name_no_ext = fname.trim_end_matches(".png");
            let base_name = if let Some(stripped) = name_no_ext.rsplit_once('-') {
                stripped.0
            } else {
                name_no_ext
            };
            let score = strsim::jaro_winkler(slug, base_name);
            if score >= 0.85 {
                if best_target.as_ref().map_or(true, |(best, _)| score > *best) {
                    let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
                    best_target = Some((score, LogoSearchResult {
                        url: url.clone(),
                        thumb_url: url,
                        source_name: "tv-logo".to_string(),
                        format: "PNG".to_string(),
                        title: name_no_ext.to_string(),
                    }));
                }
            }
        }
    }

    if let Some((_, res)) = best_target {
        return Some(res);
    }

    // 5. Fuzzy egyezés nemzetközi és egyéb mappákban (szigorúbb küszöb: Jaro-Winkler >= 0.88)
    let mut best_global: Option<(f64, LogoSearchResult)> = None;
    for p in paths {
        let fname = p.rsplit('/').next().unwrap_or(p.as_str());
        let name_no_ext = fname.trim_end_matches(".png");
        let base_name = if let Some(stripped) = name_no_ext.rsplit_once('-') {
            stripped.0
        } else {
            name_no_ext
        };
        let score = strsim::jaro_winkler(slug, base_name);
        if score >= 0.88 {
            if best_global.as_ref().map_or(true, |(best, _)| score > *best) {
                let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
                let src_name = if p.starts_with("countries/international/") {
                    "tv-logo (int)".to_string()
                } else {
                    "tv-logo (global)".to_string()
                };
                best_global = Some((score, LogoSearchResult {
                    url: url.clone(),
                    thumb_url: url,
                    source_name: src_name,
                    format: "PNG".to_string(),
                    title: name_no_ext.to_string(),
                }));
            }
        }
    }

    if let Some((_, res)) = best_global {
        return Some(res);
    }

    None
}

fn search_tv_logo_from_tree(
    paths: &[String],
    country_dir: &str,
    country_code: &str,
    slug: &str,
    clean_name: &str,
) -> Vec<LogoSearchResult> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Some(exact) = match_tv_logo_from_tree(paths, country_dir, country_code, slug) {
        seen.insert(exact.url.clone());
        results.push(exact);
    }

    let clean_lower = clean_name.to_lowercase();
    let country_prefix = format!("countries/{}/", country_dir);
    let int_prefix = "countries/international/";

    // 1. Target country
    for p in paths {
        if p.starts_with(&country_prefix) {
            let fname = &p[country_prefix.len()..];
            let name_lower = fname.to_lowercase();
            let is_match = (slug.len() >= 3 && name_lower.contains(slug))
                || (!clean_lower.is_empty() && name_lower.contains(&clean_lower))
                || strsim::jaro_winkler(slug, &name_lower.trim_end_matches(".png")) >= 0.82;
            if is_match {
                let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
                if seen.insert(url.clone()) {
                    results.push(LogoSearchResult {
                        url: url.clone(),
                        thumb_url: url,
                        source_name: "tv-logo".to_string(),
                        format: "PNG".to_string(),
                        title: fname.trim_end_matches(".png").to_string(),
                    });
                }
            }
        }
    }

    // 2. International
    for p in paths {
        if p.starts_with(int_prefix) {
            let fname = &p[int_prefix.len()..];
            let name_lower = fname.to_lowercase();
            let is_match = (slug.len() >= 3 && name_lower.contains(slug))
                || (!clean_lower.is_empty() && name_lower.contains(&clean_lower))
                || strsim::jaro_winkler(slug, &name_lower.trim_end_matches(".png")) >= 0.84;
            if is_match {
                let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
                if seen.insert(url.clone()) {
                    results.push(LogoSearchResult {
                        url: url.clone(),
                        thumb_url: url,
                        source_name: "tv-logo (int)".to_string(),
                        format: "PNG".to_string(),
                        title: fname.trim_end_matches(".png").to_string(),
                    });
                }
            }
        }
    }

    // 3. Any country across the entire tree
    for p in paths {
        let fname = p.rsplit('/').next().unwrap_or(p.as_str());
        let name_lower = fname.to_lowercase();
        let is_match = (slug.len() >= 3 && name_lower.contains(slug))
            || (!clean_lower.is_empty() && name_lower.contains(&clean_lower))
            || strsim::jaro_winkler(slug, &name_lower.trim_end_matches(".png")) >= 0.86;
        if is_match {
            let url = format!("https://raw.githubusercontent.com/tv-logo/tv-logos/main/{}", p);
            if seen.insert(url.clone()) {
                results.push(LogoSearchResult {
                    url: url.clone(),
                    thumb_url: url,
                    source_name: "tv-logo (global)".to_string(),
                    format: "PNG".to_string(),
                    title: fname.trim_end_matches(".png").to_string(),
                });
            }
        }
    }

    results
}


async fn fetch_iptv_org_logos(state: &AppState) -> Result<Vec<IptvOrgLogoItem>, String> {
    let cache_key = "iptv_org_logos".to_string();
    let conn = state.db.lock().await;

    let cached: Option<String> = conn.query_row(
        "SELECT content FROM external_logo_cache WHERE source_key = ?1 AND updated_at > datetime('now', '-7 days')",
        params![cache_key],
        |row| row.get(0),
    ).ok();

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
                    let _ = conn.execute(
                        "INSERT INTO external_logo_cache (source_key, content, updated_at) VALUES (?1, ?2, datetime('now'))
                         ON CONFLICT(source_key) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
                        params![cache_key, text],
                    );
                    return Ok(logos);
                }
            }
        }
        _ => {
            let fallback: Option<String> = conn.query_row(
                "SELECT content FROM external_logo_cache WHERE source_key = ?1",
                params![cache_key],
                |row| row.get(0),
            ).ok();
            if let Some(json_str) = fallback {
                if let Ok(logos) = serde_json::from_str::<Vec<IptvOrgLogoItem>>(&json_str) {
                    if !logos.is_empty() {
                        return Ok(logos);
                    }
                }
            }
        }
    }

    Ok(Vec::new())
}

fn match_iptv_org_logos(
    logos: &[IptvOrgLogoItem],
    slug: &str,
    country_code: &str,
) -> Option<LogoSearchResult> {
    let suffix = format!(".{}", country_code.to_lowercase());
    let slug_alphanumeric: String = slug.chars().filter(|c| c.is_alphanumeric()).collect();

    // 1. Pontos egyezés
    for item in logos {
        let ch_lower = item.channel.to_lowercase();
        if ch_lower.ends_with(&suffix) {
            let base = ch_lower.trim_end_matches(&suffix);
            let base_alphanumeric: String = base.chars().filter(|c| c.is_alphanumeric()).collect();
            if base == slug || (!slug_alphanumeric.is_empty() && base_alphanumeric == slug_alphanumeric) {
                let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };
                return Some(LogoSearchResult {
                    url: item.url.clone(),
                    thumb_url: item.url.clone(),
                    source_name: "iptv-org".to_string(),
                    format: fmt.to_string(),
                    title: item.channel.clone(),
                });
            }
        }
    }

    // 2. Fuzzy egyezés az adott ország logóiban (Jaro-Winkler >= 0.88)
    if !slug_alphanumeric.is_empty() {
        let mut best: Option<(f64, &IptvOrgLogoItem)> = None;
        for item in logos {
            let ch_lower = item.channel.to_lowercase();
            if ch_lower.ends_with(&suffix) {
                let base = ch_lower.trim_end_matches(&suffix);
                let base_alphanumeric: String = base.chars().filter(|c| c.is_alphanumeric()).collect();
                let score = strsim::jaro_winkler(&slug_alphanumeric, &base_alphanumeric);
                if score >= 0.88 {
                    if best.as_ref().map_or(true, |(b_score, _)| score > *b_score) {
                        best = Some((score, item));
                    }
                }
            }
        }
        if let Some((_, item)) = best {
            let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };
            return Some(LogoSearchResult {
                url: item.url.clone(),
                thumb_url: item.url.clone(),
                source_name: "iptv-org (fuzzy)".to_string(),
                format: fmt.to_string(),
                title: item.channel.clone(),
            });
        }
    }

    None
}

fn match_iptv_org_logos_any(
    logos: &[IptvOrgLogoItem],
    slug: &str,
) -> Option<LogoSearchResult> {
    let slug_alphanumeric: String = slug.chars().filter(|c| c.is_alphanumeric()).collect();
    if slug_alphanumeric.is_empty() {
        return None;
    }

    // 1. Pontos egyezés
    for item in logos {
        let ch_lower = item.channel.to_lowercase();
        let base = ch_lower.split('.').next().unwrap_or(&ch_lower);
        let base_alphanumeric: String = base.chars().filter(|c| c.is_alphanumeric()).collect();
        if base == slug || base_alphanumeric == slug_alphanumeric {
            let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };
            return Some(LogoSearchResult {
                url: item.url.clone(),
                thumb_url: item.url.clone(),
                source_name: "iptv-org".to_string(),
                format: fmt.to_string(),
                title: item.channel.clone(),
            });
        }
    }

    // 2. Fuzzy egyezés globálisan (szigorúbb küszöb: Jaro-Winkler >= 0.90)
    let mut best: Option<(f64, &IptvOrgLogoItem)> = None;
    for item in logos {
        let ch_lower = item.channel.to_lowercase();
        let base = ch_lower.split('.').next().unwrap_or(&ch_lower);
        let base_alphanumeric: String = base.chars().filter(|c| c.is_alphanumeric()).collect();
        let score = strsim::jaro_winkler(&slug_alphanumeric, &base_alphanumeric);
        if score >= 0.90 {
            if best.as_ref().map_or(true, |(b_score, _)| score > *b_score) {
                best = Some((score, item));
            }
        }
    }
    if let Some((_, item)) = best {
        let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };
        return Some(LogoSearchResult {
            url: item.url.clone(),
            thumb_url: item.url.clone(),
            source_name: "iptv-org (global fuzzy)".to_string(),
            format: fmt.to_string(),
            title: item.channel.clone(),
        });
    }

    None
}

#[allow(dead_code)]
fn match_iptv_org(channels: &[IptvOrgChannel], slug: &str, clean_name: &str) -> Option<LogoSearchResult> {
    for ch in channels {
        let ch_slug = slugify_channel_name(&ch.name);
        let mut matches = ch_slug == slug || ch.name.eq_ignore_ascii_case(clean_name);
        if !matches {
            for alt in &ch.alt_names {
                if slugify_channel_name(alt) == slug || alt.eq_ignore_ascii_case(clean_name) {
                    matches = true;
                    break;
                }
            }
        }
        if matches {
            if let Some(ref l) = ch.logo {
                let l_trimmed = l.trim();
                if !l_trimmed.is_empty() {
                    let fmt = if l_trimmed.ends_with(".svg") { "SVG" } else { "PNG" };
                    return Some(LogoSearchResult {
                        url: l_trimmed.to_string(),
                        thumb_url: l_trimmed.to_string(),
                        source_name: "iptv-org".to_string(),
                        format: fmt.to_string(),
                        title: ch.name.clone(),
                    });
                }
            }
        }
    }
    None
}


async fn search_wikimedia_commons(state: &AppState, clean_name: &str, country_name: &str) -> Vec<LogoSearchResult> {
    let client = &state.http;

    let query = if country_name.trim().is_empty() {
        format!("{} logo", clean_name.trim())
    } else {
        format!("{} {} logo", clean_name.trim(), country_name.trim())
    };

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
    if let Ok(r) = resp {
        if r.status().is_success() {
            if let Ok(text) = r.text().await {
                if let Ok(data) = serde_json::from_str::<MwQueryResponse>(&text) {
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
    let already_have = {
        let r = TV_LOGO_TREE_CACHE.read().unwrap();
        r.is_some()
    };
    if !already_have {
        if let Ok(paths) = fetch_tv_logo_full_tree(state).await {
            let mut w = TV_LOGO_TREE_CACHE.write().unwrap();
            *w = Some(paths);
        }
    }
    Ok(())
}

async fn ensure_iptv_logos_loaded(state: &AppState) -> Result<(), String> {
    let already_have = {
        let r = IPTV_LOGOS_MEM_CACHE.read().unwrap();
        r.is_some()
    };
    if !already_have {
        if let Ok(logos) = fetch_iptv_org_logos(state).await {
            let mut w = IPTV_LOGOS_MEM_CACHE.write().unwrap();
            *w = Some(logos);
        }
    }
    Ok(())
}

#[tauri::command]
async fn preload_logo_sources(state: tauri::State<'_, AppState>, _country_codes: Vec<String>) -> Result<(), String> {
    let _ = ensure_tv_logo_tree_loaded(state.inner()).await;
    let _ = ensure_iptv_logos_loaded(state.inner()).await;
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

    // 1. Forrás A: tv-logo full tree
    let _ = ensure_tv_logo_tree_loaded(state).await;
    {
        let r = TV_LOGO_TREE_CACHE.read().unwrap();
        if let Some(ref tree) = *r {
            if let Some(res) = match_tv_logo_from_tree(tree, country_dir, c_code, &slug) {
                candidate = Some((res.url, res.source_name));
            }
        }
    }

    // 2. Forrás B: iptv-org logos
    if candidate.is_none() {
        let _ = ensure_iptv_logos_loaded(state).await;
        let r = IPTV_LOGOS_MEM_CACHE.read().unwrap();
        if let Some(ref iptv_logos) = *r {
            if let Some(res) = match_iptv_org_logos(iptv_logos, &slug, c_code) {
                candidate = Some((res.url, "iptv-org".to_string()));
            } else if c_code != "INT" {
                if let Some(res) = match_iptv_org_logos(iptv_logos, &slug, "int") {
                    candidate = Some((res.url, "iptv-org (int)".to_string()));
                } else if let Some(res) = match_iptv_org_logos_any(iptv_logos, &slug) {
                    candidate = Some((res.url, "iptv-org (global)".to_string()));
                }
            } else if let Some(res) = match_iptv_org_logos_any(iptv_logos, &slug) {
                candidate = Some((res.url, "iptv-org (global)".to_string()));
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
    let wiki_results = search_wikimedia_commons(state, &clean, country_dir).await;
    for w in &wiki_results {
        let title_lower = w.title.to_lowercase();
        if title_lower.contains(&slug) || title_lower.contains(&clean.to_lowercase()) {
            candidate = Some((w.url.clone(), "Wikimedia Commons".to_string()));
            break;
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

async fn find_channel_logo_url_inner(state: &AppState, 
    source_id: String,
    channel_id: String,
    channel_name: String,
    country_code: String,
) -> Result<Option<MatchedChannelItem>, String> {
    if let Some(item) = find_channel_logo_github_inner(state, source_id.clone(), channel_id.clone(), channel_name.clone(), country_code.clone()).await? {
        return Ok(Some(item));
    }
    find_channel_logo_wikimedia_inner(state, source_id, channel_id, channel_name, country_code).await
}

async fn auto_match_channel_logo_inner(state: &AppState, 
    source_id: String,
    channel_id: String,
    channel_name: String,
    country_code: String,
) -> Result<Option<MatchedChannelItem>, String> {
    if let Some(mut item) = find_channel_logo_url_inner(state, source_id, channel_id, channel_name, country_code).await? {
        if let Ok(cached_data) = cache_logo_inner(state, item.logo_url.clone()).await {
            item.cached_data = Some(cached_data);
            return Ok(Some(item));
        }
    }
    Ok(None)
}

#[tauri::command]
async fn cache_logo(state: tauri::State<'_, AppState>, url: String) -> Result<String, String> {
    cache_logo_inner(state.inner(), url).await
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
async fn find_channel_logo_url(state: tauri::State<'_, AppState>, source_id: String, channel_id: String, channel_name: String, country_code: String) -> Result<Option<MatchedChannelItem>, String> {
    find_channel_logo_url_inner(state.inner(), source_id, channel_id, channel_name, country_code).await
}

#[tauri::command]
async fn auto_match_channel_logo(state: tauri::State<'_, AppState>, source_id: String, channel_id: String, channel_name: String, country_code: String) -> Result<Option<MatchedChannelItem>, String> {
    auto_match_channel_logo_inner(state.inner(), source_id, channel_id, channel_name, country_code).await
}

#[tauri::command]
async fn get_channel_logo_stats(state: tauri::State<'_, AppState>, source_id: Option<String>) -> Result<ChannelLogoStats, String> {
    let conn = state.db.lock().await;
    let query = if let Some(ref sid) = source_id {
        if sid.is_empty() || sid == "__ALL_SOURCES__" {
            "SELECT 
                COUNT(*) as total,
                SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END) as with_logo
             FROM channels c
             LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id".to_string()
        } else {
            format!(
                "SELECT 
                    COUNT(*) as total,
                    SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END) as with_logo
                 FROM channels c
                 LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
                 WHERE c.source_id = '{}'",
                sid.replace('\'', "''")
            )
        }
    } else {
        "SELECT 
            COUNT(*) as total,
            SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END) as with_logo
         FROM channels c
         LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id".to_string()
    };

    let (total, with_logo): (i64, i64) = conn.query_row(&query, [], |r| {
        let tot: i64 = r.get(0).unwrap_or(0);
        let wl: i64 = r.get(1).unwrap_or(0);
        Ok((tot, wl))
    }).unwrap_or((0, 0));

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

    // Ensure tv-logo tree is loaded
    let tv_tree = {
        let r = TV_LOGO_TREE_CACHE.read().unwrap();
        r.clone()
    };
    let tv_tree = match tv_tree {
        Some(t) => t,
        None => {
            let t = fetch_tv_logo_full_tree(state.inner()).await.unwrap_or_default();
            let mut w = TV_LOGO_TREE_CACHE.write().unwrap();
            *w = Some(t.clone());
            t
        }
    };

    // 1. Forrás A: tv-logo (from full tree: Country, International, Any country in tree)
    let tv_matches = search_tv_logo_from_tree(&tv_tree, country_dir, c_code, &slug, &clean);
    for m in tv_matches {
        if seen_urls.insert(m.url.clone()) {
            all_results.push(m);
        }
    }

    // 2. Forrás B: iptv-org (Logos database)
    let iptv_logos = {
        let r = IPTV_LOGOS_MEM_CACHE.read().unwrap();
        r.clone()
    };
    let iptv_logos = match iptv_logos {
        Some(l) => l,
        None => {
            let l = fetch_iptv_org_logos(state.inner()).await.unwrap_or_default();
            let mut w = IPTV_LOGOS_MEM_CACHE.write().unwrap();
            *w = Some(l.clone());
            l
        }
    };

    let suffix = format!(".{}", c_code.to_lowercase());
    let clean_lower = clean.to_lowercase();
    // Country matches first
    for item in &iptv_logos {
        let ch_lower = item.channel.to_lowercase();
        if ch_lower.ends_with(&suffix) {
            let base = ch_lower.trim_end_matches(&suffix);
            if (slug.len() >= 3 && base.contains(&slug)) || base.contains(&clean_lower) {
                if seen_urls.insert(item.url.clone()) {
                    let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };
                    all_results.push(LogoSearchResult {
                        url: item.url.clone(),
                        thumb_url: item.url.clone(),
                        source_name: "iptv-org".to_string(),
                        format: fmt.to_string(),
                        title: item.channel.clone(),
                    });
                }
            }
        }
    }
    // Then international or without language
    for item in &iptv_logos {
        let ch_lower = item.channel.to_lowercase();
        let base = ch_lower.split('.').next().unwrap_or(&ch_lower);
        if (slug.len() >= 3 && base.contains(&slug)) || base.contains(&clean_lower) {
            if seen_urls.insert(item.url.clone()) {
                let fmt = if item.url.ends_with(".svg") { "SVG" } else { "PNG" };
                all_results.push(LogoSearchResult {
                    url: item.url.clone(),
                    thumb_url: item.url.clone(),
                    source_name: "iptv-org".to_string(),
                    format: fmt.to_string(),
                    title: item.channel.clone(),
                });
            }
        }
    }

    // 3. Forrás C: Wikimedia Commons (With Country)
    let wiki_matches = search_wikimedia_commons(state.inner(), &clean, country_dir).await;
    for m in wiki_matches {
        if seen_urls.insert(m.url.clone()) {
            all_results.push(m);
        }
    }

    // 3b. Forrás C: Wikimedia Commons (Without Country / Global)
    let wiki_global = search_wikimedia_commons(state.inner(), &clean, "").await;
    for m in wiki_global {
        if seen_urls.insert(m.url.clone()) {
            all_results.push(m);
        }
    }

    // Fuzzy rendezés: a keresési kifejezéshez legjobban illeszkedő logók kerüljenek előre
    let slug_cmp = slug.clone();
    all_results.sort_by(|a, b| {
        let slug_a = slugify_channel_name(&clean_channel_name(&a.title));
        let slug_b = slugify_channel_name(&clean_channel_name(&b.title));
        let score_a = strsim::jaro_winkler(&slug_cmp, &slug_a);
        let score_b = strsim::jaro_winkler(&slug_cmp, &slug_b);
        score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(all_results)
}

#[tauri::command]
async fn auto_match_source_logos(state: tauri::State<'_, AppState>, source_id: Option<String>) -> Result<AutoMatchSummary, String> {
    let filter_source = match source_id.as_deref() {
        Some(s) if !s.is_empty() && s != "__ALL_SOURCES__" => Some(s.to_string()),
        _ => None,
    };

    let (total_channels, with_logo_count): (i64, i64) = {
        let conn = state.db.lock().await;
        if let Some(ref sid) = filter_source {
            conn.query_row(
                "SELECT COUNT(*),
                        SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END)
                 FROM channels c
                 LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
                 WHERE c.source_id = ?1",
                params![sid],
                |r| Ok((r.get(0).unwrap_or(0), r.get(1).unwrap_or(0))),
            ).unwrap_or((0, 0))
        } else {
            conn.query_row(
                "SELECT COUNT(*),
                        SUM(CASE WHEN (cus.custom_icon IS NOT NULL AND cus.custom_icon != '') OR (c.icon IS NOT NULL AND c.icon != '') THEN 1 ELSE 0 END)
                 FROM channels c
                 LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id",
                [],
                |r| Ok((r.get(0).unwrap_or(0), r.get(1).unwrap_or(0))),
            ).unwrap_or((0, 0))
        }
    };

    let app_state = state.inner();
    let missing_channels = get_missing_logo_channels_inner(app_state, source_id).await?;
    let missing_count = missing_channels.len();
    if missing_count == 0 {
        return Ok(AutoMatchSummary {
            total_channels: total_channels as usize,
            with_logo_count: with_logo_count as usize,
            missing_logos: 0,
            matched_count: 0,
            failed_count: 0,
            skipped_existing: with_logo_count as usize,
            matches: Vec::new(),
        });
    }

    let mut matches = Vec::new();
    let mut failed_count = 0;

    for ch in &missing_channels {
        if let Ok(Some(item)) = auto_match_channel_logo_inner(app_state, ch.source_id.clone(), ch.id.clone(), ch.name.clone(), ch.country_code.clone()).await {
            matches.push(item);
        } else {
            failed_count += 1;
        }
    }

    Ok(AutoMatchSummary {
        total_channels: total_channels as usize,
        with_logo_count: with_logo_count as usize,
        missing_logos: missing_count,
        matched_count: matches.len(),
        failed_count,
        skipped_existing: with_logo_count as usize,
        matches,
    })
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ChannelExportRow {
    source_id: String,
    source_name: String,
    source_url: String,
    channel_id: String,
    channel_name: String,
    is_favorite: i32,
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
    for r in rows {
        if let Ok(row) = r { channels.push(row); }
    }

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

#[tauri::command]
async fn export_settings(state: tauri::State<'_, AppState>) -> Result<BackupData, String> {
    let conn = state.db.lock().await;

    // Sources
    let mut stmt = conn.prepare(
        "SELECT s.id, s.name, s.url, s.last_downloaded, s.country_code, s.time_offset,
                (SELECT COUNT(*) FROM channels c WHERE c.source_id = s.id) AS ch_count,
                (SELECT COUNT(*) FROM channel_user_settings cus WHERE cus.source_id = s.id AND cus.is_favorite = 1) AS fav_count,
                (SELECT COUNT(*) FROM channel_user_settings cus WHERE cus.source_id = s.id AND cus.is_hidden = 1) AS hid_count
         FROM sources s
         ORDER BY s.rowid ASC"
    ).map_err(|e| e.to_string())?;
    let sources_iter = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            last_downloaded: row.get(3)?,
            country_code: row.get(4)?,
            time_offset: row.get(5)?,
            channel_count: Some(row.get(6)?),
            favorite_count: Some(row.get(7)?),
            hidden_count: Some(row.get(8)?),
        })
    }).map_err(|e| e.to_string())?;
    let mut sources = Vec::new();
    for s in sources_iter {
        if let Ok(src) = s { sources.push(src); }
    }

    // Favorites
    let mut stmt = conn.prepare("SELECT title FROM favorites ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let fav_iter = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut favorites = Vec::new();
    for f in fav_iter {
        if let Ok(title) = f { favorites.push(title); }
    }

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
    for cus in cus_iter {
        if let Ok(c) = cus { channel_user_settings.push(c); }
    }

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
    for co in co_iter {
        if let Ok(c) = co { channel_orders.push(c); }
    }

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
    for csp in csp_iter {
        if let Ok(p) = csp { channel_sort_presets.push(p); }
    }

    // Settings
    let mut stmt = conn.prepare("SELECT key, value FROM settings").map_err(|e| e.to_string())?;
    let s_iter = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).map_err(|e| e.to_string())?;
    let mut settings = HashMap::new();
    for item in s_iter {
        if let Ok((k, v)) = item { settings.insert(k, v); }
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
async fn import_settings(state: tauri::State<'_, AppState>, backup: BackupData) -> Result<(), String> {
    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    // Sources
    for s in backup.sources {
        tx.execute(
            "INSERT INTO sources (id, name, url, last_downloaded, country_code, time_offset) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET 
                name = excluded.name, 
                url = excluded.url, 
                last_downloaded = COALESCE(excluded.last_downloaded, sources.last_downloaded),
                country_code = excluded.country_code,
                time_offset = excluded.time_offset",
            params![s.id, s.name, s.url, s.last_downloaded, s.country_code, s.time_offset],
        ).map_err(|e| e.to_string())?;
    }

    // Favorites
    tx.execute("DELETE FROM favorites", []).ok();
    for fav in backup.favorites {
        tx.execute(
            "INSERT OR REPLACE INTO favorites (title) VALUES (?1)",
            params![fav],
        ).ok();
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
        ).ok();
    }

    // Channel orders
    for co in backup.channel_orders {
        tx.execute(
            "INSERT INTO channel_orders (source_id, channel_id, position)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(source_id, channel_id) DO UPDATE SET
                position = excluded.position",
            params![co.source_id, co.channel_id, co.position],
        ).ok();
    }

    // Channel sort presets
    if !backup.channel_sort_presets.is_empty() {
        tx.execute("DELETE FROM channel_sort_presets", []).ok();
        for csp in backup.channel_sort_presets {
            let order_json = serde_json::to_string(&csp.channel_order).unwrap_or_else(|_| "[]".to_string());
            tx.execute(
                "INSERT INTO channel_sort_presets (id, name, source_id, channel_order, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![csp.id, csp.name, csp.source_id, order_json, csp.created_at],
            ).ok();
        }
    }

    // Settings
    for (k, v) in backup.settings {
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![k, v],
        ).ok();
    }

    tx.commit().map_err(|e| format!("Import mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let conn = state.db.lock().await;
    let mut stmt = conn.prepare("SELECT key, value FROM settings").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }).map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    for r in rows {
        if let Ok((k, v)) = r { map.insert(k, v); }
    }
    Ok(map)
}

#[tauri::command]
async fn set_setting(state: tauri::State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_language_content(state: tauri::State<'_, AppState>, lang: String) -> Result<String, String> {
    let conn = state.db.lock().await;
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

#[tauri::command]
async fn restore_languages_to_disk(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().await;

    conn.execute(
        "INSERT INTO app_languages (lang, content, updated_at) VALUES ('english', ?1, datetime('now', 'localtime'))
         ON CONFLICT(lang) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
        params![EMBEDDED_ENGLISH_LNG],
    ).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO app_languages (lang, content, updated_at) VALUES ('hungarian', ?1, datetime('now', 'localtime'))
         ON CONFLICT(lang) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
        params![EMBEDDED_HUNGARIAN_LNG],
    ).map_err(|e| e.to_string())?;

    let mut candidate_dirs = Vec::new();
    candidate_dirs.push(PathBuf::from("public/languages"));
    candidate_dirs.push(PathBuf::from("../public/languages"));
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            candidate_dirs.push(parent.join("public/languages"));
            candidate_dirs.push(parent.join("languages"));
        }
    }

    for dir in candidate_dirs {
        if dir.exists() || dir.parent().map(|p| p.exists()).unwrap_or(false) {
            let _ = fs::create_dir_all(&dir);
            let _ = fs::write(dir.join("english.lng"), EMBEDDED_ENGLISH_LNG);
            let _ = fs::write(dir.join("hungarian.lng"), EMBEDDED_HUNGARIAN_LNG);
        }
    }

    Ok(())
}

#[tauri::command]
async fn download_languages_from_github(state: tauri::State<'_, AppState>) -> Result<HashMap<String, String>, String> {
    let langs = ["english", "hungarian"];
    let mut result = HashMap::new();
    let conn = state.db.lock().await;

    for lang in &langs {
        let url = format!(
            "https://raw.githubusercontent.com/norti/EPG/main/public/languages/{}.lng",
            lang
        );
        let resp = state.http.get(&url).send().await.map_err(|e| format!("Letöltési hiba ({url}): {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("GitHub letöltés sikertelen ({url}): HTTP {}", resp.status()));
        }
        let text = resp.text().await.map_err(|e| format!("Válasz olvasási hiba ({url}): {e}"))?;
        let _: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("Érvénytelen JSON ({lang}.lng): {e}"))?;

        conn.execute(
            "INSERT INTO app_languages (lang, content, updated_at) VALUES (?1, ?2, datetime('now', 'localtime'))
             ON CONFLICT(lang) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
            params![lang, text],
        ).map_err(|e| e.to_string())?;

        let _ = fs::create_dir_all("public/languages");
        let _ = fs::write(format!("public/languages/{}.lng", lang), &text);

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

fn clean_title(title: &str) -> String {
    let mut s = title.replace('\u{a0}', " ").trim().to_string();

    // 1. Remove year in parentheses / brackets e.g. (1999), [2021]
    if let Ok(re_year) = regex::Regex::new(r"(?i)\s*[\(\[\{]\s*(?:19|20)\d{2}\s*[\)\]\}]") {
        s = re_year.replace_all(&s, "").to_string();
    }

    // 2. Remove rating and tech tags e.g. (12), (16), (18), (HD), (élő), (ism.), etc.
    if let Ok(re_tags) = regex::Regex::new(r"(?i)\s*[\(\[\{]\s*(?:12|16|18|BA|KN|HD|UHD|FHD|4K|3D|élő|live|ism\.?|premier|új|uj|feliratos|szinkronizált|magyarul beszélő[^)]*)\s*[\)\]\}]") {
        s = re_tags.replace_all(&s, "").to_string();
    }

    // 3. Remove written-out Hungarian season names e.g. " - Második évad"
    if let Ok(re_hu_season) = regex::Regex::new(r"(?i)\s*[-–—:]?\s*(?:első|második|harmadik|negyedik|ötödik|hatodik|hetedik|nyolcadik|kilencedik|tizedik)\s+évad.*$") {
        let trimmed_cand = re_hu_season.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 4. Remove episode / season patterns with delimiter or explicit keyword:
    // e.g. " - 1. rész", " 2. évad", " - S01E02", " - Ep. 5", " 1. epizód"
    if let Ok(re_ep1) = regex::Regex::new(r"(?i)\s*[-–—:]\s*(?:s\d+(?:e\d+)?|\d+[\.:]?\s*(?:rész|évad|epizód)|season\s*\d+|episode\s*\d+|ep\.\s*\d+).*$") {
        let trimmed_cand = re_ep1.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    if let Ok(re_ep2) = regex::Regex::new(r"(?i)\s+(?:s\d+e\d+|\d+[\.:]?\s*(?:rész|évad|epizód)|season\s*\d+|episode\s*\d+|ep\.\s*\d+).*$") {
        let trimmed_cand = re_ep2.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 5. Roman numeral season/part WITH explicit keyword only (e.g. " - I. évad", " II. rész")
    // Requires word boundary \b and explicit keyword to prevent matching words starting with Roman letters (like "Das...", "Megtorlás")
    if let Ok(re_roman_part) = regex::Regex::new(r"(?i)\s*(?:[-–—:]\s*)?\b(?:[IVXLCDM]+)\.?\s*(?:rész|évad|epizód).*$") {
        let trimmed_cand = re_roman_part.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 6. Season / episode slash numbering e.g. " I./3.", " I/3.", " - I./3.", " 1./3.", " 1/3", " II/4. rész"
    if let Ok(re_slash) = regex::Regex::new(r"(?i)\s*(?:[-–—:]\s*)?\b(?:[IVXLCDM]+|\d+)\.?\s*/\s*\d+[\.:]?(?:\s*(?:rész|évad|epizód))?.*$") {
        let trimmed_cand = re_slash.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 7. Remove trailing generic parenthesis/bracket tag if it still exists at the end
    if let Ok(re_tail_paren) = regex::Regex::new(r"\s*[\(\[\{][^\(\)\[\]\{\}]*[\)\]\}]\s*$") {
        let trimmed_cand = re_tail_paren.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 8. Remove trailing punctuation: dashes, colons, dots, slashes, commas
    s = s.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | ':' | ',' | '.' | '/' | '\\')).to_string();

    // 9. Collapse multiple spaces
    if let Ok(re_spaces) = regex::Regex::new(r"\s+") {
        s = re_spaces.replace_all(&s, " ").to_string();
    }

    s = s.trim().to_string();
    if s.len() < 2 {
        title.trim().to_string()
    } else {
        s
    }
}

// Extract a 4-digit year from parentheses/brackets in a raw EPG title, e.g. "Film (2018)" → Some(2018).
// Called BEFORE clean_title() so the year is not yet stripped.
fn extract_year_hint(raw_title: &str) -> Option<u32> {
    regex::Regex::new(r"[\(\[]\s*((?:19|20)\d{2})\s*[\)\]]")
        .ok()?
        .captures(raw_title)?
        .get(1)?
        .as_str()
        .parse()
        .ok()
}

// Lowercase + keep only alphanumeric chars and spaces, then collapse whitespace.
fn normalise_for_match(s: &str) -> String {
    let mapped: String = s
        .chars()
        .flat_map(|c| {
            if c.is_alphanumeric() {
                // to_lowercase() handles Unicode (ÁáÉéŐő etc.) correctly
                c.to_lowercase().collect::<String>().chars().collect::<Vec<_>>()
            } else {
                vec![' ']
            }
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

// Score how well a TMDB candidate title matches the cleaned query.
// Returns a value in roughly [0.0, 1.5]; higher is better.
fn tmdb_title_score(
    query: &str,
    cand_title: &str,
    cand_orig: Option<&str>,
    year_hint: Option<u32>,
    cand_year: Option<u32>,
) -> f64 {
    let nq = normalise_for_match(query);
    let nc = normalise_for_match(cand_title);
    let no = cand_orig.map(|o| normalise_for_match(o));

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

#[tauri::command]
async fn get_tmdb_api_key(state: tauri::State<'_, AppState>) -> Result<Option<String>, String> {
    let conn = state.db.lock().await;
    let res: Option<String> = conn.query_row(
        "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
        [],
        |row| row.get(0),
    ).ok();
    Ok(res)
}

#[tauri::command]
async fn set_tmdb_api_key(state: tauri::State<'_, AppState>, api_key: String) -> Result<(), String> {
    let conn = state.db.lock().await;
    let clean_key = api_key.trim();
    if clean_key.is_empty() {
        conn.execute("DELETE FROM settings WHERE key = 'tmdb_api_key'", []).map_err(|e| e.to_string())?;
    } else {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('tmdb_api_key', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![clean_key],
        ).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn clear_tmdb_cache(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let conn = state.db.lock().await;
    conn.execute("DELETE FROM tmdb_cache", []).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn test_tmdb_api_key(state: tauri::State<'_, AppState>, api_key: String) -> Result<bool, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(false);
    }
    let is_v4 = key.starts_with("eyJ") || key.len() > 40;
    let req = if is_v4 {
        state.http.get("https://api.themoviedb.org/3/authentication")
            .header("Authorization", format!("Bearer {}", key))
            .header("Accept", "application/json")
    } else {
        state.http.get("https://api.themoviedb.org/3/authentication")
            .query(&[("api_key", key)])
            .header("Accept", "application/json")
    };

    match req.send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(e) => Err(format!("Hálózati hiba: {}", e)),
    }
}

#[tauri::command]
async fn get_tmdb_details(state: tauri::State<'_, AppState>, title: String, lang: Option<String>) -> Result<Option<TmdbDetails>, String> {
    // Extract year hint from raw title BEFORE cleaning strips it (used for scoring later)
    let year_hint = extract_year_hint(&title);
    let clean = clean_title(&title);
    if clean.is_empty() {
        return Ok(None);
    }

    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());

    // 1. Check local cache
    let cached = {
        let conn = state.db.lock().await;
        conn.query_row(
        "SELECT tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date
         FROM tmdb_cache WHERE query_title = ?1 AND lang = ?2",
        params![clean, target_lang],
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
            Ok((tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date))
        }
    ).ok()
    };

    if let Some((tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date)) = cached {
        if media_type.as_deref() == Some("none") || tmdb_id.is_none() {
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

    // 2. Retrieve TMDB API key from settings
    let api_key: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
            [],
            |row| row.get(0),
        ).ok()
    };

    let api_key = match api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return Ok(None),
    };

    let is_v4 = api_key.starts_with("eyJ") || api_key.len() > 40;

    let execute_search = |q: &str, l: &str| {
        let mut req = state.http.get("https://api.themoviedb.org/3/search/multi")
            .header("Accept", "application/json")
            .query(&[("query", q), ("language", l), ("include_adult", "false")]);
        if is_v4 {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        } else {
            req = req.query(&[("api_key", api_key.as_str())]);
        }
        req
    };

    let resp = execute_search(&clean, &target_lang).send().await;
    let resp = match resp {
        Ok(r) => {
            if r.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err("Érvénytelen TMDB API kulcs / Invalid TMDB API key".to_string());
            }
            r
        }
        Err(e) => return Err(format!("TMDB keresési hiba: {}", e)),
    };

    let text = resp.text().await.map_err(|e| e.to_string())?;
    let search_json: Result<TmdbSearchResponse, _> = serde_json::from_str(&text);
    let search_res = search_json.ok().and_then(|r| r.results).unwrap_or_default();

    // Pick best-matching movie/tv candidate instead of blindly taking the first result
    let mut matched_item = best_tmdb_match(search_res, &clean, year_hint);

    if matched_item.is_none() && target_lang != "en-US" {
        if let Ok(fallback_resp) = execute_search(&clean, "en-US").send().await {
            if let Ok(fallback_text) = fallback_resp.text().await {
                if let Ok(fallback_json) = serde_json::from_str::<TmdbSearchResponse>(&fallback_text) {
                    matched_item = best_tmdb_match(fallback_json.results.unwrap_or_default(), &clean, year_hint);
                }
            }
        }
    }

    let item = match matched_item {
        Some(it) => it,
        None => {
            let conn = state.db.lock().await;
            conn.execute(
                "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, updated_at) VALUES (?1, ?2, 'none', datetime('now', 'localtime'))",
                params![clean, target_lang],
            ).ok();
            return Ok(None);
        }
    };

        let details = fetch_tmdb_item_details(
        state.inner(),
        item,
        &clean,
        &target_lang,
        &api_key,
    ).await?;

    Ok(Some(details))
}

async fn fetch_tmdb_item_details(
    state: &AppState,
    item: TmdbRawItem,
    clean: &str,
    target_lang: &str,
    api_key: &str,
) -> Result<TmdbDetails, String> {
    let is_v4 = api_key.starts_with("eyJ") || api_key.len() > 40;
    let tmdb_id = item.id;
    let media_type = item.media_type.unwrap_or_else(|| "movie".to_string());
    let display_title = item.title.clone().or_else(|| item.name.clone()).unwrap_or_else(|| clean.to_string());
    let orig_title = item.original_title.clone().or_else(|| item.original_name.clone());
    let rel_date = item.release_date.clone().or_else(|| item.first_air_date.clone());

    // Translations request to find hu_title and en_title
    let mut hu_title: Option<String> = None;
    let mut en_title: Option<String> = None;
    let mut hu_overview: Option<String> = None;

    let trans_url = format!("https://api.themoviedb.org/3/{}/{}/translations", media_type, tmdb_id);
    let mut trans_req = state.http.get(&trans_url).header("Accept", "application/json");
    if is_v4 {
        trans_req = trans_req.header("Authorization", format!("Bearer {}", api_key));
    } else {
        trans_req = trans_req.query(&[("api_key", api_key)]);
    }

    if let Ok(trans_resp) = trans_req.send().await {
        if let Ok(trans_text) = trans_resp.text().await {
            if let Ok(trans_data) = serde_json::from_str::<TmdbTranslationsResponse>(&trans_text) {
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
                            }
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

    let final_overview = if target_lang.starts_with("hu") && hu_overview.is_some() {
        hu_overview
    } else {
        item.overview.or(hu_overview)
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

    // Save to cache
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT OR REPLACE INTO tmdb_cache (
            query_title, lang, tmdb_id, media_type, title, hu_title, en_title, original_title,
            original_language, poster_path, backdrop_path, overview, vote_average, release_date, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now', 'localtime'))",
        params![
            clean,
            target_lang,
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
        ],
    ).map_err(|e| e.to_string())?;

    Ok(details)
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

    // Check cache first (keyed by query_title + lang)
    let cached = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT tmdb_id, media_type, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date
             FROM tmdb_cache WHERE query_title = ?1 AND lang = ?2",
            params![cache_key, lang],
            |row| {
                let cached_id: Option<i64> = row.get(0)?;
                let media: Option<String> = row.get(1)?;
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
                Ok((cached_id, media, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date))
            }
        ).ok()
    };

    if let Some((Some(cached_id), media, title, hu_title, en_title, original_title, original_language, poster_path, backdrop_path, overview, vote_average, release_date)) = cached {
        if media.as_deref() != Some("none") {
            return Ok(TmdbDetails {
                tmdb_id: cached_id,
                title: title.unwrap_or_else(|| cache_key.clone()),
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

    // Not in cache — fetch from TMDB by ID
    let api_key: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
            [],
            |row| row.get(0),
        ).ok()
    };

    let api_key = match api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return Err("TMDB API kulcs nincs beállítva / TMDB API key is not configured".to_string()),
    };

    let is_v4 = api_key.starts_with("eyJ") || api_key.len() > 40;

    let details_url = format!("https://api.themoviedb.org/3/{}/{}", media_type, tmdb_id);
    let mut details_req = state.http.get(&details_url)
        .header("Accept", "application/json")
        .query(&[("language", lang.as_str())]);
    if is_v4 {
        details_req = details_req.header("Authorization", format!("Bearer {}", api_key));
    } else {
        details_req = details_req.query(&[("api_key", api_key.as_str())]);
    }

    let resp = details_req.send().await.map_err(|e| format!("TMDB lekérési hiba: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("TMDB hiba ({}): {}", resp.status(), details_url));
    }

    let text = resp.text().await.map_err(|e| e.to_string())?;
    let mut raw_item: TmdbRawItem = serde_json::from_str(&text).map_err(|e| format!("JSON hiba: {}", e))?;
    raw_item.media_type = Some(media_type);

    fetch_tmdb_item_details(state.inner(), raw_item, &cache_key, &lang, &api_key).await
}

#[tauri::command]
async fn search_tmdb(state: tauri::State<'_, AppState>, query: String, lang: Option<String>) -> Result<Vec<TmdbSearchResultItem>, String> {
    let q = query.trim().to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }

    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());

    let api_key: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
            [],
            |row| row.get(0),
        ).ok()
    };

    let api_key = match api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return Err("TMDB API kulcs nincs beállítva / TMDB API key is not configured".to_string()),
    };

    let is_v4 = api_key.starts_with("eyJ") || api_key.len() > 40;

    let execute_search = |term: &str, l: &str| {
        let mut req = state.http.get("https://api.themoviedb.org/3/search/multi")
            .header("Accept", "application/json")
            .query(&[("query", term), ("language", l), ("include_adult", "false")]);
        if is_v4 {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        } else {
            req = req.query(&[("api_key", api_key.as_str())]);
        }
        req
    };

    let mut results: Vec<TmdbRawItem> = Vec::new();
    let resp = execute_search(&q, &target_lang).send().await.map_err(|e| format!("TMDB keresési hiba: {}", e))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Érvénytelen TMDB API kulcs / Invalid TMDB API key".to_string());
    }
    if resp.status().is_success() {
        if let Ok(text) = resp.text().await {
            if let Ok(search_json) = serde_json::from_str::<TmdbSearchResponse>(&text) {
                results = search_json.results.unwrap_or_default();
            }
        }
    }

    // If no results and clean_title(q) differs from q, try searching with clean_title
    let cleaned = clean_title(&q);
    if results.is_empty() && !cleaned.is_empty() && cleaned != q {
        if let Ok(resp2) = execute_search(&cleaned, &target_lang).send().await {
            if resp2.status().is_success() {
                if let Ok(text2) = resp2.text().await {
                    if let Ok(search_json2) = serde_json::from_str::<TmdbSearchResponse>(&text2) {
                        results = search_json2.results.unwrap_or_default();
                    }
                }
            }
        }
    }

    // Fallback to en-US if still empty and language was not en-US
    if results.is_empty() && target_lang != "en-US" {
        let query_term = if !cleaned.is_empty() { &cleaned } else { &q };
        if let Ok(resp3) = execute_search(query_term, "en-US").send().await {
            if resp3.status().is_success() {
                if let Ok(text3) = resp3.text().await {
                    if let Ok(search_json3) = serde_json::from_str::<TmdbSearchResponse>(&text3) {
                        results = search_json3.results.unwrap_or_default();
                    }
                }
            }
        }
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
async fn set_tmdb_match(state: tauri::State<'_, AppState>, 
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

    let api_key: Option<String> = {
        let conn = state.db.lock().await;
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
            [],
            |row| row.get(0),
        ).ok()
    };

    let api_key = match api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return Err("TMDB API kulcs nincs beállítva / TMDB API key is not configured".to_string()),
    };

    let is_v4 = api_key.starts_with("eyJ") || api_key.len() > 40;

    let details_url = format!("https://api.themoviedb.org/3/{}/{}", media_type, tmdb_id);
    let mut details_req = state.http.get(&details_url)
        .header("Accept", "application/json")
        .query(&[("language", target_lang.as_str())]);
    if is_v4 {
        details_req = details_req.header("Authorization", format!("Bearer {}", api_key));
    } else {
        details_req = details_req.query(&[("api_key", api_key.as_str())]);
    }

    let resp = details_req.send().await.map_err(|e| format!("TMDB lekérési hiba: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("TMDB hiba ({}): {}", resp.status(), details_url));
    }

    let text = resp.text().await.map_err(|e| e.to_string())?;
    let mut raw_item: TmdbRawItem = serde_json::from_str(&text).map_err(|e| format!("JSON hiba: {}", e))?;
    raw_item.media_type = Some(media_type);

    let details = fetch_tmdb_item_details(
        state.inner(),
        raw_item,
        &cache_key,
        &target_lang,
        &api_key,
    ).await?;

    Ok(details)
}

#[tauri::command]
async fn clear_tmdb_match(state: tauri::State<'_, AppState>, query_title: String, lang: Option<String>) -> Result<(), String> {
    let clean = clean_title(&query_title);
    let cache_key = if clean.is_empty() { query_title.trim().to_string() } else { clean };
    if cache_key.is_empty() {
        return Ok(());
    }
    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());
    let conn = state.db.lock().await;
    conn.execute(
        "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, updated_at) VALUES (?1, ?2, 'none', datetime('now', 'localtime'))",
        params![cache_key, target_lang],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn update_source_epg(state: tauri::State<'_, AppState>, source_id: String, url: String) -> Result<(), String> {
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
        let rewritten = if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
            let parts: Vec<&str> = rest.splitn(4, '/').collect();
            // parts: [owner, repo, "raw", "refs/heads/branch/path" or "branch/path"]
            if parts.len() == 4 && parts[2] == "raw" {
                format!("https://raw.githubusercontent.com/{}/{}/{}", parts[0], parts[1], parts[3])
            } else {
                trimmed.to_string()
            }
        } else {
            trimmed.to_string()
        };
        rewritten
    };

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/159.0.0.0 Safari/537.36")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(60))
        .no_gzip()
        .build()
        .map_err(|e| format!("Kliens hiba: {}", e))?;

    let response = client.get(&url).send().await.map_err(|e| format!("Hálózati hiba: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("A szerver hibakódot adott vissza: {}", response.status()));
    }

    let bytes = response.bytes().await.map_err(|e| format!("Adatolvasási hiba: {}", e))?;

    let xml_string = if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut s = String::new();
        decoder.read_to_string(&mut s).map_err(|e| format!("Kitömörítési hiba: {}", e))?;
        s
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|e| format!("UTF8 konverziós hiba: {}", e))?
    };

    let mut conn = state.db.lock().await;
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM channels WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM programmes WHERE source_id = ?1", params![source_id]).ok();

    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_str(&xml_string);
    reader.trim_text(true);

    let mut buf = Vec::new();
    let mut current_tag = String::new();
    let mut curr_ch_id = String::new();
    let mut curr_ch_name = String::new();
    let mut curr_ch_icon = String::new();

    let mut curr_prog_ch = String::new();
    let mut curr_prog_start = String::new();
    let mut curr_prog_stop = String::new();
    let mut curr_prog_title = String::new();
    let mut curr_prog_desc = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                current_tag = name.clone();

                if name == "channel" {
                    curr_ch_id.clear(); curr_ch_name.clear(); curr_ch_icon.clear();
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"id" {
                            curr_ch_id = String::from_utf8_lossy(&attr.value).to_string();
                        }
                    }
                } else if name == "programme" {
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
                } else if name == "icon" {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"src" {
                            curr_ch_icon = String::from_utf8_lossy(&attr.value).to_string();
                        }
                    }
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                if current_tag == "display-name" {
                    curr_ch_name = text;
                } else if current_tag == "title" {
                    curr_prog_title = text;
                } else if current_tag == "desc" {
                    curr_prog_desc = text;
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if name == "channel" {
                    tx.execute(
                        "INSERT OR REPLACE INTO channels (id, source_id, name, icon) VALUES (?1, ?2, ?3, ?4)",
                        params![curr_ch_id, source_id, curr_ch_name, curr_ch_icon],
                    ).ok();
                } else if name == "programme" {
                    tx.execute(
                        "INSERT INTO programmes (channel_id, source_id, start, stop, title, desc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![curr_prog_ch, source_id, curr_prog_start, curr_prog_stop, curr_prog_title, curr_prog_desc],
                    ).ok();
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => (),
        }
        buf.clear();
    }

    tx.execute(
        "UPDATE sources SET last_downloaded = datetime('now', 'localtime') WHERE id = ?1",
        params![source_id],
    ).ok();

    tx.commit().map_err(|e| format!("Mentési hiba: {}", e))?;
    Ok(())
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
        let dwmapi = LoadLibraryA(b"dwmapi.dll\0".as_ptr());
        if !dwmapi.is_null() {
            let func = GetProcAddress(dwmapi, b"DwmSetWindowAttribute\0".as_ptr());
            if let Some(set_attr) = std::mem::transmute::<_, Option<DwmSetWindowAttributeFn>>(func) {
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

#[tauri::command]
fn open_url(url: String) {
    let _ = open::that(url);
}

fn main() {
    let http_client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) EPGViewer/1.0")
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .expect("HTTP kliens létrehozási hiba");

    tauri::Builder::default()
        .manage(AppState {
            db: Mutex::new(open_and_init_db()),
            http: http_client,
        })
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                for window in app.webview_windows().values() {
                    if let Ok(hwnd) = window.hwnd() {
                        apply_windows_dark_titlebar(hwnd.0 as isize);
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_epg_data,
            get_sources,
            save_source,
            delete_source,
            save_source_order,
            clear_entire_database,
            update_source_epg,
            get_favorites,
            toggle_favorite,
            set_favorites,
            get_channel_order,
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
            get_channel_logo_stats,
            get_missing_logo_channels,
            preload_logo_sources,
            find_channel_logo_github,
            find_channel_logo_wikimedia,
            find_channel_logo_url,
            auto_match_channel_logo,
            auto_match_source_logos,
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
            restore_languages_to_disk,
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
            open_url
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
        assert_eq!(map_country_code("HU"), ("hungary", "hu"));
        assert_eq!(map_country_code("de"), ("germany", "de"));
        assert_eq!(map_country_code("at"), ("austria", "at"));
        assert_eq!(map_country_code("US"), ("united-states", "us"));
        assert_eq!(map_country_code("UK"), ("united-kingdom", "uk"));
    }

    #[test]
    fn test_match_tv_logo() {
        let items = vec![
            TvLogoFileItem {
                name: "cool-tv-hu.png".to_string(),
                download_url: None,
            },
            TvLogoFileItem {
                name: "arena4-hu.png".to_string(),
                download_url: None,
            },
            TvLogoFileItem {
                name: "rtl-hu.png".to_string(),
                download_url: None,
            },
        ];

        let res = match_tv_logo(&items, "hungary", "hu", "cool-tv");
        assert!(res.is_some());
        let logo = res.unwrap();
        assert_eq!(logo.source_name, "tv-logo");
        assert_eq!(logo.format, "PNG");
        assert!(logo.url.contains("cool-tv-hu.png"));

        let res2 = match_tv_logo(&items, "hungary", "hu", "unknown-channel");
        assert!(res2.is_none());
    }

    #[test]
    fn test_match_iptv_org() {
        let channels = vec![
            IptvOrgChannel {
                id: "MoziPlus.hu".to_string(),
                name: "Mozi+".to_string(),
                alt_names: vec!["Mozi Plusz".to_string()],
                country: Some("HU".to_string()),
                logo: Some("https://example.com/mozi-plus.png".to_string()),
            },
        ];

        let res = match_iptv_org(&channels, "mozi-plus", "Mozi+");
        assert!(res.is_some());
        let logo = res.unwrap();
        assert_eq!(logo.source_name, "iptv-org");
        assert_eq!(logo.url, "https://example.com/mozi-plus.png");

        let res_alt = match_iptv_org(&channels, "mozi-plusz", "Mozi Plusz");
        assert!(res_alt.is_some());
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
        ];

        // 1. Country match
        let res1 = match_tv_logo_from_tree(&paths, "hungary", "hu", "cool-tv");
        assert!(res1.is_some());
        assert_eq!(res1.unwrap().source_name, "tv-logo");

        // 2. International fallback
        let res2 = match_tv_logo_from_tree(&paths, "hungary", "hu", "animal-planet");
        assert!(res2.is_some());
        assert_eq!(res2.unwrap().source_name, "tv-logo (int)");

        // 3. Global in any country fallback
        let res3 = match_tv_logo_from_tree(&paths, "hungary", "hu", "eurosport-1");
        assert!(res3.is_some());
        assert_eq!(res3.unwrap().source_name, "tv-logo (global)");

        // 4. Fuzzy matching test
        let res_fuzzy = match_tv_logo_from_tree(&paths, "hungary", "hu", "cool-tv-hd");
        assert!(res_fuzzy.is_some());
        assert_eq!(res_fuzzy.unwrap().source_name, "tv-logo");
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
}

