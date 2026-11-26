use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

use crate::AppState;

pub const CHANGELOG_URL: &str = "https://epg.norti.workers.dev/changelog.json";
pub const REPO_URL: &str = "https://github.com/norti/epg-viewer";
pub const APP_AUTHOR: &str = "norti";
pub const EMBEDDED_CHANGELOG_JSON: &str = include_str!("../../CHANGELOG.json");
pub const STARTUP_CHECK_INTERVAL_SECS: i64 = 86_400;
pub const SETTING_LAST_CHECKED_AT: &str = "update_last_checked_at";

#[derive(Default)]
pub struct UpdaterState {
    pub pending_update: Mutex<Option<Update>>,
    pub check_lock: Mutex<()>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogLangMap {
    #[serde(default)]
    pub hu: Vec<String>,
    #[serde(default)]
    pub en: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogReleaseEntry {
    pub version: String,
    #[serde(default)]
    pub pub_date: String,
    #[serde(default)]
    pub installer_file: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub changelog: ChangelogLangMap,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogDocument {
    #[serde(default)]
    pub latest_version: String,
    #[serde(default)]
    pub releases: Vec<ChangelogReleaseEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalizedReleaseInfo {
    pub version: String,
    pub pub_date: String,
    pub size_bytes: u64,
    pub items: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppAboutInfo {
    pub version: String,
    pub author: String,
    pub repo_url: String,
    pub latest_release: Option<LocalizedReleaseInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub checked: bool,
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub pub_date: Option<String>,
    pub size_bytes: Option<u64>,
    pub releases: Vec<LocalizedReleaseInfo>,
    pub latest_release: Option<LocalizedReleaseInfo>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

pub fn normalize_lang_code(lang: &str) -> &'static str {
    let lower = lang.trim().to_ascii_lowercase();
    if lower == "english" || lower == "en" || lower.starts_with("en-") || lower.starts_with("en_") {
        "en"
    } else {
        "hu"
    }
}

pub fn parse_semver(raw: &str) -> Option<semver::Version> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    semver::Version::parse(trimmed).ok()
}

#[cfg(test)]
pub fn is_newer_semver(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

pub fn parse_changelog_doc(json_str: &str) -> Result<ChangelogDocument, String> {
    serde_json::from_str::<ChangelogDocument>(json_str)
        .map_err(|e| format!("Changelog JSON feldolgozási hiba: {}", e))
}

pub fn pick_localized_items(entry: &ChangelogReleaseEntry, lang: &str) -> Vec<String> {
    let code = normalize_lang_code(lang);
    let primary = if code == "en" {
        &entry.changelog.en
    } else {
        &entry.changelog.hu
    };
    if !primary.is_empty() {
        return primary.clone();
    }
    let fallback = if code == "en" {
        &entry.changelog.hu
    } else {
        &entry.changelog.en
    };
    fallback.clone()
}

pub fn to_localized_release(entry: &ChangelogReleaseEntry, lang: &str) -> LocalizedReleaseInfo {
    LocalizedReleaseInfo {
        version: entry.version.trim_start_matches(['v', 'V']).to_string(),
        pub_date: entry.pub_date.clone(),
        size_bytes: entry.size_bytes,
        items: pick_localized_items(entry, lang),
    }
}

pub fn latest_release_from_doc(doc: &ChangelogDocument, lang: &str) -> Option<LocalizedReleaseInfo> {
    if !doc.latest_version.trim().is_empty() {
        if let Some(target) = parse_semver(&doc.latest_version) {
            if let Some(found) = doc
                .releases
                .iter()
                .find(|r| parse_semver(&r.version).as_ref() == Some(&target))
            {
                return Some(to_localized_release(found, lang));
            }
        }
    }

    doc.releases
        .iter()
        .max_by(|a, b| parse_semver(&a.version).cmp(&parse_semver(&b.version)))
        .map(|r| to_localized_release(r, lang))
}

pub fn collect_releases_between(
    doc: &ChangelogDocument,
    current_ver: &str,
    target_ver: &str,
    lang: &str,
) -> Vec<LocalizedReleaseInfo> {
    let Some(cur) = parse_semver(current_ver) else {
        return Vec::new();
    };
    let Some(target) = parse_semver(target_ver) else {
        return Vec::new();
    };

    let mut matched: Vec<(semver::Version, LocalizedReleaseInfo)> = doc
        .releases
        .iter()
        .filter_map(|entry| {
            let ver = parse_semver(&entry.version)?;
            if ver > cur && ver <= target {
                Some((ver, to_localized_release(entry, lang)))
            } else {
                None
            }
        })
        .collect();

    matched.sort_by(|a, b| b.0.cmp(&a.0));
    matched.into_iter().map(|(_, info)| info).collect()
}

pub fn should_run_startup_check(
    last_checked_epoch: Option<i64>,
    now_epoch: i64,
    interval_secs: i64,
) -> bool {
    match last_checked_epoch {
        None => true,
        Some(last) => {
            if last > now_epoch + 3600 {
                true
            } else {
                now_epoch.saturating_sub(last) >= interval_secs
            }
        }
    }
}

fn now_epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn embedded_latest_release(lang: &str) -> Option<LocalizedReleaseInfo> {
    parse_changelog_doc(EMBEDDED_CHANGELOG_JSON)
        .ok()
        .and_then(|doc| latest_release_from_doc(&doc, lang))
}

async fn fetch_remote_changelog(client: &reqwest::Client) -> Option<ChangelogDocument> {
    let resp = client
        .get(CHANGELOG_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    parse_changelog_doc(&text).ok()
}

fn fallback_items_from_notes(notes: Option<&str>) -> Vec<String> {
    notes
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().trim_start_matches(['-', '*', '•']).trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[tauri::command]
pub async fn get_app_about_info(lang: Option<String>) -> Result<AppAboutInfo, String> {
    let lang_str = lang.unwrap_or_else(|| "hungarian".to_string());
    Ok(AppAboutInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        author: APP_AUTHOR.to_string(),
        repo_url: REPO_URL.to_string(),
        latest_release: embedded_latest_release(&lang_str),
    })
}

#[tauri::command]
pub async fn check_for_updates(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
    force: Option<bool>,
    lang: Option<String>,
) -> Result<UpdateCheckResult, String> {
    let _guard = state.updater.check_lock.lock().await;
    let force_check = force.unwrap_or(true);
    let lang_str = lang.unwrap_or_else(|| "hungarian".to_string());
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let now = now_epoch_secs();

    if !force_check {
        let last_checked: Option<i64> = {
            let conn = state.db.lock().await;
            conn.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![SETTING_LAST_CHECKED_AT],
                |r| r.get::<_, String>(0),
            )
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
        };

        if !should_run_startup_check(last_checked, now, STARTUP_CHECK_INTERVAL_SECS) {
            return Ok(UpdateCheckResult {
                checked: false,
                update_available: false,
                current_version,
                latest_version: None,
                pub_date: None,
                size_bytes: None,
                releases: Vec::new(),
                latest_release: embedded_latest_release(&lang_str),
            });
        }
    }

    let updater = app
        .updater()
        .map_err(|e| format!("Frissítő inicializálási hiba: {}", e))?;

    let update_opt = updater
        .check()
        .await
        .map_err(|e| format!("Frissítés-ellenőrzési hiba: {}", e))?;

    let remote_doc = fetch_remote_changelog(&state.http).await;

    {
        let conn = state.db.lock().await;
        let _ = conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SETTING_LAST_CHECKED_AT, now.to_string()],
        );
    }

    if let Some(update) = update_opt {
        let latest_ver = update.version.trim_start_matches(['v', 'V']).to_string();
        let raw_pub_date = update
            .raw_json
            .get("pub_date")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let mut releases = remote_doc
            .as_ref()
            .map(|doc| collect_releases_between(doc, &current_version, &latest_ver, &lang_str))
            .unwrap_or_default();

        let size_bytes = remote_doc
            .as_ref()
            .and_then(|doc| {
                let target = parse_semver(&latest_ver)?;
                doc.releases
                    .iter()
                    .find(|r| parse_semver(&r.version).as_ref() == Some(&target))
                    .map(|r| r.size_bytes)
            })
            .filter(|&s| s > 0);

        let pub_date = releases
            .first()
            .map(|r| r.pub_date.clone())
            .filter(|s| !s.is_empty())
            .or(raw_pub_date);

        if releases.is_empty() {
            let items = fallback_items_from_notes(update.body.as_deref());
            releases.push(LocalizedReleaseInfo {
                version: latest_ver.clone(),
                pub_date: pub_date.clone().unwrap_or_default(),
                size_bytes: size_bytes.unwrap_or(0),
                items,
            });
        }

        let latest_release = releases.first().cloned();

        {
            let mut pending = state.updater.pending_update.lock().await;
            *pending = Some(update);
        }

        Ok(UpdateCheckResult {
            checked: true,
            update_available: true,
            current_version,
            latest_version: Some(latest_ver),
            pub_date,
            size_bytes,
            releases,
            latest_release,
        })
    } else {
        {
            let mut pending = state.updater.pending_update.lock().await;
            *pending = None;
        }

        let latest_release = remote_doc
            .as_ref()
            .and_then(|doc| latest_release_from_doc(doc, &lang_str))
            .or_else(|| embedded_latest_release(&lang_str));

        Ok(UpdateCheckResult {
            checked: true,
            update_available: false,
            current_version: current_version.clone(),
            latest_version: Some(current_version),
            pub_date: latest_release.as_ref().map(|r| r.pub_date.clone()),
            size_bytes: None,
            releases: Vec::new(),
            latest_release,
        })
    }
}

#[tauri::command]
pub async fn install_available_update(
    state: tauri::State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    let update = {
        let mut pending = state.updater.pending_update.lock().await;
        pending.take()
    };

    let update = match update {
        Some(u) => u,
        None => {
            let updater = app
                .updater()
                .map_err(|e| format!("Frissítő inicializálási hiba: {}", e))?;
            updater
                .check()
                .await
                .map_err(|e| format!("Frissítés-ellenőrzési hiba: {}", e))?
                .ok_or_else(|| "Nincs elérhető frissítés.".to_string())?
        }
    };

    let downloaded = Arc::new(AtomicU64::new(0));
    let downloaded_cb = Arc::clone(&downloaded);
    let app_progress = app.clone();
    let app_finish = app.clone();

    let res = update
        .clone()
        .download_and_install(
            move |chunk_len, content_len| {
                let total_downloaded =
                    downloaded_cb.fetch_add(chunk_len as u64, Ordering::Relaxed) + chunk_len as u64;
                let _ = app_progress.emit(
                    "update-download-progress",
                    UpdateDownloadProgress {
                        downloaded: total_downloaded,
                        total: content_len,
                    },
                );
            },
            move || {
                let _ = app_finish.emit("update-download-finished", ());
            },
        )
        .await;

    if let Err(e) = res {
        let mut pending = state.updater.pending_update.lock().await;
        *pending = Some(update);
        return Err(format!("Frissítés letöltési/telepítési hiba: {}", e));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_comparison_handles_multi_digit_segments() {
        assert!(is_newer_semver("1.0.0", "1.0.1"));
        assert!(is_newer_semver("1.2.0", "1.10.0"));
        assert!(is_newer_semver("1.0.2", "1.0.10"));
        assert!(is_newer_semver("v1.0.0", "v2.0.0"));
        assert!(!is_newer_semver("1.1.0", "1.1.0"));
        assert!(!is_newer_semver("1.2.0", "1.1.9"));
    }

    #[test]
    fn test_embedded_changelog_parses_cleanly() {
        let doc = parse_changelog_doc(EMBEDDED_CHANGELOG_JSON).expect("embedded CHANGELOG.json must parse");
        assert_eq!(doc.latest_version, env!("CARGO_PKG_VERSION"));
        let hu = latest_release_from_doc(&doc, "hungarian").expect("hu release");
        let en = latest_release_from_doc(&doc, "english").expect("en release");
        assert_eq!(hu.version, env!("CARGO_PKG_VERSION"));
        assert!(!hu.items.is_empty());
        assert!(!en.items.is_empty());
        assert_ne!(hu.items, en.items);
    }

    #[test]
    fn test_collect_releases_between_aggregates_intermediate_versions() {
        let sample = r#"{
          "latestVersion": "1.2.0",
          "releases": [
            {
              "version": "1.2.0",
              "pubDate": "2026-12-20T10:00:00Z",
              "installerFile": "epg-viewer_1.2.0_x64-setup.exe",
              "sizeBytes": 5500000,
              "sha256": "abc",
              "changelog": { "hu": ["Új funkció"], "en": ["New feature"] }
            },
            {
              "version": "1.0.1",
              "pubDate": "2026-12-01T10:00:00Z",
              "installerFile": "epg-viewer_1.0.1_x64-setup.exe",
              "sizeBytes": 5100000,
              "sha256": "def",
              "changelog": { "hu": ["Hibajavítás"], "en": ["Bug fix"] }
            },
            {
              "version": "1.0.0",
              "pubDate": "2026-11-25T00:00:00Z",
              "installerFile": "epg-viewer_1.0.0_x64-setup.exe",
              "sizeBytes": 5000000,
              "sha256": "ghi",
              "changelog": { "hu": ["Kezdeti"], "en": ["Initial"] }
            }
          ]
        }"#;

        let doc = parse_changelog_doc(sample).unwrap();
        let between = collect_releases_between(&doc, "1.0.0", "1.2.0", "english");
        assert_eq!(between.len(), 2);
        assert_eq!(between[0].version, "1.2.0");
        assert_eq!(between[0].items, vec!["New feature"]);
        assert_eq!(between[1].version, "1.0.1");
        assert_eq!(between[1].items, vec!["Bug fix"]);
    }

    #[test]
    fn test_startup_check_throttle_respects_24h_interval() {
        let now = 1_700_100_000;
        assert!(should_run_startup_check(None, now, STARTUP_CHECK_INTERVAL_SECS));
        assert!(!should_run_startup_check(
            Some(now - 3600),
            now,
            STARTUP_CHECK_INTERVAL_SECS
        ));
        assert!(should_run_startup_check(
            Some(now - STARTUP_CHECK_INTERVAL_SECS),
            now,
            STARTUP_CHECK_INTERVAL_SECS
        ));
        assert!(should_run_startup_check(
            Some(now + 7200),
            now,
            STARTUP_CHECK_INTERVAL_SECS
        ));
    }
}
