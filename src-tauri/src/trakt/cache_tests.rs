//! Tests of the Trakt HTTP cache layer: pagination info, ETag handling, housekeeping and the shared
//! calendar start date. They run against a local mock server and never need a Trakt login.

use crate::trakt::calendar::fetch_and_cache_global_calendar;
use crate::trakt::client::{
    prune_http_cache, prune_stale_calendar_keys, trakt_get_cached, HTTP_CACHE_PRUNE_GRACE_SECS,
};
use crate::trakt::sync::{
    calendar_start_for_sync, local_yesterday_ymd, remember_calendar_start, resolve_calendar_start,
};
use crate::trakt::tests::{make_test_state, spawn_mock_server};
use crate::trakt::{epoch_to_iso_utc, now_epoch_secs, now_iso_utc};
use crate::AppState;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const PAGED_PATH: &str = "/users/test-hidden?page=1";

async fn scalar(state: &AppState, sql: &str) -> i64 {
    state.db.lock().await.query_row(sql, [], |r| r.get(0)).unwrap()
}

async fn insert_cache_row(state: &AppState, key: &str, fetched_epoch: i64, ttl: i64, page_count: Option<i64>) {
    state
        .db
        .lock()
        .await
        .execute(
            "INSERT INTO trakt_http_cache (key, etag, body, fetched_at, ttl_seconds, page_count)
             VALUES (?1, NULL, '{}', ?2, ?3, ?4)",
            rusqlite::params![key, epoch_to_iso_utc(fetched_epoch), ttl, page_count],
        )
        .unwrap();
}

fn ymd_days_ago(days: i64) -> String {
    epoch_to_iso_utc(now_epoch_secs() - days * 86_400)[..10].to_string()
}

fn offline_state() -> AppState {
    make_test_state("http://127.0.0.1:1".to_string(), "http://127.0.0.1:1".to_string())
}

#[tokio::test]
async fn test_page_count_survives_fresh_cache_and_304() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let base = spawn_mock_server(move |method, path, _body| {
        if method == "GET" && path.starts_with("/users/test-hidden") {
            let n = h.fetch_add(1, Ordering::SeqCst);
            let hdrs = vec![
                ("ETag".to_string(), "\"v1\"".to_string()),
                ("X-Pagination-Page-Count".to_string(), "3".to_string()),
            ];
            if n == 0 {
                return (200, hdrs, "[1]".to_string());
            }
            return (304, hdrs, String::new());
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = make_test_state(base.clone(), base);

    let first = trakt_get_cached(&state, None, PAGED_PATH, false, 3600, false).await.unwrap();
    assert!(!first.from_cache);
    assert_eq!(first.page_count, Some(3));

    // Fresh cache hit: the pagination info must not be lost (it used to come back as None -> "1 page").
    let second = trakt_get_cached(&state, None, PAGED_PATH, false, 3600, false).await.unwrap();
    assert!(second.from_cache);
    assert_eq!(second.page_count, Some(3));
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    // Expired -> conditional request -> 304: still the stored page count.
    state
        .db
        .lock()
        .await
        .execute("UPDATE trakt_http_cache SET fetched_at = '2020-01-01T00:00:00Z'", [])
        .unwrap();
    let third = trakt_get_cached(&state, None, PAGED_PATH, false, 3600, false).await.unwrap();
    assert!(third.from_cache);
    assert_eq!(third.body, "[1]");
    assert_eq!(third.page_count, Some(3));
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_answer_without_pagination_header_has_no_page_count() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let base = spawn_mock_server(move |_m, _p, _b| {
        h.fetch_add(1, Ordering::SeqCst);
        (200, vec![], "{}".to_string())
    })
    .await;
    let state = make_test_state(base.clone(), base);

    let first = trakt_get_cached(&state, None, "/shows/1", false, 3600, false).await.unwrap();
    assert_eq!(first.page_count, None);
    let second = trakt_get_cached(&state, None, "/shows/1", false, 3600, false).await.unwrap();
    assert!(second.from_cache);
    assert_eq!(second.page_count, None);
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    // stored as "known: not paginated" (0), not as the legacy marker NULL
    assert_eq!(scalar(&state, "SELECT page_count FROM trakt_http_cache WHERE key = '/shows/1'").await, 0);
}

#[tokio::test]
async fn test_legacy_cache_row_without_page_count_is_refetched() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let base = spawn_mock_server(move |_m, _p, _b| {
        h.fetch_add(1, Ordering::SeqCst);
        (
            200,
            vec![("X-Pagination-Page-Count".to_string(), "2".to_string())],
            r#"["fresh"]"#.to_string(),
        )
    })
    .await;
    let state = make_test_state(base.clone(), base);

    // A row written by the previous version: fresh by TTL, but its pagination info is unknown.
    state
        .db
        .lock()
        .await
        .execute(
            "INSERT INTO trakt_http_cache (key, etag, body, fetched_at, ttl_seconds)
             VALUES (?1, '\"old\"', '[\"stale\"]', ?2, 3600)",
            rusqlite::params![PAGED_PATH, now_iso_utc()],
        )
        .unwrap();

    let resp = trakt_get_cached(&state, None, PAGED_PATH, false, 3600, false).await.unwrap();
    assert!(!resp.from_cache, "a legacy row must not be served");
    assert_eq!(resp.body, r#"["fresh"]"#);
    assert_eq!(resp.page_count, Some(2));
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        scalar(&state, &format!("SELECT page_count FROM trakt_http_cache WHERE key = '{}'", PAGED_PATH)).await,
        2
    );
}

#[tokio::test]
async fn test_new_answer_without_etag_does_not_keep_the_old_etag() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let base = spawn_mock_server(move |_m, _p, _b| {
        if h.fetch_add(1, Ordering::SeqCst) == 0 {
            return (200, vec![("ETag".to_string(), "\"v1\"".to_string())], "\"A\"".to_string());
        }
        (200, vec![], "\"B\"".to_string()) // new content, no ETag header
    })
    .await;
    let state = make_test_state(base.clone(), base);

    trakt_get_cached(&state, None, "/shows/9", false, 3600, false).await.unwrap();
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key = '/shows/9' AND etag = '\"v1\"'").await,
        1
    );

    state
        .db
        .lock()
        .await
        .execute("UPDATE trakt_http_cache SET fetched_at = '2020-01-01T00:00:00Z'", [])
        .unwrap();
    let second = trakt_get_cached(&state, None, "/shows/9", false, 3600, false).await.unwrap();
    assert_eq!(second.body, "\"B\"");
    // The body is new; the ETag of the old body must be gone (it used to survive via COALESCE).
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key = '/shows/9' AND etag IS NULL").await,
        1
    );
}

#[tokio::test]
async fn test_prune_removes_only_rows_expired_beyond_the_grace_period() {
    let state = offline_state();
    let now = now_epoch_secs();
    let grace = HTTP_CACHE_PRUNE_GRACE_SECS;

    insert_cache_row(&state, "/fresh", now - 60, 3600, Some(0)).await;
    insert_cache_row(&state, "/expired-inside-grace", now - 86_400, 3600, Some(0)).await; // expired 23 h ago
    insert_cache_row(&state, "/expired-beyond-grace", now - 3600 - grace - 600, 3600, Some(0)).await;
    insert_cache_row(&state, "/old-but-long-ttl", now - 10 * 86_400, 30 * 86_400, Some(0)).await; // still fresh

    let deleted = {
        let conn = state.db.lock().await;
        prune_http_cache(&conn)
    };
    assert_eq!(deleted, 1);

    let left: Vec<String> = {
        let conn = state.db.lock().await;
        let mut stmt = conn.prepare("SELECT key FROM trakt_http_cache ORDER BY key").unwrap();
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
        rows.flatten().collect()
    };
    assert_eq!(left, vec!["/expired-inside-grace", "/fresh", "/old-but-long-ttl"]);

    // idempotent
    let conn = state.db.lock().await;
    assert_eq!(prune_http_cache(&conn), 0);
}

#[tokio::test]
async fn test_prune_stale_calendar_keys_keeps_only_the_current_key_of_that_calendar() {
    let state = offline_state();
    let now = now_epoch_secs();
    insert_cache_row(&state, "/calendars/my/shows/2026-11-19/31?extended=full", now, 7200, Some(0)).await;
    insert_cache_row(&state, "/calendars/my/shows/2026-11-20/31?extended=full", now, 7200, Some(0)).await;
    insert_cache_row(&state, "/calendars/all/shows/2026-11-19/14?extended=full", now, 7200, Some(0)).await;
    insert_cache_row(&state, "/shows/1", now, 7200, Some(0)).await;

    let deleted = {
        let conn = state.db.lock().await;
        prune_stale_calendar_keys(&conn, "/calendars/my/shows/", "/calendars/my/shows/2026-11-20/31?extended=full")
    };
    assert_eq!(deleted, 1);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key LIKE '/calendars/my/%'").await, 1);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key LIKE '/calendars/all/%'").await, 1);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key = '/shows/1'").await, 1);
}

#[tokio::test]
async fn test_global_calendar_download_drops_the_previous_days_body() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let base = spawn_mock_server(move |method, path, _b| {
        if method == "GET" && path.starts_with("/calendars/all/shows/") {
            h.fetch_add(1, Ordering::SeqCst);
            return (200, vec![], "[]".to_string());
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = make_test_state(base.clone(), base);
    insert_cache_row(&state, "/shows/1", now_epoch_secs(), 3600, Some(0)).await;

    fetch_and_cache_global_calendar(&state, None, "2026-11-20", false).await.unwrap();
    fetch_and_cache_global_calendar(&state, None, "2026-11-21", false).await.unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key LIKE '/calendars/all/shows/%'").await, 1);
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key LIKE '/calendars/all/shows/2026-11-21/%'").await,
        1
    );
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key = '/shows/1'").await, 1);
}

#[tokio::test]
async fn test_calendar_start_date_is_shared_between_ui_and_sync() {
    let state = offline_state();
    let guess = local_yesterday_ymd();

    // nothing remembered yet -> the UTC based guess
    {
        let conn = state.db.lock().await;
        assert_eq!(calendar_start_for_sync(&conn), guess);
    }

    // The UI asks with its own local date (one day earlier than the guess): remembered and reused by the sync.
    let ui_date = ymd_days_ago(2);
    assert_eq!(resolve_calendar_start(&state, Some(ui_date.clone())).await, ui_date);
    {
        let conn = state.db.lock().await;
        assert_eq!(calendar_start_for_sync(&conn), ui_date);
    }

    // No explicit request (e.g. a command without start_date) resolves to the same shared date.
    assert_eq!(resolve_calendar_start(&state, None).await, ui_date);

    // Something that is not a date is passed through as before, but never remembered.
    assert_eq!(
        resolve_calendar_start(&state, Some("not-a-date-at-all".to_string())).await,
        "not-a-date-at-all"
    );
    {
        let conn = state.db.lock().await;
        assert_eq!(calendar_start_for_sync(&conn), ui_date);
    }

    // An implausible remembered date (app closed for days / clock jump) is ignored by the sync.
    for implausible in [ymd_days_ago(6), ymd_days_ago(-4)] {
        let conn = state.db.lock().await;
        remember_calendar_start(&conn, &implausible);
        assert_eq!(calendar_start_for_sync(&conn), guess, "{} must fall back to the guess", implausible);
    }
}

#[tokio::test]
async fn test_ui_and_sync_use_one_calendar_download() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let base = spawn_mock_server(move |method, path, _b| {
        if method == "GET" && path.starts_with("/calendars/all/shows/") {
            h.fetch_add(1, Ordering::SeqCst);
            return (200, vec![], "[]".to_string());
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = make_test_state(base.clone(), base);

    // The UI requests the calendar with its local date ...
    let ui_start = resolve_calendar_start(&state, Some(ymd_days_ago(2))).await;
    fetch_and_cache_global_calendar(&state, None, &ui_start, false).await.unwrap();

    // ... and the background sync now asks for exactly the same window: no second download.
    let sync_start = {
        let conn = state.db.lock().await;
        calendar_start_for_sync(&conn)
    };
    assert_eq!(sync_start, ui_start);
    fetch_and_cache_global_calendar(&state, None, &sync_start, false).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}
