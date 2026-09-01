#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use flate2::read::GzDecoder;

#[derive(Serialize, Deserialize, Debug)]
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
        CREATE INDEX IF NOT EXISTS idx_prog_time ON programmes (source_id, start, stop);",
    ).unwrap();
    conn
}

#[tauri::command]
async fn get_epg_data(source_id: String) -> Result<EpgResponse, String> {
    let conn = init_db();

    let mut stmt = conn.prepare("SELECT id, name, icon FROM channels WHERE source_id = ?1").map_err(|e| e.to_string())?;
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
async fn delete_source_data(source_id: String) -> Result<(), String> {
    let mut conn = init_db();
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM channels WHERE source_id = ?1", params![source_id]).ok();
    tx.execute("DELETE FROM programmes WHERE source_id = ?1", params![source_id]).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
    Ok(())
}

#[tauri::command]
async fn clear_entire_database() -> Result<(), String> {
    let mut conn = init_db();
    let tx = conn.transaction().map_err(|e| format!("DB tranzakció hiba: {}", e))?;

    tx.execute("DELETE FROM channels", []).ok();
    tx.execute("DELETE FROM programmes", []).ok();

    tx.commit().map_err(|e| format!("Törlési mentési hiba: {}", e))?;
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

    tx.commit().map_err(|e| format!("Mentési hiba: {}", e))?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_epg_data, update_source_epg, delete_source_data, clear_entire_database])
        .run(tauri::generate_context!())
        .expect("Hiba a Tauri indításakor");
}