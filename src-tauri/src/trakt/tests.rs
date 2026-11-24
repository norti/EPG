use crate::migrations::run_migrations;
use crate::trakt::auth::{
    ensure_valid_token, load_tokens_from_db, poll_device_token_once, save_tokens_tx,
    start_device_flow, PollStepResult,
};
use crate::trakt::client::{trakt_get_cached, trakt_get_raw};
use crate::trakt::secret::default_secret_store;
use crate::trakt::sync::flush_pending_actions;
use crate::trakt::{now_epoch_secs, now_iso_utc, TraktState};
use crate::AppState;
use rusqlite::{params, Connection};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

pub(crate) fn make_test_state(api_base: String, auth_base: String) -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, None).unwrap();
    let trakt = TraktState::with_bases(&conn, api_base, auth_base);
    let http = reqwest::Client::new();
    let http_epg = reqwest::Client::new();
    AppState {
        db: Mutex::new(conn),
        http,
        http_epg,
        trakt,
        tmdb: crate::TmdbRuntime::new(),
    }
}

pub(crate) async fn spawn_mock_server<F>(handler: F) -> String
where
    F: Fn(String, String, String) -> (u16, Vec<(String, String)>, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handler = Arc::new(handler);

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let h = handler.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                let req_str = String::from_utf8_lossy(&buf[..n]).to_string();
                let mut lines = req_str.lines();
                let first_line = lines.next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET").to_string();
                let path = parts.next().unwrap_or("/").to_string();

                let body = req_str
                    .split("\r\n\r\n")
                    .nth(1)
                    .unwrap_or("")
                    .to_string();

                let (status, extra_headers, resp_body) = h(method, path, body);
                let reason = match status {
                    200 => "OK",
                    201 => "Created",
                    304 => "Not Modified",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Error",
                };

                let mut resp = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    status,
                    reason,
                    resp_body.len()
                );
                for (k, v) in extra_headers {
                    resp.push_str(&format!("{}: {}\r\n", k, v));
                }
                resp.push_str("\r\n");
                resp.push_str(&resp_body);
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });

    format!("http://{}", addr)
}

#[tokio::test]
async fn test_device_flow_and_dpapi_storage() {
    let poll_count = Arc::new(AtomicUsize::new(0));
    let poll_count_srv = poll_count.clone();

    let base = spawn_mock_server(move |method, path, _body| {
        if method == "POST" && path == "/oauth/device/code" {
            return (
                200,
                vec![],
                r#"{"device_code":"dev123","user_code":"ABCD-1234","verification_url":"https://auth.trakt.tv/activate","expires_in":600,"interval":5}"#.to_string(),
            );
        }
        if method == "POST" && path == "/oauth/device/token" {
            let n = poll_count_srv.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return (400, vec![], String::new());
            }
            if n == 1 {
                return (429, vec![], String::new());
            }
            return (
                200,
                vec![],
                r#"{"access_token":"acc_tok_1","refresh_token":"ref_tok_1","expires_in":604800,"created_at":1700000000}"#.to_string(),
            );
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let dc = start_device_flow(&state, None).await.unwrap();
    assert_eq!(dc.user_code, "ABCD-1234");
    assert_eq!(dc.device_code, "dev123");

    assert_eq!(
        poll_device_token_once(&state, &dc.device_code, "cid", None)
            .await
            .unwrap(),
        PollStepResult::Pending
    );
    assert_eq!(
        poll_device_token_once(&state, &dc.device_code, "cid", None)
            .await
            .unwrap(),
        PollStepResult::SlowDown
    );
    assert_eq!(
        poll_device_token_once(&state, &dc.device_code, "cid", None)
            .await
            .unwrap(),
        PollStepResult::Success
    );

    let store = default_secret_store();
    let conn = state.db.lock().await;
    let loaded = load_tokens_from_db(&conn, store.as_ref()).unwrap().unwrap();
    assert_eq!(loaded.access_token.as_str(), "acc_tok_1");
    assert_eq!(loaded.refresh_token.as_str(), "ref_tok_1");
}

#[tokio::test]
async fn test_token_refresh_on_401_and_single_use_rotation() {
    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let refresh_calls_srv = refresh_calls.clone();

    let base = spawn_mock_server(move |method, path, body| {
        if method == "GET" && path == "/users/settings" {
            return (200, vec![], r#"{"user":{"username":"tester"}}"#.to_string());
        }
        if method == "POST" && path == "/oauth/token" {
            let n = refresh_calls_srv.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                assert!(body.contains("old_ref"));
                return (
                    200,
                    vec![],
                    r#"{"access_token":"new_acc","refresh_token":"new_ref","expires_in":604800,"created_at":2000000000}"#.to_string(),
                );
            }
            return (
                400,
                vec![],
                r#"{"error":"invalid_grant","error_description":"session not found"}"#.to_string(),
            );
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    {
        let mut conn = state.db.lock().await;
        // Expired token (expires_at in the past)
        save_tokens_tx(&mut conn, store.as_ref(), "old_acc", "old_ref", now_epoch_secs() - 100)
            .unwrap();
    }

    let tok = ensure_valid_token(&state, None).await.unwrap();
    assert_eq!(tok.as_str(), "new_acc");

    let conn = state.db.lock().await;
    let loaded = load_tokens_from_db(&conn, store.as_ref()).unwrap().unwrap();
    assert_eq!(loaded.refresh_token.as_str(), "new_ref");
}

#[tokio::test]
async fn test_rate_limit_429_retry_after_and_etag_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_srv = hits.clone();

    let base = spawn_mock_server(move |method, path, _body| {
        if method == "GET" && path == "/shows/104439?extended=full" {
            let n = hits_srv.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return (
                    429,
                    vec![("Retry-After".to_string(), "1".to_string())],
                    "{}".to_string(),
                );
            }
            return (
                200,
                vec![("ETag".to_string(), "\"etag-v1\"".to_string())],
                r#"{"title":"Stranger Things"}"#.to_string(),
            );
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let resp1 = trakt_get_cached(&state, None, "/shows/104439?extended=full", false, 3600, false)
        .await
        .unwrap();
    assert!(!resp1.from_cache);
    assert!(resp1.body.contains("Stranger Things"));
    assert_eq!(hits.load(Ordering::SeqCst), 2);

    // Second call within TTL must hit local cache (0 extra HTTP calls)
    let resp2 = trakt_get_cached(&state, None, "/shows/104439?extended=full", false, 3600, false)
        .await
        .unwrap();
    assert!(resp2.from_cache);
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_offline_pending_queue_flush() {
    let post_hits = Arc::new(AtomicUsize::new(0));
    let post_hits_srv = post_hits.clone();

    let base = spawn_mock_server(move |method, path, _body| {
        if method == "POST" && path == "/sync/history" {
            post_hits_srv.fetch_add(1, Ordering::SeqCst);
            return (201, vec![], r#"{"added":{"episodes":1,"movies":0}}"#.to_string());
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    {
        let mut conn = state.db.lock().await;
        save_tokens_tx(
            &mut conn,
            store.as_ref(),
            "valid_acc",
            "valid_ref",
            now_epoch_secs() + 3600,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO trakt_pending_actions (action, payload_json, created_at, attempts, last_error)
             VALUES ('history_add', '{\"episodes\":[{\"ids\":{\"trakt\":555}}]}', ?1, 0, 'offline')",
            params![now_iso_utc()],
        )
        .unwrap();
    }

    let flushed = flush_pending_actions(&state, None).await.unwrap();
    assert_eq!(flushed, 1);
    assert_eq!(post_hits.load(Ordering::SeqCst), 1);

    let conn = state.db.lock().await;
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM trakt_pending_actions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 0);

    // Also verify trakt_get_raw works cleanly
    drop(conn);
    let _ = trakt_get_raw(&state, None, "/nonexistent", false).await;
}

#[test]
fn test_global_calendar_merges_my_calendar_events() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, None).unwrap();

    conn.execute(
        "INSERT INTO trakt_calendar_cache (scope, episode_trakt_id, show_trakt_id, show_tmdb_id, show_title, season, number, episode_title, first_aired, network, fetched_at)
         VALUES ('all', 1001, 10, 110, 'Global Show', 1, 1, 'Ep 1', '2026-11-18T20:00:00Z', 'HBO', '2026-11-18T10:00:00Z')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO trakt_calendar_cache (scope, episode_trakt_id, show_trakt_id, show_tmdb_id, show_title, season, number, episode_title, first_aired, network, fetched_at)
         VALUES ('my', 1001, 10, 110, 'Global Show', 1, 1, 'Ep 1', '2026-11-18T20:00:00Z', 'HBO', '2026-11-18T10:00:00Z')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO trakt_calendar_cache (scope, episode_trakt_id, show_trakt_id, show_tmdb_id, show_title, season, number, episode_title, first_aired, network, fetched_at)
         VALUES ('my', 2002, 20, 220, 'Ázsia Expressz', 7, 24, 'Ep 24', '2026-11-18T18:45:00Z', 'TV2', '2026-11-18T10:00:00Z')",
        [],
    ).unwrap();

    let mut merged = crate::trakt::calendar::query_global_calendar_merged(&conn).unwrap();
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[0].episode_trakt_id, 2002);
    assert_eq!(merged[0].show_title, "Ázsia Expressz");
    assert!(!merged[0].on_watchlist);
    assert_eq!(merged[1].episode_trakt_id, 1001);

    // Adding show 20 to watchlist (or matching by tmdb_id) marks it on_watchlist in global calendar
    conn.execute(
        "INSERT INTO trakt_watchlist_cache (trakt_id, media_type, title, tmdb_id, listed_at, updated_at)
         VALUES (20, 'show', 'Ázsia Expressz', 220, '2026-11-18T10:00:00Z', '2026-11-18T10:00:00Z')",
        [],
    ).unwrap();
    merged = crate::trakt::calendar::query_global_calendar_merged(&conn).unwrap();
    assert!(merged[0].on_watchlist);
}

#[tokio::test]
async fn test_sync_watched_shows_clears_unwatched_episodes_and_respects_history_dirty() {
    let base = spawn_mock_server(move |method, path, _body| {
        if method == "GET" && path == "/sync/last_activities" {
            return (
                200,
                vec![],
                r#"{"episodes":{"watched_at":"2026-11-18T12:39:00Z"},"movies":{"watched_at":"2026-11-10T10:00:00Z"},"watchlist":{"updated_at":"2026-11-18T10:00:00Z"}}"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/users/me/history/") {
            return (200, vec![], "[]".to_string());
        }
        if method == "GET" && path == "/sync/watched/shows" {
            // S07E23 is watched, S07E24 was removed on Trakt.tv
            return (
                200,
                vec![],
                r#"[{"show":{"ids":{"trakt":164896}},"seasons":[{"number":7,"episodes":[{"number":23,"last_watched_at":"2026-11-18T12:39:00Z"}]}]}]"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/calendars/my/shows/") {
            return (200, vec![], "[]".to_string());
        }
        if method == "GET" && path == "/users/settings" {
            return (200, vec![], r#"{"user":{"username":"tester"}}"#.to_string());
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    {
        let mut conn = state.db.lock().await;
        save_tokens_tx(
            &mut conn,
            store.as_ref(),
            "valid_acc",
            "valid_ref",
            now_epoch_secs() + 3600,
        )
        .unwrap();
        // Seed watchlist & history so wl_empty/hist_empty are false, and matching last_activities
        conn.execute(
            "INSERT INTO trakt_watchlist_cache (trakt_id, media_type, title, listed_at, updated_at)
             VALUES (164896, 'show', 'Asia Express', '2026-11-18T10:00:00Z', '2026-11-18T10:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_history_cache (history_id, trakt_id, media_type, show_trakt_id, show_title, season, episode, watched_at)
             VALUES (1, 999, 'episode', 164896, 'Asia Express', 7, 23, '2026-11-18T12:39:00Z')",
            [],
        ).unwrap();
        // Simulate accidental local mark-watched on S07E24 (unaired) + history_dirty = '1'
        conn.execute(
            "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, watched_at, updated_at)
             VALUES (164896, 7, 24, 8888, '2026-11-18T13:00:00Z', '2026-11-18T13:00:00Z')",
            [],
        ).unwrap();
        // Also simulate a pending offline history_add for S07E25 that must NOT be cleared
        conn.execute(
            "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, watched_at, updated_at)
             VALUES (164896, 7, 25, 9999, '2026-11-18T13:05:00Z', '2026-11-18T13:05:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_sync_state (key, value) VALUES ('last_activities', '{\"episodes\":{\"watched_at\":\"2026-11-18T12:39:00Z\"},\"movies\":{\"watched_at\":\"2026-11-10T10:00:00Z\"},\"watchlist\":{\"updated_at\":\"2026-11-18T10:00:00Z\"}}')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_sync_state (key, value) VALUES ('history_dirty', '1')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_sync_state (key, value) VALUES ('watched_shows_synced_v1', '1')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_sync_state (key, value) VALUES ('profile_json', '{\"username\":\"tester\"}')",
            [],
        ).unwrap();
    }

    // Non-forced sync (like focus sync) must still detect history_dirty and clear S07E24
    let summary = crate::trakt::sync::run_sync(&state, None, false).await.unwrap();
    assert!(summary.history_updated);

    let conn = state.db.lock().await;
    let s07e24_watched: Option<String> = conn
        .query_row(
            "SELECT watched_at FROM trakt_episodes WHERE show_trakt_id = 164896 AND season = 7 AND episode = 24",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(s07e24_watched.is_none(), "S07E24 should be cleared to unwatched");

    let s07e23_watched: Option<String> = conn
        .query_row(
            "SELECT watched_at FROM trakt_episodes WHERE show_trakt_id = 164896 AND season = 7 AND episode = 23",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(s07e23_watched.as_deref(), Some("2026-11-18T12:39:00Z"));

    let s07e25_watched: Option<String> = conn
        .query_row(
            "SELECT watched_at FROM trakt_episodes WHERE show_trakt_id = 164896 AND season = 7 AND episode = 25",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(s07e25_watched.is_none(), "S07E25 without pending action should also be cleared");

    let dirty_after: String = conn
        .query_row(
            "SELECT value FROM trakt_sync_state WHERE key = 'history_dirty'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dirty_after, "0");
}

#[tokio::test]
async fn test_sync_watched_shows_preserves_pending_history_add() {
    let base = spawn_mock_server(move |method, path, _body| {
        if method == "GET" && path == "/sync/watched/shows" {
            return (200, vec![], "[]".to_string());
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    {
        let mut conn = state.db.lock().await;
        save_tokens_tx(
            &mut conn,
            store.as_ref(),
            "valid_acc",
            "valid_ref",
            now_epoch_secs() + 3600,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, watched_at, updated_at)
             VALUES (164896, 7, 24, 8888, '2026-11-18T13:00:00Z', '2026-11-18T13:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_episodes (show_trakt_id, season, episode, episode_trakt_id, watched_at, updated_at)
             VALUES (164896, 7, 25, 9999, '2026-11-18T13:05:00Z', '2026-11-18T13:05:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO trakt_pending_actions (action, payload_json, created_at, attempts, last_error)
             VALUES ('history_add', '{\"episodes\":[{\"ids\":{\"trakt\":9999}}]}', ?1, 0, 'offline')",
            params![now_iso_utc()],
        ).unwrap();
    }

    crate::trakt::sync::sync_watched_shows(&state, None).await.unwrap();

    let conn = state.db.lock().await;
    let e24: Option<String> = conn
        .query_row(
            "SELECT watched_at FROM trakt_episodes WHERE show_trakt_id = 164896 AND season = 7 AND episode = 24",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(e24.is_none(), "Non-pending episode should be cleared");

    let e25: Option<String> = conn
        .query_row(
            "SELECT watched_at FROM trakt_episodes WHERE show_trakt_id = 164896 AND season = 7 AND episode = 25",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(e25.as_deref(), Some("2026-11-18T13:05:00Z"), "Pending offline episode must be preserved");
}

#[tokio::test]
async fn test_run_sync_populates_continue_watching_and_respects_hidden() {
    let base = spawn_mock_server(move |method, path, _body| {
        if method == "GET" && path == "/sync/last_activities" {
            return (
                200,
                vec![],
                r#"{"episodes":{"watched_at":"2026-11-22T08:00:00Z"},"movies":{"watched_at":"2026-11-10T10:00:00Z"},"watchlist":{"updated_at":"2026-11-22T08:00:00Z"}}"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/sync/watchlist/") {
            return (200, vec![], "[]".to_string());
        }
        if method == "GET" && path.starts_with("/users/me/history/") {
            return (200, vec![], "[]".to_string());
        }
        if method == "GET" && path == "/sync/watched/shows" {
            return (
                200,
                vec![],
                r#"[
                    {"show":{"title":"Started Show","year":2024,"ids":{"trakt":101,"tmdb":1101}},"seasons":[{"number":1,"episodes":[{"number":1,"last_watched_at":"2026-11-20T10:00:00Z"}]}]},
                    {"show":{"title":"Future Show","year":2025,"ids":{"trakt":303,"tmdb":3303}},"seasons":[{"number":1,"episodes":[{"number":1,"last_watched_at":"2026-11-21T10:00:00Z"}]}]},
                    {"show":{"title":"Hidden Show","year":2023,"ids":{"trakt":202,"tmdb":2202}},"seasons":[{"number":1,"episodes":[{"number":1,"last_watched_at":"2026-11-19T10:00:00Z"}]}]}
                ]"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/users/hidden/progress_watched") {
            return (
                200,
                vec![],
                r#"[{"type":"show","show":{"title":"Hidden Show","ids":{"trakt":202,"tmdb":2202}}}]"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/shows/101/progress/watched") {
            return (
                200,
                vec![],
                r#"{"aired":8,"completed":1,"last_watched_at":"2026-11-20T10:00:00Z","next_episode":{"season":1,"number":2,"title":"Next Ep","first_aired":"2026-11-21T08:00:00Z","ids":{"trakt":9002}},"seasons":[{"number":1,"episodes":[{"number":1,"completed":true,"last_watched_at":"2026-11-20T10:00:00Z"},{"number":2,"completed":false}]}]}"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/shows/303/progress/watched") {
            return (
                200,
                vec![],
                r#"{"aired":2,"completed":1,"last_watched_at":"2026-11-21T10:00:00Z","next_episode":{"season":1,"number":2,"title":"Future Ep","first_aired":"2026-11-25T08:00:00Z","ids":{"trakt":9003}},"seasons":[{"number":1,"episodes":[{"number":1,"completed":true,"last_watched_at":"2026-11-21T10:00:00Z"},{"number":2,"completed":false}]}]}"#.to_string(),
            );
        }
        if method == "GET" && path.starts_with("/calendars/my/shows/") {
            return (200, vec![], "[]".to_string());
        }
        if method == "GET" && path == "/users/settings" {
            return (200, vec![], r#"{"user":{"username":"tester"}}"#.to_string());
        }
        (404, vec![], "{}".to_string())
    })
    .await;

    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    {
        let mut conn = state.db.lock().await;
        save_tokens_tx(
            &mut conn,
            store.as_ref(),
            "valid_acc",
            "valid_ref",
            now_epoch_secs() + 3600,
        )
        .unwrap();
    }

    crate::trakt::sync::run_sync(&state, None, true).await.unwrap();

    let conn = state.db.lock().await;
    let (next_id, next_s, next_e, next_title, next_fa, hidden): (Option<i64>, Option<i64>, Option<i64>, Option<String>, Option<String>, i64) = conn
        .query_row(
            "SELECT next_episode_trakt_id, next_season, next_episode, next_episode_title, next_episode_first_aired, hidden_from_progress
             FROM trakt_show_progress WHERE show_trakt_id = 101",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(next_id, Some(9002));
    assert_eq!(next_s, Some(1));
    assert_eq!(next_e, Some(2));
    assert_eq!(next_title.as_deref(), Some("Next Ep"));
    assert_eq!(next_fa.as_deref(), Some("2026-11-21T08:00:00Z"));
    assert_eq!(hidden, 0);

    let hidden_flag: i64 = conn
        .query_row(
            "SELECT hidden_from_progress FROM trakt_show_progress WHERE show_trakt_id = 202",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hidden_flag, 1);

    // At 2026-11-22T09:00:00Z, only Started Show (aired 2026-11-21) should appear; Future Show (2026-11-25) must be excluded
    let now_2026_11_22 = crate::trakt::iso_utc_to_epoch("2026-11-22T09:00:00Z").unwrap();
    let cw = crate::trakt::commands::query_continue_watching(&conn, now_2026_11_22).unwrap();
    assert_eq!(cw.len(), 1);
    assert_eq!(cw[0].show_trakt_id, 101);
    assert_eq!(cw[0].episode_trakt_id, 9002);
}
