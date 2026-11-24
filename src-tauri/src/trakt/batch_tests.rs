//! Tests of the F6 sync changes: batched offline queue, per-stage activity stamps, hidden list refresh.

use crate::trakt::auth::save_tokens_tx;
use crate::trakt::secret::default_secret_store;
use crate::trakt::sync::flush_pending_actions;
use crate::trakt::tests::{make_test_state, spawn_mock_server};
use crate::trakt::{now_epoch_secs, now_iso_utc};
use crate::AppState;
use rusqlite::params;
use std::sync::{Arc, Mutex as StdMutex};

type Posts = Arc<StdMutex<Vec<(String, String)>>>;

async fn logged_in_state(base: String) -> AppState {
    let state = make_test_state(base.clone(), base);
    let store = default_secret_store();
    let mut conn = state.db.lock().await;
    save_tokens_tx(&mut conn, store.as_ref(), "valid_acc", "valid_ref", now_epoch_secs() + 3600).unwrap();
    drop(conn);
    state
}

async fn queue(state: &AppState, action: &str, payload: &str) {
    state
        .db
        .lock()
        .await
        .execute(
            "INSERT INTO trakt_pending_actions (action, payload_json, created_at, attempts, last_error)
             VALUES (?1, ?2, ?3, 0, 'offline')",
            params![action, payload, now_iso_utc()],
        )
        .unwrap();
}

async fn remaining(state: &AppState) -> i64 {
    state
        .db
        .lock()
        .await
        .query_row("SELECT COUNT(*) FROM trakt_pending_actions", [], |r| r.get(0))
        .unwrap()
}

fn recording_server(reject_batches: bool, reject_item: Option<i64>) -> (Posts, impl Fn(String, String, String) -> (u16, Vec<(String, String)>, String) + Send + Sync + 'static) {
    let posts: Posts = Arc::new(StdMutex::new(Vec::new()));
    let p = posts.clone();
    let handler = move |method: String, path: String, body: String| {
        if method != "POST" {
            return (404, vec![], "{}".to_string());
        }
        p.lock().unwrap().push((path, body.clone()));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let count: usize = ["episodes", "movies", "shows"]
            .iter()
            .map(|k| v.get(*k).and_then(|a| a.as_array()).map_or(0, |a| a.len()))
            .sum();
        if reject_batches && count > 1 {
            return (422, vec![], r#"{"error":"bad batch"}"#.to_string());
        }
        if let Some(bad) = reject_item {
            if body.contains(&format!("\"trakt\":{}", bad)) {
                return (422, vec![], r#"{"error":"bad item"}"#.to_string());
            }
        }
        (201, vec![], r#"{"added":{"episodes":1,"movies":0}}"#.to_string())
    };
    (posts, handler)
}

#[tokio::test]
async fn test_ten_pending_history_adds_are_one_post() {
    let (posts, handler) = recording_server(false, None);
    let base = spawn_mock_server(handler).await;
    let state = logged_in_state(base).await;
    for id in 1..=10 {
        queue(&state, "history_add", &format!(r#"{{"episodes":[{{"watched_at":"2026-11-20T10:00:00Z","ids":{{"trakt":{id}}}}}]}}"#)).await;
    }

    let flushed = flush_pending_actions(&state, None).await.unwrap();
    assert_eq!(flushed, 10);
    let posts = posts.lock().unwrap();
    assert_eq!(posts.len(), 1, "10 queued adds must be a single request");
    let body: serde_json::Value = serde_json::from_str(&posts[0].1).unwrap();
    assert_eq!(body["episodes"].as_array().unwrap().len(), 10);
    drop(posts);
    assert_eq!(remaining(&state).await, 0);
}

#[tokio::test]
async fn test_mixed_add_remove_keeps_their_order() {
    let (posts, handler) = recording_server(false, None);
    let base = spawn_mock_server(handler).await;
    let state = logged_in_state(base).await;
    queue(&state, "watchlist_add", r#"{"shows":[{"ids":{"trakt":1}}]}"#).await;
    queue(&state, "watchlist_add", r#"{"shows":[{"ids":{"trakt":2}}]}"#).await;
    queue(&state, "watchlist_remove", r#"{"shows":[{"ids":{"trakt":1}}]}"#).await;
    queue(&state, "watchlist_add", r#"{"shows":[{"ids":{"trakt":1}}]}"#).await;

    assert_eq!(flush_pending_actions(&state, None).await.unwrap(), 4);
    let paths: Vec<String> = posts.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(
        paths,
        vec!["/sync/watchlist", "/sync/watchlist/remove", "/sync/watchlist"],
        "add+add | remove | add: runs of one action only, never reordered"
    );
}

#[tokio::test]
async fn test_batches_are_capped_at_100_items() {
    let (posts, handler) = recording_server(false, None);
    let base = spawn_mock_server(handler).await;
    let state = logged_in_state(base).await;
    for id in 1..=101 {
        queue(&state, "history_add", &format!(r#"{{"episodes":[{{"ids":{{"trakt":{id}}}}}]}}"#)).await;
    }
    assert_eq!(flush_pending_actions(&state, None).await.unwrap(), 101);
    assert_eq!(posts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_rejected_batch_falls_back_to_single_posts_and_drops_only_the_bad_item() {
    let (posts, handler) = recording_server(true, Some(2));
    let base = spawn_mock_server(handler).await;
    let state = logged_in_state(base).await;
    for id in 1..=3 {
        queue(&state, "history_add", &format!(r#"{{"episodes":[{{"ids":{{"trakt":{id}}}}}]}}"#)).await;
    }

    let flushed = flush_pending_actions(&state, None).await.unwrap();
    assert_eq!(flushed, 2, "items 1 and 3 go through, the refused item 2 is dropped");
    assert_eq!(posts.lock().unwrap().len(), 1 + 3, "1 refused batch + 3 single retries");
    assert_eq!(remaining(&state).await, 0);
}

#[tokio::test]
async fn test_transient_error_keeps_the_whole_batch_queued() {
    // nothing listens on this port -> network error
    let state = logged_in_state("http://127.0.0.1:1".to_string()).await;
    queue(&state, "history_add", r#"{"episodes":[{"ids":{"trakt":1}}]}"#).await;
    queue(&state, "history_add", r#"{"episodes":[{"ids":{"trakt":2}}]}"#).await;

    assert!(flush_pending_actions(&state, None).await.is_err());
    assert_eq!(remaining(&state).await, 2);
    let attempts: i64 = state
        .db
        .lock()
        .await
        .query_row("SELECT MIN(attempts) FROM trakt_pending_actions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attempts, 1);
}
