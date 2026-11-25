//! Tests of the fingerprint based progress refresh: a stable account must cost no progress request,
//! a change must cost exactly the affected show, and a failed download must be retried.

use crate::trakt::auth::save_tokens_tx;
use crate::trakt::commands::{classify_history_response, HistoryPostOutcome};
use crate::trakt::secret::default_secret_store;
use crate::trakt::sync::{run_sync, select_progress_show_ids};
use crate::trakt::tests::{make_test_state, spawn_mock_server};
use crate::trakt::{epoch_to_iso_utc, now_epoch_secs, now_iso_utc};
use crate::AppState;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex as StdMutex};

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::migrations::run_migrations(&conn, None).unwrap();
    conn
}

fn iso_ago(secs: i64) -> String {
    epoch_to_iso_utc(now_epoch_secs() - secs)
}

/// A show with a downloaded progress row (and optionally a fingerprint pair) plus one watched episode.
fn seed_show(conn: &Connection, id: i64, updated_at: &str, fp: Option<(&str, &str)>) {
    conn.execute(
        "INSERT INTO trakt_show_progress (show_trakt_id, aired, completed, updated_at) VALUES (?1, 10, 3, ?2)",
        params![id, updated_at],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO trakt_episodes (show_trakt_id, season, episode, watched_at, updated_at) VALUES (?1, 1, 1, ?2, ?2)",
        params![id, now_iso_utc()],
    )
    .unwrap();
    if let Some((seen, synced)) = fp {
        conn.execute(
            "INSERT INTO trakt_show_fp (show_trakt_id, seen_fp, synced_fp, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, seen, synced, now_iso_utc()],
        )
        .unwrap();
    }
}

fn select(conn: &Connection, all: bool) -> Vec<i64> {
    let mut ids = select_progress_show_ids(conn, all, &now_iso_utc()).unwrap();
    ids.sort();
    ids
}

#[test]
fn test_unchanged_shows_need_no_progress_request() {
    let conn = db();
    for id in 1..=35 {
        seed_show(&conn, id, &iso_ago(60), Some(("3:2026-11-20T10:00:00Z", "3:2026-11-20T10:00:00Z")));
    }
    assert!(select(&conn, false).is_empty());
    assert_eq!(select(&conn, true).len(), 35, "a full sync refreshes every candidate");
}

#[test]
fn test_changed_fingerprint_selects_exactly_that_show() {
    let conn = db();
    seed_show(&conn, 1, &iso_ago(60), Some(("3:a", "3:a")));
    seed_show(&conn, 2, &iso_ago(60), Some(("4:b", "3:a")));
    assert_eq!(select(&conn, false), vec![2]);
}

#[test]
fn test_show_without_progress_row_is_selected() {
    let conn = db();
    conn.execute(
        "INSERT INTO trakt_episodes (show_trakt_id, season, episode, watched_at, updated_at) VALUES (7, 1, 1, ?1, ?1)",
        params![now_iso_utc()],
    )
    .unwrap();
    assert_eq!(select(&conn, false), vec![7]);
}

#[test]
fn test_calendar_episode_aired_after_last_download_is_selected() {
    let conn = db();
    seed_show(&conn, 1, &iso_ago(7200), Some(("1:a", "1:a")));
    seed_show(&conn, 2, &iso_ago(7200), Some(("1:a", "1:a")));
    // aired one hour ago (after the download 2 h ago) for show 1; show 2 airs in the future
    for (show, ep, aired) in [(1i64, 11i64, iso_ago(3600)), (2, 12, epoch_to_iso_utc(now_epoch_secs() + 86_400))] {
        conn.execute(
            "INSERT INTO trakt_calendar_cache (scope, episode_trakt_id, show_trakt_id, show_title, season, number, first_aired, fetched_at)
             VALUES ('my', ?1, ?2, 'S', 1, 2, ?3, ?3)",
            params![ep, show, aired],
        )
        .unwrap();
    }
    assert_eq!(select(&conn, false), vec![1]);
}

#[test]
fn test_next_episode_that_aired_since_the_download_is_selected() {
    let conn = db();
    seed_show(&conn, 1, &iso_ago(7200), Some(("1:a", "1:a")));
    seed_show(&conn, 2, &iso_ago(7200), Some(("1:a", "1:a")));
    // Trakt style timestamp with fraction; compared to the second
    conn.execute(
        "UPDATE trakt_show_progress SET next_episode_trakt_id = 5, next_episode_first_aired = ?1 WHERE show_trakt_id = 1",
        params![iso_ago(3600).replace('Z', ".000Z")],
    )
    .unwrap();
    conn.execute(
        "UPDATE trakt_show_progress SET next_episode_trakt_id = 6, next_episode_first_aired = ?1 WHERE show_trakt_id = 2",
        params![epoch_to_iso_utc(now_epoch_secs() + 86_400)],
    )
    .unwrap();
    assert_eq!(select(&conn, false), vec![1]);
}

#[test]
fn test_weekly_safety_net_is_capped_and_hidden_shows_are_skipped() {
    let conn = db();
    for id in 1..=8 {
        seed_show(&conn, id, &iso_ago(8 * 86_400), Some(("1:a", "1:a")));
    }
    seed_show(&conn, 9, &iso_ago(8 * 86_400), Some(("1:a", "1:a")));
    conn.execute("UPDATE trakt_show_progress SET hidden_from_progress = 1 WHERE show_trakt_id = 9", [])
        .unwrap();
    let ids = select(&conn, false);
    assert_eq!(ids.len(), 5, "at most 5 stale shows per sync: {:?}", ids);
    assert!(!ids.contains(&9));
}

#[test]
fn test_history_response_classification() {
    assert_eq!(
        classify_history_response(r#"{"added":{"episodes":1,"movies":0},"not_found":{"episodes":[],"movies":[]}}"#),
        HistoryPostOutcome::Added
    );
    assert_eq!(
        classify_history_response(r#"{"added":{"episodes":0,"movies":0},"not_found":{"episodes":[{"ids":{"trakt":1}}]}}"#),
        HistoryPostOutcome::Rejected
    );
    assert_eq!(classify_history_response("not json"), HistoryPostOutcome::Unknown);
    assert_eq!(classify_history_response("{}"), HistoryPostOutcome::Unknown);
}

struct Server {
    /// Paths of every request, in order.
    log: Arc<StdMutex<Vec<String>>>,
    watched: Arc<StdMutex<String>>,
    activities: Arc<StdMutex<String>>,
    fail_progress_for: Arc<StdMutex<Option<i64>>>,
    fail_calendar: Arc<StdMutex<bool>>,
}

fn watched_entry(show: i64, episodes: &[(i64, &str)]) -> String {
    let eps: Vec<String> = episodes
        .iter()
        .map(|(n, w)| format!(r#"{{"number":{n},"last_watched_at":"{w}"}}"#))
        .collect();
    format!(
        r#"{{"show":{{"title":"Show {show}","year":2024,"ids":{{"trakt":{show},"tmdb":{}}}}},"seasons":[{{"number":1,"episodes":[{}]}}]}}"#,
        show + 1000,
        eps.join(",")
    )
}

fn activities(episodes_watched_at: &str) -> String {
    format!(
        r#"{{"episodes":{{"watched_at":"{episodes_watched_at}"}},"movies":{{"watched_at":"2026-11-10T10:00:00Z"}},"watchlist":{{"updated_at":"2026-11-10T10:00:00Z"}}}}"#
    )
}

async fn start_server() -> (String, Server) {
    let log: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
    let watched = Arc::new(StdMutex::new(format!(
        "[{},{},{}]",
        watched_entry(101, &[(1, "2026-11-20T10:00:00Z")]),
        watched_entry(102, &[(1, "2026-11-20T11:00:00Z")]),
        watched_entry(103, &[(1, "2026-11-20T12:00:00Z")]),
    )));
    let acts = Arc::new(StdMutex::new(activities("2026-11-20T12:00:00Z")));
    let fail = Arc::new(StdMutex::new(None::<i64>));
    let fail_cal = Arc::new(StdMutex::new(false));
    let (l, w, a, f, fc) = (log.clone(), watched.clone(), acts.clone(), fail.clone(), fail_cal.clone());
    let base = spawn_mock_server(move |method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if method != "GET" {
            return (404, vec![], "{}".to_string());
        }
        if path == "/sync/last_activities" {
            return (200, vec![], a.lock().unwrap().clone());
        }
        if path == "/sync/watched/shows" {
            return (200, vec![], w.lock().unwrap().clone());
        }
        if path.starts_with("/sync/history/episodes") {
            let start_at = path
                .split("start_at=")
                .nth(1)
                .and_then(|s| s.split('&').next())
                .unwrap_or("");
            let watched_json: serde_json::Value =
                serde_json::from_str(&w.lock().unwrap()).unwrap_or(serde_json::json!([]));
            let mut out: Vec<serde_json::Value> = Vec::new();
            let mut hid = 100i64;
            if let Some(arr) = watched_json.as_array() {
                for item in arr {
                    let show = &item["show"];
                    if let Some(seasons) = item["seasons"].as_array() {
                        for s in seasons {
                            let snum = s["number"].as_i64().unwrap_or(1);
                            if let Some(eps) = s["episodes"].as_array() {
                                for ep in eps {
                                    let ts = ep["last_watched_at"].as_str().unwrap_or("");
                                    if ts >= start_at {
                                        hid += 1;
                                        let enum_ = ep["number"].as_i64().unwrap_or(1);
                                        out.push(serde_json::json!({
                                            "id": hid,
                                            "watched_at": ts,
                                            "type": "episode",
                                            "show": show,
                                            "episode": {
                                                "season": snum,
                                                "number": enum_,
                                                "title": format!("E{}", enum_),
                                                "ids": { "trakt": hid * 10 }
                                            }
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return (200, vec![], serde_json::Value::Array(out).to_string());
        }
        if path.starts_with("/calendars/my/shows/") && *fc.lock().unwrap() {
            return (500, vec![], "{}".to_string());
        }
        if path.starts_with("/sync/watchlist/") || path.starts_with("/calendars/my/shows/") || path.starts_with("/users/hidden/") {
            return (200, vec![], "[]".to_string());
        }
        if path.starts_with("/users/me/history/") {
            return (
                200,
                vec![],
                r#"[{"id":1,"watched_at":"2026-11-20T12:00:00Z","type":"episode","show":{"title":"Show 103","ids":{"trakt":103}},"episode":{"season":1,"number":1,"title":"E","ids":{"trakt":9103}}}]"#.to_string(),
            );
        }
        if path == "/users/settings" {
            return (200, vec![], r#"{"user":{"username":"tester"}}"#.to_string());
        }
        if let Some(rest) = path.strip_prefix("/shows/") {
            if let Some((id, _)) = rest.split_once("/progress/watched") {
                let id: i64 = id.parse().unwrap_or(0);
                if *f.lock().unwrap() == Some(id) {
                    return (500, vec![], "{}".to_string());
                }
                return (
                    200,
                    vec![],
                    r#"{"aired":5,"completed":1,"last_watched_at":"2026-11-20T10:00:00Z","seasons":[{"number":1,"episodes":[{"number":1,"completed":true,"last_watched_at":"2026-11-20T10:00:00Z"}]}]}"#.to_string(),
                );
            }
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    (base, Server { log, watched, activities: acts, fail_progress_for: fail, fail_calendar: fail_cal })
}

async fn logged_in_state(base: String) -> AppState {
    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    let mut conn = state.db.lock().await;
    save_tokens_tx(&mut conn, store.as_ref(), "valid_acc", "valid_ref", now_epoch_secs() + 3600).unwrap();
    drop(conn);
    state
}

fn progress_hits(server: &Server) -> Vec<String> {
    server
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.contains("/progress/watched"))
        .cloned()
        .collect()
}

#[tokio::test]
async fn test_stable_account_costs_no_progress_requests_and_one_change_costs_one() {
    let (base, server) = start_server().await;
    let state = logged_in_state(base).await;

    // 1st sync: nothing is known yet -> every show downloads its progress once
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(progress_hits(&server).len(), 3);

    // 2nd sync, nothing changed on Trakt -> no progress request at all
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert!(progress_hits(&server).is_empty(), "got {:?}", progress_hits(&server));
    // Scenario A: a focus sync of an unchanged account costs at most 2 Trakt requests (0 TMDB).
    let total = server.log.lock().unwrap().clone();
    assert!(total.len() <= 2, "scenario A: {:?}", total);

    // Show 102 gets a second watched episode -> exactly one progress request, for show 102
    *server.watched.lock().unwrap() = format!(
        "[{},{},{}]",
        watched_entry(101, &[(1, "2026-11-20T10:00:00Z")]),
        watched_entry(102, &[(1, "2026-11-20T11:00:00Z"), (2, "2026-11-23T09:00:00Z")]),
        watched_entry(103, &[(1, "2026-11-20T12:00:00Z")]),
    );
    *server.activities.lock().unwrap() = activities("2026-11-23T09:00:00Z");
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    let hits = progress_hits(&server);
    assert_eq!(hits.len(), 1, "got {:?}", hits);
    assert!(hits[0].starts_with("/shows/102/progress/watched"));
    // Scenario B: the sync after one watched episode costs at most 5 requests
    // (last_activities, history, watched/shows, 1 progress).
    let total = server.log.lock().unwrap().clone();
    assert!(total.len() <= 5, "scenario B: {:?}", total);

    // ... and the next sync is quiet again
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert!(progress_hits(&server).is_empty());
}

#[tokio::test]
async fn test_failed_progress_download_is_retried_on_the_next_sync_only_for_that_show() {
    let (base, server) = start_server().await;
    let state = logged_in_state(base).await;

    *server.fail_progress_for.lock().unwrap() = Some(102);
    run_sync(&state, None, false).await.unwrap();

    let conn = state.db.lock().await;
    let synced: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_show_fp WHERE synced_fp IS NOT NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(synced, 2, "the failed show must stay un-synced");
    drop(conn);

    *server.fail_progress_for.lock().unwrap() = None;
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    let hits = progress_hits(&server);
    assert_eq!(hits.len(), 1, "got {:?}", hits);
    assert!(hits[0].starts_with("/shows/102/progress/watched"));
}

fn hits_with_prefix(server: &Server, prefix: &str) -> usize {
    server.log.lock().unwrap().iter().filter(|p| p.starts_with(prefix)).count()
}

#[tokio::test]
async fn test_late_calendar_failure_does_not_redownload_history_on_retry() {
    let (base, server) = start_server().await;
    let state = logged_in_state(base).await;

    *server.fail_calendar.lock().unwrap() = true;
    assert!(run_sync(&state, None, false).await.is_err(), "the calendar failure aborts the sync");
    assert_eq!(hits_with_prefix(&server, "/users/me/history/"), 1);
    assert_eq!(hits_with_prefix(&server, "/sync/watched/shows"), 1);

    // The history stage finished, so its stamps were saved even though the sync as a whole failed.
    *server.fail_calendar.lock().unwrap() = false;
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/me/history/"), 0, "history must not be fetched again");
    assert_eq!(hits_with_prefix(&server, "/sync/watched/shows"), 0);
    // the progress of the shows (never downloaded because the calendar step aborted) is still caught up
    assert_eq!(progress_hits(&server).len(), 3);
}

#[tokio::test]
async fn test_hidden_list_is_only_requested_when_hidden_at_changes() {
    let (base, server) = start_server().await;
    let state = logged_in_state(base).await;
    let with_hidden = |hidden_at: &str| {
        format!(
            r#"{{"episodes":{{"watched_at":"2026-11-20T12:00:00Z"}},"movies":{{"watched_at":"2026-11-10T10:00:00Z"}},"shows":{{"hidden_at":"{hidden_at}"}},"watchlist":{{"updated_at":"2026-11-10T10:00:00Z"}}}}"#
        )
    };

    *server.activities.lock().unwrap() = with_hidden("2026-11-01T00:00:00Z");
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/hidden/"), 1, "first sync reads the hidden list");

    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/hidden/"), 0, "unchanged hidden_at -> no request");

    *server.activities.lock().unwrap() = with_hidden("2026-11-24T00:00:00Z");
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/hidden/"), 1, "changed hidden_at -> one request");

    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/hidden/"), 0);

    // Changing shows.dropped_at while hidden_at stays unchanged must also refresh the hidden list
    *server.activities.lock().unwrap() = r#"{"episodes":{"watched_at":"2026-11-20T12:00:00Z"},"movies":{"watched_at":"2026-11-10T10:00:00Z"},"shows":{"hidden_at":"2026-11-24T00:00:00Z","dropped_at":"2026-11-25T09:00:00Z"},"watchlist":{"updated_at":"2026-11-10T10:00:00Z"}}"#.to_string();
    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/hidden/"), 1, "changed dropped_at -> one request");

    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/users/hidden/"), 0);
}

#[tokio::test]
async fn test_clear_trakt_cache_clears_fingerprints_and_sync_flags() {
    let (base, _server) = start_server().await;
    let state = logged_in_state(base).await;
    run_sync(&state, None, false).await.unwrap();

    let fp_before: i64 = state
        .db
        .lock()
        .await
        .query_row("SELECT COUNT(*) FROM trakt_show_fp", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fp_before, 3);

    {
        let mut conn = state.db.lock().await;
        crate::trakt::commands::clear_trakt_cache_db(&mut conn).unwrap();
    }

    let fp_after: i64 = state
        .db
        .lock()
        .await
        .query_row("SELECT COUNT(*) FROM trakt_show_fp", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fp_after, 0, "trakt_show_fp must be emptied by clear_trakt_cache");
}

#[tokio::test]
async fn test_optional_mark_watched_activity_skip_respects_setting() {
    let (base, server) = start_server().await;
    let state = logged_in_state(base).await;
    run_sync(&state, None, false).await.unwrap();

    // Default (off): maybe_record_mark_watched_activity does not touch last_activities
    *server.activities.lock().unwrap() = activities("2026-11-25T10:00:00Z");
    server.log.lock().unwrap().clear();
    crate::trakt::sync::maybe_record_mark_watched_activity(&state, None, "episode").await;
    assert_eq!(hits_with_prefix(&server, "/sync/last_activities"), 0);

    // Enabled: advances episodes.watched_at so the next sync skips the history branch
    state
        .db
        .lock()
        .await
        .execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('trakt_skip_next_history_sync_after_mark', 'true')",
            [],
        )
        .unwrap();
    crate::trakt::sync::maybe_record_mark_watched_activity(&state, None, "episode").await;
    assert_eq!(hits_with_prefix(&server, "/sync/last_activities"), 1);

    server.log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(hits_with_prefix(&server, "/sync/watched/shows"), 0);
    assert_eq!(hits_with_prefix(&server, "/users/me/history/"), 0);
}

#[tokio::test]
async fn test_incremental_watched_sync_avoids_full_download_and_reconciles_weekly() {
    let log: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
    let activities_body = Arc::new(StdMutex::new(activities("2026-11-20T12:00:00Z")));
    let watched_body = Arc::new(StdMutex::new(format!(
        "[{},{}]",
        watched_entry(101, &[(1, "2026-11-20T10:00:00Z")]),
        watched_entry(102, &[(1, "2026-11-20T11:00:00Z")]),
    )));
    let inc_body = Arc::new(StdMutex::new("[]".to_string()));

    let (l, a, w, inc) = (
        log.clone(),
        activities_body.clone(),
        watched_body.clone(),
        inc_body.clone(),
    );
    let base = spawn_mock_server(move |method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if method != "GET" {
            return (404, vec![], "{}".to_string());
        }
        if path == "/sync/last_activities" {
            return (200, vec![], a.lock().unwrap().clone());
        }
        if path == "/sync/watched/shows" {
            return (200, vec![], w.lock().unwrap().clone());
        }
        if path.starts_with("/sync/history/episodes") {
            return (200, vec![], inc.lock().unwrap().clone());
        }
        if path.starts_with("/sync/watchlist/")
            || path.starts_with("/calendars/my/shows/")
            || path.starts_with("/users/hidden/")
        {
            return (200, vec![], "[]".to_string());
        }
        if path.starts_with("/users/me/history/") {
            return (
                200,
                vec![],
                r#"[{"id":1,"watched_at":"2026-11-20T12:00:00Z","type":"episode","show":{"title":"Show 102","ids":{"trakt":102}},"episode":{"season":1,"number":1,"title":"E1","ids":{"trakt":9102}}}]"#.to_string(),
            );
        }
        if path == "/users/settings" {
            return (200, vec![], r#"{"user":{"username":"tester"}}"#.to_string());
        }
        if path.contains("/progress/watched") {
            return (
                200,
                vec![],
                r#"{"aired":5,"completed":2,"last_watched_at":"2026-11-23T09:00:00Z","seasons":[{"number":1,"episodes":[{"number":1,"completed":true,"last_watched_at":"2026-11-20T11:00:00Z"},{"number":2,"completed":true,"last_watched_at":"2026-11-23T09:00:00Z"}]}]}"#.to_string(),
            );
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = logged_in_state(base).await;

    // 1st sync: no full sync yet -> calls /sync/watched/shows
    run_sync(&state, None, false).await.unwrap();
    assert!(log.lock().unwrap().iter().any(|p| p == "/sync/watched/shows"));

    // 2nd sync with 1 new episode -> uses incremental /sync/history/episodes, 0 /sync/watched/shows!
    *inc_body.lock().unwrap() = r#"[{"id":2,"watched_at":"2026-11-23T09:00:00Z","type":"episode","show":{"title":"Show 102","year":2024,"ids":{"trakt":102,"tmdb":1102}},"episode":{"season":1,"number":2,"title":"E2","ids":{"trakt":9202}}}]"#.to_string();
    *activities_body.lock().unwrap() = activities("2026-11-23T09:00:00Z");
    log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    {
        let reqs = log.lock().unwrap();
        assert_eq!(reqs.iter().filter(|p| p.as_str() == "/sync/watched/shows").count(), 0, "incremental sync must not call /sync/watched/shows");
        assert_eq!(reqs.iter().filter(|p| p.starts_with("/sync/history/episodes?start_at=")).count(), 1);
        let prog: Vec<&String> = reqs.iter().filter(|p| p.contains("/progress/watched")).collect();
        assert_eq!(prog.len(), 1);
        assert!(prog[0].starts_with("/shows/102/progress/watched"));
    }

    // 3rd sync after >7 days since last full sync -> falls back to full /sync/watched/shows reconciliation
    state
        .db
        .lock()
        .await
        .execute(
            "UPDATE trakt_sync_state SET value = '2020-01-01T00:00:00Z' WHERE key = 'watched_full_sync_at'",
            [],
        )
        .unwrap();
    *activities_body.lock().unwrap() = activities("2026-11-24T10:00:00Z");
    log.lock().unwrap().clear();
    run_sync(&state, None, false).await.unwrap();
    assert_eq!(
        log.lock().unwrap().iter().filter(|p| p.as_str() == "/sync/watched/shows").count(),
        1,
        "stale watched_full_sync_at (>7d) must trigger full reconciliation"
    );
}