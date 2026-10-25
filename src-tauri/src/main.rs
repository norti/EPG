#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use flate2::read::GzDecoder;

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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Source {
    id: String,
    name: String,
    url: String,
    last_downloaded: Option<String>,
    country_code: Option<String>,
    time_offset: Option<i32>,
}

fn get_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("epg-viewer");
    fs::create_dir_all(&path).ok();
    path.push("epg.db");
    path
}

fn init_db() -> Connection {
    let conn = Connection::open(get_db_path()).expect("DB megnyitási hiba");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS channels (
            id TEXT, source_id TEXT, name TEXT, icon TEXT, PRIMARY KEY (id, source_id)
        );
        CREATE TABLE IF NOT EXISTS programmes (
            id INTEGER PRIMARY KEY AUTOINCREMENT, channel_id TEXT, source_id TEXT, start TEXT, stop TEXT, title TEXT, desc TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_prog_time ON programmes (source_id, start, stop);

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

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );

        CREATE TABLE IF NOT EXISTS logo_cache (
            url TEXT PRIMARY KEY,
            data TEXT,
            created_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_logo_url ON logo_cache (url);

        CREATE TABLE IF NOT EXISTS tmdb_cache (
            query_title TEXT NOT NULL,
            lang TEXT NOT NULL,
            tmdb_id INTEGER,
            media_type TEXT,
            title TEXT,
            hu_title TEXT,
            en_title TEXT,
            original_title TEXT,
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
    let _ = conn.execute("ALTER TABLE channel_user_settings ADD COLUMN custom_name TEXT", []);
    let _ = conn.execute("ALTER TABLE tmdb_cache ADD COLUMN hu_title TEXT", []);
    let _ = conn.execute("ALTER TABLE tmdb_cache ADD COLUMN en_title TEXT", []);

    // Korábbi alapértelmezett források törlése a tiszta induláshoz
    conn.execute("DELETE FROM sources WHERE id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM channels WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM programmes WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM channel_orders WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();

    conn
}

#[tauri::command]
async fn get_all_favorites_epg_data() -> Result<EpgResponse, String> {
    let conn = init_db();

    let mut stmt = conn.prepare(
        "SELECT c.id, 
                c.source_id,
                COALESCE(s.name, '') AS source_name,
                COALESCE(NULLIF(cus.custom_name, ''), c.name) AS display_name,
                c.name AS original_name,
                cus.custom_name,
                c.icon,
                cus.custom_icon,
                lc.data AS cached_icon,
                COALESCE(cus.is_favorite, 0),
                COALESCE(cus.is_hidden, 0),
                COALESCE(s.country_code, 'HU') AS country_code,
                COALESCE(s.time_offset, 0) AS time_offset
         FROM channels c 
         JOIN sources s ON s.id = c.source_id
         JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id 
         LEFT JOIN channel_orders co_fav ON co_fav.source_id = '__ALL_FAVORITES__' AND co_fav.channel_id = (c.source_id || '::' || c.id)
         LEFT JOIN logo_cache lc ON lc.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
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
         FROM programmes p
         JOIN sources s ON s.id = p.source_id
         JOIN channel_user_settings cus ON cus.source_id = p.source_id AND cus.channel_id = p.channel_id
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
async fn get_epg_data(source_id: String) -> Result<EpgResponse, String> {
    if source_id.trim().is_empty() {
        return Ok(EpgResponse { channels: Vec::new(), programmes: Vec::new() });
    }

    if source_id == "__ALL_FAVORITES__" {
        return get_all_favorites_epg_data().await;
    }

    let conn = init_db();

    let mut stmt = conn.prepare(
        "SELECT c.id, 
                COALESCE(NULLIF(cus.custom_name, ''), c.name) AS display_name,
                c.name AS original_name,
                cus.custom_name,
                c.icon,
                cus.custom_icon,
                lc.data AS cached_icon,
                COALESCE(cus.is_favorite, 0),
                COALESCE(cus.is_hidden, 0)
         FROM channels c 
         LEFT JOIN channel_user_settings cus ON cus.source_id = c.source_id AND cus.channel_id = c.id
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id 
         LEFT JOIN logo_cache lc ON lc.url = COALESCE(NULLIF(cus.custom_icon, ''), c.icon)
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
            source_name: None,
            country_code: None,
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
async fn get_sources() -> Result<Vec<Source>, String> {
    let conn = init_db();
    let mut stmt = conn.prepare("SELECT id, name, url, last_downloaded, country_code, time_offset FROM sources ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            last_downloaded: row.get(3)?,
            country_code: row.get(4)?,
            time_offset: row.get(5)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut sources = Vec::new();
    for s in rows {
        if let Ok(src) = s { sources.push(src); }
    }
    Ok(sources)
}

#[tauri::command]
async fn save_source(source: Source) -> Result<(), String> {
    let conn = init_db();
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
async fn delete_source(source_id: String) -> Result<(), String> {
    let mut conn = init_db();
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM sources WHERE id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channels WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM programmes WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_orders WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM channel_user_settings WHERE source_id = ?1", params![source_id]).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn delete_source_data(source_id: String) -> Result<(), String> {
    delete_source(source_id).await
}

#[tauri::command]
async fn clear_entire_database() -> Result<(), String> {
    let mut conn = init_db();
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM channels", []).ok();
    tx.execute("DELETE FROM programmes", []).ok();
    tx.execute("DELETE FROM tmdb_cache", []).ok();
    tx.execute("UPDATE sources SET last_downloaded = NULL", []).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn get_favorites() -> Result<Vec<String>, String> {
    let conn = init_db();
    let mut stmt = conn.prepare("SELECT title FROM favorites ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?;
    let mut favs = Vec::new();
    for f in rows {
        if let Ok(title) = f { favs.push(title); }
    }
    Ok(favs)
}

#[tauri::command]
async fn toggle_favorite(title: String) -> Result<bool, String> {
    let conn = init_db();
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
async fn set_favorites(favorites: Vec<String>) -> Result<(), String> {
    let mut conn = init_db();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM favorites", []).ok();
    for f in favorites {
        tx.execute("INSERT INTO favorites (title) VALUES (?1)", params![f]).ok();
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn get_channel_order(source_id: String) -> Result<Vec<String>, String> {
    let conn = init_db();
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
async fn save_channel_order(source_id: String, order: Vec<String>) -> Result<(), String> {
    let mut conn = init_db();
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

#[tauri::command]
async fn toggle_channel_favorite(source_id: String, channel_id: String) -> Result<bool, String> {
    let conn = init_db();
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
async fn toggle_channel_hidden(source_id: String, channel_id: String) -> Result<bool, String> {
    let conn = init_db();
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
async fn set_channel_logo(source_id: String, channel_id: String, logo_url: String) -> Result<(), String> {
    let conn = init_db();
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
async fn rename_channel(source_id: String, channel_id: String, name: String) -> Result<(), String> {
    let conn = init_db();
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
async fn get_cached_logo(url: String) -> Result<Option<String>, String> {
    let clean_url = url.trim();
    if clean_url.is_empty() {
        return Ok(None);
    }
    let conn = init_db();
    let res: Option<String> = conn.query_row(
        "SELECT data FROM logo_cache WHERE url = ?1",
        params![clean_url],
        |row| row.get(0),
    ).ok();
    Ok(res)
}

#[tauri::command]
async fn cache_logo(url: String) -> Result<String, String> {
    let clean_url = url.trim().to_string();
    if clean_url.is_empty() {
        return Err("Üres URL".to_string());
    }
    if clean_url.starts_with("data:") {
        return Ok(clean_url);
    }

    let conn = init_db();
    let cached: Option<String> = conn.query_row(
        "SELECT data FROM logo_cache WHERE url = ?1",
        params![clean_url],
        |row| row.get(0),
    ).ok();

    if let Some(data) = cached {
        if !data.is_empty() {
            return Ok(data);
        }
    }

    if !clean_url.starts_with("http://") && !clean_url.starts_with("https://") {
        return Err("Nem támogatott URL protokoll".to_string());
    }

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client.get(&clean_url).send().await.map_err(|e| e.to_string())?;
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

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let data_uri = format!("data:{};base64,{}", mime, b64);

    conn.execute(
        "INSERT INTO logo_cache (url, data, created_at) VALUES (?1, ?2, datetime('now'))
         ON CONFLICT(url) DO UPDATE SET data = excluded.data, created_at = excluded.created_at",
        params![clean_url, data_uri],
    ).map_err(|e| e.to_string())?;

    Ok(data_uri)
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
async fn export_channel_data() -> Result<ChannelDataExport, String> {
    let conn = init_db();

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
async fn import_channel_data(data: ChannelDataExport) -> Result<(), String> {
    let mut conn = init_db();
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
async fn export_settings() -> Result<BackupData, String> {
    let conn = init_db();

    // Sources
    let mut stmt = conn.prepare("SELECT id, name, url, last_downloaded, country_code, time_offset FROM sources ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let sources_iter = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            last_downloaded: row.get(3)?,
            country_code: row.get(4)?,
            time_offset: row.get(5)?,
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
        settings,
    })
}

#[tauri::command]
async fn import_settings(backup: BackupData) -> Result<(), String> {
    let mut conn = init_db();
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
async fn get_settings() -> Result<HashMap<String, String>, String> {
    let conn = init_db();
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
async fn set_setting(key: String, value: String) -> Result<(), String> {
    let conn = init_db();
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TmdbDetails {
    pub tmdb_id: i64,
    pub title: String,
    pub hu_title: Option<String>,
    pub en_title: Option<String>,
    pub original_title: Option<String>,
    pub media_type: Option<String>,
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

    // 3. Remove episode / season patterns with delimiter or explicit keyword:
    // e.g. " - 1. rész", " 2. évad", " - S01E02", " - Ep. 5", " 1. epizód"
    if let Ok(re_ep1) = regex::Regex::new(r"(?i)\s*[-–—:]\s*(?:s\d+e\d+|\d+[\.:]?\s*(?:rész|évad|epizód)|season\s*\d+|episode\s*\d+|ep\.\s*\d+).*$") {
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

    // Season / episode slash numbering e.g. " I./3.", " I/3.", " - I./3.", " 1./3.", " 1/3", " II/4. rész"
    if let Ok(re_slash) = regex::Regex::new(r"(?i)\s*(?:[-–—:]\s*)?(?:[IVXLCDM]+|\d+)\.?\s*/\s*\d+[\.:]?(?:\s*(?:rész|évad|epizód))?.*$") {
        let trimmed_cand = re_slash.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // Roman numeral part with keyword e.g. " I. rész", " II. évad"
    if let Ok(re_roman_part) = regex::Regex::new(r"(?i)\s+(?:[IVXLCDM]+|[ivxlcdm]+)\.?\s*(?:rész|évad|epizód).*$") {
        let trimmed_cand = re_roman_part.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // Roman numeral after dash e.g. " - I.", " - II"
    if let Ok(re_roman) = regex::Regex::new(r"\s*[-–—:]\s*(?:[IVXLCDM]+|[ivxlcdm]+)[\.:]?(?:\s*(?:rész|évad|epizód))?.*$") {
        let trimmed_cand = re_roman.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 4. Remove trailing generic parenthesis/bracket tag if it still exists at the end
    if let Ok(re_tail_paren) = regex::Regex::new(r"\s*[\(\[\{][^\(\)\[\]\{\}]*[\)\]\}]\s*$") {
        let trimmed_cand = re_tail_paren.replace(&s, "").to_string();
        if trimmed_cand.trim().len() >= 2 {
            s = trimmed_cand;
        }
    }

    // 5. Remove trailing punctuation: dashes, colons, dots, slashes, commas
    if let Ok(re_punct) = regex::Regex::new(r"[\s\-\–\—\:\,\.\/\\]+$") {
        s = re_punct.replace(&s, "").to_string();
    }

    // 6. Collapse multiple spaces
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

#[tauri::command]
async fn get_tmdb_api_key() -> Result<Option<String>, String> {
    let conn = init_db();
    let res: Option<String> = conn.query_row(
        "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
        [],
        |row| row.get(0),
    ).ok();
    Ok(res)
}

#[tauri::command]
async fn set_tmdb_api_key(api_key: String) -> Result<(), String> {
    let conn = init_db();
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
async fn clear_tmdb_cache() -> Result<(), String> {
    let conn = init_db();
    conn.execute("DELETE FROM tmdb_cache", []).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn test_tmdb_api_key(api_key: String) -> Result<bool, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Ok(false);
    }
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) EPGViewer/1.0")
        .timeout(std::time::Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;

    let is_v4 = key.starts_with("eyJ") || key.len() > 40;
    let req = if is_v4 {
        client.get("https://api.themoviedb.org/3/authentication")
            .header("Authorization", format!("Bearer {}", key))
            .header("Accept", "application/json")
    } else {
        client.get("https://api.themoviedb.org/3/authentication")
            .query(&[("api_key", key)])
            .header("Accept", "application/json")
    };

    match req.send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(e) => Err(format!("Hálózati hiba: {}", e)),
    }
}

#[tauri::command]
async fn get_tmdb_details(title: String, lang: Option<String>) -> Result<Option<TmdbDetails>, String> {
    let clean = clean_title(&title);
    if clean.is_empty() {
        return Ok(None);
    }

    let target_lang = lang.unwrap_or_else(|| "hu-HU".to_string());
    let conn = init_db();

    // 1. Check local cache
    let cached = conn.query_row(
        "SELECT tmdb_id, media_type, title, hu_title, en_title, original_title, poster_path, backdrop_path, overview, vote_average, release_date
         FROM tmdb_cache WHERE query_title = ?1 AND lang = ?2",
        params![clean, target_lang],
        |row| {
            let tmdb_id: Option<i64> = row.get(0)?;
            let media_type: Option<String> = row.get(1)?;
            let title: Option<String> = row.get(2)?;
            let hu_title: Option<String> = row.get(3)?;
            let en_title: Option<String> = row.get(4)?;
            let original_title: Option<String> = row.get(5)?;
            let poster_path: Option<String> = row.get(6)?;
            let backdrop_path: Option<String> = row.get(7)?;
            let overview: Option<String> = row.get(8)?;
            let vote_average: Option<f64> = row.get(9)?;
            let release_date: Option<String> = row.get(10)?;
            Ok((tmdb_id, media_type, title, hu_title, en_title, original_title, poster_path, backdrop_path, overview, vote_average, release_date))
        }
    ).ok();

    if let Some((tmdb_id, media_type, title, hu_title, en_title, original_title, poster_path, backdrop_path, overview, vote_average, release_date)) = cached {
        if media_type.as_deref() == Some("none") || tmdb_id.is_none() {
            return Ok(None);
        }
        return Ok(Some(TmdbDetails {
            tmdb_id: tmdb_id.unwrap_or(0),
            title: title.unwrap_or_else(|| clean.clone()),
            hu_title,
            en_title,
            original_title,
            media_type,
            poster_path,
            backdrop_path,
            overview,
            vote_average,
            release_date,
        }));
    }

    // 2. Retrieve TMDB API key from settings
    let api_key: Option<String> = conn.query_row(
        "SELECT value FROM settings WHERE key = 'tmdb_api_key'",
        [],
        |row| row.get(0),
    ).ok();

    let api_key = match api_key {
        Some(k) if !k.trim().is_empty() => k.trim().to_string(),
        _ => return Ok(None),
    };

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) EPGViewer/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let is_v4 = api_key.starts_with("eyJ") || api_key.len() > 40;

    let execute_search = |q: &str, l: &str| {
        let mut req = client.get("https://api.themoviedb.org/3/search/multi")
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

    let mut matched_item = search_res.into_iter().find(|i| {
        matches!(i.media_type.as_deref(), Some("movie") | Some("tv"))
    });

    if matched_item.is_none() && target_lang != "en-US" {
        if let Ok(fallback_resp) = execute_search(&clean, "en-US").send().await {
            if let Ok(fallback_text) = fallback_resp.text().await {
                if let Ok(fallback_json) = serde_json::from_str::<TmdbSearchResponse>(&fallback_text) {
                    matched_item = fallback_json.results.unwrap_or_default().into_iter().find(|i| {
                        matches!(i.media_type.as_deref(), Some("movie") | Some("tv"))
                    });
                }
            }
        }
    }

    let item = match matched_item {
        Some(it) => it,
        None => {
            conn.execute(
                "INSERT OR REPLACE INTO tmdb_cache (query_title, lang, media_type, updated_at) VALUES (?1, ?2, 'none', datetime('now', 'localtime'))",
                params![clean, target_lang],
            ).ok();
            return Ok(None);
        }
    };

    let tmdb_id = item.id;
    let media_type = item.media_type.unwrap_or_else(|| "movie".to_string());
    let display_title = item.title.clone().or_else(|| item.name.clone()).unwrap_or_else(|| clean.clone());
    let orig_title = item.original_title.clone().or_else(|| item.original_name.clone());
    let rel_date = item.release_date.clone().or_else(|| item.first_air_date.clone());

    // Translations request to find hu_title and en_title
    let mut hu_title: Option<String> = None;
    let mut en_title: Option<String> = None;
    let mut hu_overview: Option<String> = None;

    let trans_url = format!("https://api.themoviedb.org/3/{}/{}/translations", media_type, tmdb_id);
    let mut trans_req = client.get(&trans_url).header("Accept", "application/json");
    if is_v4 {
        trans_req = trans_req.header("Authorization", format!("Bearer {}", api_key));
    } else {
        trans_req = trans_req.query(&[("api_key", api_key.as_str())]);
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
        media_type: Some(media_type),
        poster_path: item.poster_path,
        backdrop_path: item.backdrop_path,
        overview: final_overview,
        vote_average: item.vote_average,
        release_date: rel_date,
    };

    // Save to cache
    conn.execute(
        "INSERT OR REPLACE INTO tmdb_cache (
            query_title, lang, tmdb_id, media_type, title, hu_title, en_title, original_title,
            poster_path, backdrop_path, overview, vote_average, release_date, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now', 'localtime'))",
        params![
            clean,
            target_lang,
            details.tmdb_id,
            details.media_type,
            details.title,
            details.hu_title,
            details.en_title,
            details.original_title,
            details.poster_path,
            details.backdrop_path,
            details.overview,
            details.vote_average,
            details.release_date,
        ],
    ).ok();

    Ok(Some(details))
}

#[tauri::command]
async fn update_source_epg(source_id: String, url: String) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("Az EPG URL címe üres!".to_string());
    }

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .redirect(reqwest::redirect::Policy::limited(10))
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

    let mut conn = init_db();
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

fn main() {
    tauri::Builder::default()
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
            get_all_favorites_epg_data,
            get_sources,
            save_source,
            delete_source,
            delete_source_data,
            clear_entire_database,
            update_source_epg,
            get_favorites,
            toggle_favorite,
            set_favorites,
            get_channel_order,
            save_channel_order,
            toggle_channel_favorite,
            toggle_channel_hidden,
            set_channel_logo,
            rename_channel,
            cache_logo,
            get_cached_logo,
            export_settings,
            import_settings,
            export_channel_data,
            import_channel_data,
            get_settings,
            set_setting,
            get_tmdb_details,
            get_tmdb_api_key,
            set_tmdb_api_key,
            clear_tmdb_cache,
            test_tmdb_api_key
        ])
        .run(tauri::generate_context!())
        .expect("Hiba a Tauri indításakor");
}
