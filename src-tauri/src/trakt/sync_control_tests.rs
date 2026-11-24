//! Tests of the sync button logic: activity based sync (header) vs. full sync (Settings), debounce, double click.
//! The sync itself is replaced by a counting fake, so no Trakt login or network is involved.

use crate::trakt::sync::{
    manual_sync_in, mark_sync_done, resolve_force, set_sync_state_val, SyncSummary,
    MANUAL_SYNC_DEBOUNCE_SECS,
};
use crate::trakt::tests::make_test_state;
use crate::AppState;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const WINDOW: Duration = Duration::from_secs(MANUAL_SYNC_DEBOUNCE_SECS);

fn offline_state() -> AppState {
    make_test_state("http://127.0.0.1:1".to_string(), "http://127.0.0.1:1".to_string())
}

fn ok_summary() -> SyncSummary {
    SyncSummary {
        watchlist_updated: true,
        history_updated: true,
        calendar_updated: true,
        pending_flushed: 0,
        last_sync_ok_at: "2026-11-24T10:00:00Z".to_string(),
        throttled: false,
    }
}

async fn click(state: &AppState, force: bool, window: Duration, calls: &Arc<AtomicUsize>) -> Result<SyncSummary, String> {
    let c = calls.clone();
    manual_sync_in(state, force, window, move || async move {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(ok_summary())
    })
    .await
}

#[test]
fn test_trakt_sync_command_defaults_to_activity_based_sync() {
    // It used to default to a full re-download (force = true) whenever the frontend passed no value.
    assert!(!resolve_force(None));
    assert!(!resolve_force(Some(false)));
    assert!(resolve_force(Some(true)));
}

#[tokio::test]
async fn test_repeated_header_click_is_throttled() {
    let state = offline_state();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let conn = state.db.lock().await;
        set_sync_state_val(&conn, "last_sync_ok_at", "2026-11-24T09:59:00Z").unwrap();
    }

    let first = click(&state, false, WINDOW, &calls).await.unwrap();
    assert!(!first.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let second = click(&state, false, WINDOW, &calls).await.unwrap();
    assert!(second.throttled, "a second click within the window must not start another sync");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // the answer still carries the stored last-sync time, so the UI status stays correct
    assert_eq!(second.last_sync_ok_at, "2026-11-24T09:59:00Z");
    assert!(!second.watchlist_updated && !second.history_updated && !second.calendar_updated);
}

#[tokio::test]
async fn test_full_sync_is_never_throttled() {
    let state = offline_state();
    let calls = Arc::new(AtomicUsize::new(0));

    click(&state, false, WINDOW, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Settings > Trakt.tv "full sync" right after a normal sync: runs
    let full = click(&state, true, WINDOW, &calls).await.unwrap();
    assert!(!full.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let full_again = click(&state, true, WINDOW, &calls).await.unwrap();
    assert!(!full_again.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // and a full sync makes a normal one pointless for a minute
    let normal = click(&state, false, WINDOW, &calls).await.unwrap();
    assert!(normal.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_failed_sync_does_not_start_the_debounce_window() {
    let state = offline_state();
    let calls = Arc::new(AtomicUsize::new(0));

    let c = calls.clone();
    let failed = manual_sync_in(&state, false, WINDOW, move || async move {
        c.fetch_add(1, Ordering::SeqCst);
        Err::<SyncSummary, String>("TRAKT_OFFLINE".to_string())
    })
    .await;
    assert!(failed.is_err());

    // back online: the retry must run immediately instead of being swallowed as "too recent"
    let retry = click(&state, false, WINDOW, &calls).await.unwrap();
    assert!(!retry.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_double_click_runs_one_sync() {
    let state = offline_state();
    let calls = Arc::new(AtomicUsize::new(0));

    let slow = |c: Arc<AtomicUsize>| async move {
        c.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(60)).await;
        Ok::<SyncSummary, String>(ok_summary())
    };
    let c1 = calls.clone();
    let c2 = calls.clone();
    let (a, b) = tokio::join!(
        manual_sync_in(&state, false, WINDOW, || slow(c1)),
        manual_sync_in(&state, false, WINDOW, || slow(c2)),
    );
    let (a, b) = (a.unwrap(), b.unwrap());

    assert_eq!(calls.load(Ordering::SeqCst), 1, "the second click must wait and then be throttled");
    assert_eq!([a.throttled, b.throttled].iter().filter(|t| **t).count(), 1);
}

#[tokio::test]
async fn test_a_recent_focus_sync_also_satisfies_the_header_button() {
    let state = offline_state();
    let calls = Arc::new(AtomicUsize::new(0));

    mark_sync_done(&state); // what trakt_on_window_focus does after a successful background sync
    let res = click(&state, false, WINDOW, &calls).await.unwrap();
    assert!(res.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    // an explicit full sync still runs
    click(&state, true, WINDOW, &calls).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_debounce_window_expires() {
    let state = offline_state();
    let calls = Arc::new(AtomicUsize::new(0));
    let short = Duration::from_millis(40);

    click(&state, false, short, &calls).await.unwrap();
    assert!(click(&state, false, short, &calls).await.unwrap().throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    tokio::time::sleep(Duration::from_millis(80)).await;
    let after = click(&state, false, short, &calls).await.unwrap();
    assert!(!after.throttled);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
