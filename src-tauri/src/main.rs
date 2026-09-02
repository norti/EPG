#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use flate2::read::GzDecoder;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Channel {
    id: String,
    name: String,
    icon: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Programme {
    channel: String,
    start: String,
    stop: String,
    title: String,
    desc: String,
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
            last_downloaded TEXT
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

        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT
        );",
    ).unwrap();

    // Korábbi alapértelmezett források törlése a tiszta induláshoz
    conn.execute("DELETE FROM sources WHERE id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM channels WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM programmes WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();
    conn.execute("DELETE FROM channel_orders WHERE source_id IN ('src_porthu', 'src_magenta', 'src_sky')", []).ok();

    conn
}

#[tauri::command]
async fn get_epg_data(source_id: String) -> Result<EpgResponse, String> {
    if source_id.trim().is_empty() {
        return Ok(EpgResponse { channels: Vec::new(), programmes: Vec::new() });
    }

    let conn = init_db();

    let mut stmt = conn.prepare(
        "SELECT c.id, c.name, c.icon 
         FROM channels c 
         LEFT JOIN channel_orders co ON co.source_id = c.source_id AND co.channel_id = c.id 
         WHERE c.source_id = ?1 
         ORDER BY CASE WHEN co.position IS NOT NULL THEN 0 ELSE 1 END, co.position ASC, c.name ASC"
    ).map_err(|e| e.to_string())?;

    let channel_iter = stmt.query_map(params![source_id], |row| {
        Ok(Channel {
            id: row.get(0)?,
            name: row.get(1)?,
            icon: row.get(2)?,
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
    let mut stmt = conn.prepare("SELECT id, name, url, last_downloaded FROM sources ORDER BY rowid ASC").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |row| {
        Ok(Source {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            last_downloaded: row.get(3)?,
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
        "INSERT INTO sources (id, name, url, last_downloaded) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET name = excluded.name, url = excluded.url, last_downloaded = COALESCE(excluded.last_downloaded, sources.last_downloaded)",
        params![source.id, source.name, source.url, source.last_downloaded],
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

    // Frissítési időbélyeg elmentése a sources táblába
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
                // DWMWA_USE_IMMERSIVE_DARK_MODE = 20
                let dark_mode: i32 = 1;
                set_attr(hwnd, 20, &dark_mode as *const _ as *const c_void, 4);

                // DWMWA_CAPTION_COLOR = 35 (Windows 11): 0x001F1F1F (#1F1F1F szürke az inaktív ablak stílusához)
                let caption_color: u32 = 0x001F1F1F;
                set_attr(hwnd, 35, &caption_color as *const _ as *const c_void, 4);

                // DWMWA_TEXT_COLOR = 36 (Windows 11): fehér felirat
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
            get_settings,
            set_setting
        ])
        .run(tauri::generate_context!())
        .expect("Hiba a Tauri indításakor");
}