//! Tests of the TMDB layer against a local mock server (see `TmdbRuntime::with_base`).
//! They pin down the cache rules: only successful answers are cached, automatic "no match" rows expire,
//! the user's manual rows never do, and id-only lookups cannot collide on the title key.

use crate::trakt::tests::{make_test_state, spawn_mock_server};
use crate::{
    refresh_stale_tmdb, clean_title, clear_automatic_tmdb_cache, get_tmdb_by_id, get_tmdb_by_id_inner,
    get_tmdb_details_inner, store_manual_none, AppState, TmdbRuntime,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use crate::trakt::commands::{prefetch_tmdb_seasons, read_tmdb_season_cache, tmdb_season_cache_key};

type Log = Arc<StdMutex<Vec<String>>>;

fn hits(log: &Log, prefix: &str) -> usize {
    log.lock().unwrap().iter().filter(|p| p.starts_with(prefix)).count()
}

async fn tmdb_state(base: String) -> AppState {
    let mut state = make_test_state("http://127.0.0.1:1".to_string(), "http://127.0.0.1:1".to_string());
    state.tmdb = TmdbRuntime::with_base(base, Duration::from_millis(1));
    state
        .db
        .lock()
        .await
        .execute("INSERT INTO settings (key, value) VALUES ('tmdb_api_key', 'test-key')", [])
        .unwrap();
    state
}

async fn scalar(state: &AppState, sql: &str) -> i64 {
    state.db.lock().await.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn tv_item(id: i64, name: &str) -> String {
    format!(
        r#"{{"id":{id},"media_type":"tv","name":"{name}","original_name":"{name}","original_language":"en","poster_path":"/p{id}.jpg","first_air_date":"2014-04-15","overview":"overview {id}","vote_average":8.1}}"#
    )
}

fn tv_item_lang(id: i64, name: &str, lang: &str) -> String {
    tv_item(id, name).replace("\"original_language\":\"en\"", &format!("\"original_language\":\"{lang}\""))
}
fn translations_body() -> String {
    r#"{"translations":[{"iso_639_1":"hu","data":{"name":"Fargo HU","overview":"magyar leírás"}},{"iso_639_1":"en","data":{"name":"Fargo","overview":"english overview"}}]}"#.to_string()
}

fn search_body(items: &[String]) -> String {
    format!(r#"{{"results":[{}]}}"#, items.join(","))
}

/// Details answer of `/{movie|tv}/{id}?append_to_response=translations,...` (translations embedded).
fn details_body(id: i64, media_type: &str, title: &str) -> String {
    let (title_key, date_key) = if media_type == "movie" {
        ("title", "release_date")
    } else {
        ("name", "first_air_date")
    };
    format!(
        r#"{{"id":{id},"{title_key}":"{title}","original_language":"en","poster_path":"/p{id}.jpg","{date_key}":"2021-10-22","overview":"o{id}","vote_average":7.0,"translations":{{"translations":[{{"iso_639_1":"hu","data":{{"{title_key}":"{title} HU","overview":"hu{id}"}}}}]}}}}"#
    )
}

#[tokio::test]
async fn test_search_429_is_error_and_not_cached() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let mode = Arc::new(AtomicUsize::new(0)); // 0 = always 429, 1 = healthy
    let (l, m) = (log.clone(), mode.clone());
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/search/multi") {
            if m.load(Ordering::SeqCst) == 0 {
                return (429, vec![("Retry-After".to_string(), "0".to_string())], "{}".to_string());
            }
            return (200, vec![], search_body(&[tv_item(1, "Fargo")]));
        }
        if path.contains("/translations") {
            return (200, vec![], translations_body());
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let res = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await;
    assert!(res.is_err(), "a 429 must surface as an error, not as 'not found'");
    assert_eq!(hits(&log, "/search/multi"), 3, "1 request + 2 Retry-After retries");
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await, 0, "nothing may be cached on 429");

    // TMDB recovers -> the very next lookup succeeds (it would stay a permanent miss with a poisoned cache).
    mode.store(1, Ordering::SeqCst);
    let ok = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap();
    assert_eq!(ok.expect("match expected").tmdb_id, 1);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE media_type = 'tv'").await, 1);
}

#[tokio::test]
async fn test_search_5xx_retries_once_and_is_not_cached() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_m, path, _b| {
        l.lock().unwrap().push(path);
        (500, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let res = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await;
    assert!(res.is_err());
    assert_eq!(hits(&log, "/search/multi"), 2, "one retry after a 5xx");
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await, 0);
}

#[tokio::test]
async fn test_unauthorized_is_not_retried_and_not_cached() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_m, path, _b| {
        l.lock().unwrap().push(path);
        (401, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let err = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap_err();
    assert!(err.contains("Invalid TMDB API key"), "unexpected message: {}", err);
    assert_eq!(hits(&log, "/search/multi"), 1);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await, 0);
}

#[tokio::test]
async fn test_empty_result_creates_expiring_negative_row() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let mode = Arc::new(AtomicUsize::new(0)); // 0 = no results, 1 = a result exists now
    let (l, m) = (log.clone(), mode.clone());
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/search/multi") {
            if m.load(Ordering::SeqCst) == 0 {
                return (200, vec![], search_body(&[]));
            }
            return (200, vec![], search_body(&[tv_item(7, "Fargo")]));
        }
        if path.contains("/translations") {
            return (200, vec![], translations_body());
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    // 1st lookup: hu-HU search + en-US fallback, both empty -> automatic negative row
    assert!(get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap().is_none());
    assert_eq!(hits(&log, "/search/multi"), 2);
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE media_type = 'none' AND is_manual = 0").await,
        1
    );

    // 2nd lookup: served from the negative cache, no request
    assert!(get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap().is_none());
    assert_eq!(hits(&log, "/search/multi"), 2);

    // Younger than the TTL (6 days): still cached
    state.db.lock().await
        .execute("UPDATE tmdb_cache SET updated_at = datetime('now','localtime','-6 days')", [])
        .unwrap();
    assert!(get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap().is_none());
    assert_eq!(hits(&log, "/search/multi"), 2);

    // Older than the TTL (8 days): searched again - and now TMDB has the title
    state.db.lock().await
        .execute("UPDATE tmdb_cache SET updated_at = datetime('now','localtime','-8 days')", [])
        .unwrap();
    mode.store(1, Ordering::SeqCst);
    let found = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap();
    assert_eq!(found.expect("expired negative must be re-searched").tmdb_id, 7);
    assert_eq!(hits(&log, "/search/multi"), 3);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE media_type = 'none'").await, 0);
}

#[tokio::test]
async fn test_manual_none_marker_never_expires() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_m, path, _b| {
        l.lock().unwrap().push(path);
        (200, vec![], search_body(&[tv_item(9, "Fargo")]))
    })
    .await;
    let state = tmdb_state(base).await;

    let key = clean_title("Fargo");
    {
        let conn = state.db.lock().await;
        store_manual_none(&conn, &key, "hu-HU").unwrap();
        conn.execute("UPDATE tmdb_cache SET updated_at = '2020-01-01 00:00:00'", []).unwrap();
    }

    let res = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap();
    assert!(res.is_none(), "the user's 'not this title' decision must survive the TTL");
    assert_eq!(hits(&log, "/search/multi"), 0, "no request may be made for a manual marker");
}

#[tokio::test]
async fn test_translations_failure_returns_details_but_is_not_cached() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let mode = Arc::new(AtomicUsize::new(0)); // 0 = translations fail, 1 = ok
    let (l, m) = (log.clone(), mode.clone());
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/search/multi") {
            return (200, vec![], search_body(&[tv_item_lang(1399, "Fargo", "de")]));
        }
        if path.contains("/translations") {
            if m.load(Ordering::SeqCst) == 0 {
                return (500, vec![], "{}".to_string());
            }
            return (200, vec![], translations_body());
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let partial = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap();
    let partial = partial.expect("the UI still gets the search data");
    assert_eq!(partial.tmdb_id, 1399);
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await,
        0,
        "incomplete details (no translations) must not be cached"
    );

    mode.store(1, Ordering::SeqCst);
    let full = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap().unwrap();
    assert_eq!(full.hu_title.as_deref(), Some("Fargo HU"));
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE tmdb_id = 1399").await, 1);

    // and from now on it is served from the cache
    let search_hits = hits(&log, "/search/multi");
    let again = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap().unwrap();
    assert_eq!(again.hu_title.as_deref(), Some("Fargo HU"));
    assert_eq!(hits(&log, "/search/multi"), search_hits);
}

#[tokio::test]
async fn test_same_title_movie_and_tv_are_both_cached_by_id() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/movie/100") {
            return (200, vec![], details_body(100, "movie", "Dune"));
        }
        if path.starts_with("/tv/200") {
            return (200, vec![], details_body(200, "tv", "Dune"));
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let movie = get_tmdb_by_id(&state, 100, "movie", "hu-HU", None, false).await.unwrap();
    let tv = get_tmdb_by_id(&state, 200, "tv", "hu-HU", None, false).await.unwrap();
    assert_eq!(movie.tmdb_id, 100);
    assert_eq!(tv.tmdb_id, 200);
    assert_eq!(hits(&log, "/movie/100"), 1);
    assert_eq!(hits(&log, "/tv/200"), 1);

    // Both rows exist (with the old title key the second INSERT OR IGNORE was silently dropped)
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE query_title LIKE '~id:%'").await, 2);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE query_title = 'dune'").await, 0);

    // The second lookups are cache hits -> no new requests, correct item each
    let movie2 = get_tmdb_by_id(&state, 100, "movie", "hu-HU", None, false).await.unwrap();
    let tv2 = get_tmdb_by_id(&state, 200, "tv", "hu-HU", None, false).await.unwrap();
    assert_eq!((movie2.tmdb_id, tv2.tmdb_id), (100, 200));
    assert_eq!(hits(&log, "/movie/100"), 1);
    assert_eq!(hits(&log, "/tv/200"), 1);
}

#[tokio::test]
async fn test_by_id_404_and_transient_errors_are_not_cached() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/tv/404") {
            return (404, vec![], "{}".to_string());
        }
        (500, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    assert!(get_tmdb_by_id(&state, 404, "tv", "hu-HU", None, false).await.is_err());
    assert_eq!(hits(&log, "/tv/404"), 1, "404 is not retried");
    assert!(get_tmdb_by_id(&state, 500, "tv", "hu-HU", None, false).await.is_err());
    assert_eq!(hits(&log, "/tv/500"), 2, "5xx is retried once");
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await, 0);
}

#[tokio::test]
async fn test_clear_cache_keeps_manual_rows() {
    let base = spawn_mock_server(move |_method, path, _body| {
        if path.starts_with("/tv/1") {
            return (200, vec![], details_body(1, "tv", "Auto Show"));
        }
        if path.starts_with("/tv/2") {
            return (200, vec![], details_body(2, "tv", "Manual Show"));
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    // automatic id row + automatic negative
    get_tmdb_by_id(&state, 1, "tv", "hu-HU", None, false).await.unwrap();
    state.db.lock().await
        .execute(
            "INSERT INTO tmdb_cache (query_title, lang, media_type, is_manual, updated_at) VALUES ('auto miss', 'hu-HU', 'none', 0, datetime('now','localtime'))",
            [],
        )
        .unwrap();
    // manual match (set_tmdb_match path) + manual 'not this title' marker (clear_tmdb_match path)
    get_tmdb_by_id_inner(&state, 2, "tv", "hu-HU", Some("my title"), true, true).await.unwrap();
    {
        let conn = state.db.lock().await;
        store_manual_none(&conn, "not that one", "hu-HU").unwrap();
    }
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await, 4);

    {
        let conn = state.db.lock().await;
        clear_automatic_tmdb_cache(&conn).unwrap();
    }

    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache").await, 2);
    assert_eq!(
        scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE is_manual = 1 AND query_title IN ('my title','not that one')").await,
        2
    );
}

#[tokio::test]
async fn test_english_original_needs_no_translations_request() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/search/multi") {
            return (200, vec![], search_body(&[tv_item(1399, "Fargo")]));
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let d = get_tmdb_details_inner(&state, "Fargo".to_string(), None).await.unwrap().unwrap();
    assert_eq!(hits(&log, "/search/multi"), 1);
    assert_eq!(hits(&log, "/tv/"), 0, "no /translations request for an English original");
    assert_eq!(d.en_title.as_deref(), Some("Fargo"), "English title = original title");
    assert_eq!(d.overview.as_deref(), Some("overview 1399"), "overview comes from the hu-HU hit");
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE tmdb_id = 1399").await, 1, "and it is cached");
}

#[tokio::test]
async fn test_language_switch_is_served_from_the_other_language_row() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/tv/200") {
            // translations embedded: hu + en, both with an overview
            return (
                200,
                vec![],
                r#"{"id":200,"name":"Dune","original_language":"de","poster_path":"/p.jpg","first_air_date":"2021-10-22","overview":"o200","vote_average":7.0,"translations":{"translations":[{"iso_639_1":"hu","data":{"name":"Dűne","overview":"magyar"}},{"iso_639_1":"en","data":{"name":"Dune EN","overview":"english"}}]}}"#.to_string(),
            );
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let hu = get_tmdb_by_id(&state, 200, "tv", "hu-HU", None, false).await.unwrap();
    assert_eq!((hu.title.as_str(), hu.overview.as_deref()), ("Dune", Some("magyar")));
    assert_eq!(hits(&log, "/tv/200"), 1);

    let en = get_tmdb_by_id(&state, 200, "tv", "en-US", None, false).await.unwrap();
    assert_eq!(en.overview.as_deref(), Some("english"));
    assert_eq!(en.title, "Dune EN");
    assert_eq!(hits(&log, "/tv/200"), 1, "the switch must not hit the network");
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE tmdb_id = 200").await, 2, "stored for the next switch");

    // a row without overviews (written before the columns existed) cannot be derived -> network
    state.db.lock().await.execute("UPDATE tmdb_cache SET hu_overview = NULL, en_overview = NULL", []).unwrap();
    state.db.lock().await.execute("DELETE FROM tmdb_cache WHERE lang = 'en-US'", []).unwrap();
    get_tmdb_by_id(&state, 200, "tv", "en-US", None, false).await.unwrap();
    assert_eq!(hits(&log, "/tv/200"), 2);
}

#[tokio::test]
async fn test_eight_seasons_cost_one_tmdb_request() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/tv/55?") && path.contains("append_to_response=") {
            let seasons: Vec<String> = (1..=8)
                .map(|n| format!(r#""season/{n}":{{"name":"Season {n}","episodes":[{{"episode_number":1,"name":"S{n}E1"}}]}}"#))
                .collect();
            return (200, vec![], format!(r#"{{"id":55,"name":"Long Show",{}}}"#, seasons.join(",")));
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let seasons: Vec<i64> = (1..=8).collect();
    prefetch_tmdb_seasons(&state, "test-key", 55, "hu-HU", &seasons).await;
    assert_eq!(hits(&log, "/tv/55"), 1, "8 seasons = one request");
    {
        let conn = state.db.lock().await;
        for n in 1..=8 {
            let body = read_tmdb_season_cache(&conn, &tmdb_season_cache_key(55, n, "hu-HU"))
                .unwrap_or_else(|| panic!("season {n} must be cached under the per-season key"));
            assert!(body.contains(&format!("S{n}E1")));
        }
    }

    // everything cached -> a second visit costs nothing
    prefetch_tmdb_seasons(&state, "test-key", 55, "hu-HU", &seasons).await;
    assert_eq!(hits(&log, "/tv/55"), 1);
}

#[tokio::test]
async fn test_prefetch_splits_more_than_20_seasons_and_survives_failures() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        (500, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let seasons: Vec<i64> = (1..=25).collect();
    prefetch_tmdb_seasons(&state, "test-key", 9, "hu-HU", &seasons).await;
    assert_eq!(hits(&log, "/tv/9"), 4, "2 chunks (20 + 5), each retried once after the 5xx");
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM trakt_http_cache WHERE key LIKE 'tmdb_season:%'").await, 0);

    // a single missing season is not worth the combined request
    let before = hits(&log, "/tv/");
    prefetch_tmdb_seasons(&state, "test-key", 9, "hu-HU", &[1]).await;
    assert_eq!(hits(&log, "/tv/"), before);
}

#[tokio::test]
async fn test_stale_refresh_touches_only_old_automatic_id_rows_and_respects_the_limit() {
    let log: Log = Arc::new(StdMutex::new(Vec::new()));
    let l = log.clone();
    let base = spawn_mock_server(move |_method, path, _body| {
        l.lock().unwrap().push(path.clone());
        if path.starts_with("/tv/") {
            let id: i64 = path.trim_start_matches("/tv/").split('?').next().unwrap().parse().unwrap();
            return (200, vec![], details_body(id, "tv", "Refreshed"));
        }
        (404, vec![], "{}".to_string())
    })
    .await;
    let state = tmdb_state(base).await;

    let old = "datetime('now','localtime','-40 days')";
    {
        let conn = state.db.lock().await;
        for (key, id, manual, age) in [
            ("~id:tv:1", 1, 0, old),
            ("~id:tv:2", 2, 0, old),
            ("~id:tv:3", 3, 0, old),
            ("~id:tv:4", 4, 1, old),                                  // manual: never refreshed
            ("fargo", 5, 0, old),                                     // title mapping: never refreshed
            ("~id:tv:6", 6, 0, "datetime('now','localtime')"),       // fresh
        ] {
            conn.execute(
                &format!("INSERT INTO tmdb_cache (query_title, lang, tmdb_id, media_type, title, is_manual, updated_at) VALUES (?1, 'hu-HU', ?2, 'tv', 'Old', ?3, {age})"),
                rusqlite::params![key, id, manual],
            )
            .unwrap();
        }
    }

    assert_eq!(refresh_stale_tmdb(&state, 2).await, 2, "limit respected");
    assert_eq!(refresh_stale_tmdb(&state, 10).await, 1, "the remaining old automatic row");
    assert_eq!(refresh_stale_tmdb(&state, 10).await, 0);
    let touched: Vec<String> = log.lock().unwrap().iter().cloned().collect();
    assert!(touched.iter().all(|p| ["/tv/1", "/tv/2", "/tv/3"].iter().any(|x| p.starts_with(x))), "{:?}", touched);
    assert_eq!(scalar(&state, "SELECT COUNT(*) FROM tmdb_cache WHERE title = 'Old'").await, 3, "manual, mapping and fresh rows untouched");
}