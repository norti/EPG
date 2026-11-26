use crate::google::auth::{
    generate_pkce, generate_state_nonce, load_google_tokens_from_db, parse_callback_target,
    save_google_tokens_in_conn, GoogleTokenResponse,
};
use crate::google::calendar::{
    build_calendar_description, build_calendar_event_url, build_calendar_events_url,
    build_calendar_list_entry_url, count_tracked_events_in_conn,
    get_all_tracked_event_keys_in_conn, get_tracked_event_in_conn, parse_synced_event_item,
    select_cleanup_candidates_in_conn, upsert_tracked_event_in_conn, ParsedSyncedEvent,
    TrackedCalendarEvent,
};
use crate::google::drive::{build_sync_payload_from_conn, SyncPayload};
use crate::google::prune_http_cache;
use crate::migrations::run_migrations;
use crate::trakt::secret::default_secret_store;
use crate::import_settings_into_conn;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rusqlite::Connection;
use sha2::{Digest, Sha256};

#[test]
fn test_generate_pkce_valid_and_unique() {
    let a = generate_pkce();
    let b = generate_pkce();

    assert!(a.code_verifier.len() >= 43 && a.code_verifier.len() <= 128);
    assert_ne!(a.code_verifier, b.code_verifier);

    let expected_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(a.code_verifier.as_bytes()));
    assert_eq!(a.code_challenge, expected_challenge);

    let s1 = generate_state_nonce();
    let s2 = generate_state_nonce();
    assert_eq!(s1.len(), 32);
    assert_ne!(s1, s2);
}

#[test]
fn test_parse_callback_target_decodes_code_and_checks_csrf() {
    // Non-callback paths (e.g. browser favicon fetch) must be ignored
    assert!(parse_callback_target("/favicon.ico", "nonce123").is_none());

    // Google auth codes contain percent-encoded slashes (`4%2F0A...`)
    let ok = parse_callback_target(
        "/callback?state=nonce123&code=4%2F0AfJohX_test%2Bcode&scope=openid",
        "nonce123",
    )
    .expect("callback should match")
    .expect("valid callback should succeed");
    assert_eq!(ok, "4/0AfJohX_test+code");

    // CSRF mismatch must fail
    let bad_state = parse_callback_target("/callback?state=wrong&code=abc", "nonce123")
        .expect("callback should match");
    assert!(bad_state.is_err());

    // OAuth denial must fail with error
    let denied = parse_callback_target("/callback?error=access_denied&state=nonce123", "nonce123")
        .expect("callback should match");
    assert!(denied.is_err());
}

#[test]
fn test_build_calendar_description_utf8_safe_truncation() {
    let accented = "Árvíztűrő tükörfúrógép ".repeat(35);
    assert!(accented.chars().count() > 500);

    let desc = build_calendar_description(
        "HBO HD",
        "Telekom",
        Some(&accented),
        Some(8.4),
        Some("2010"),
    );

    assert!(desc.contains("Csatorna: HBO HD"));
    assert!(desc.contains("Forrás: Telekom"));
    assert!(desc.contains("…"));
    assert!(desc.contains("TMDB: 8.4/10 (2010)"));
}

#[test]
fn test_calendar_url_encoding_special_chars() {
    let url = build_calendar_events_url("user+tv@group.calendar.google.com").unwrap();
    assert_eq!(
        url.as_str(),
        "https://www.googleapis.com/calendar/v3/calendars/user+tv@group.calendar.google.com/events"
    );

    let hash_url = build_calendar_list_entry_url("hu.hungarian#holiday@group.v.calendar.google.com").unwrap();
    assert!(hash_url.as_str().contains("%23"));
    assert!(!hash_url.as_str().contains('#'));
}

#[cfg(windows)]
#[test]
fn test_google_token_dpapi_roundtrip_and_refresh_preservation() {
    let mut conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, None).unwrap();
    let store = default_secret_store();

    let initial = GoogleTokenResponse {
        access_token: "ya29.access_1".to_string(),
        refresh_token: Some("1//refresh_original".to_string()),
        expires_in: 3600,
        id_token: None,
    };
    save_google_tokens_in_conn(&mut conn, store.as_ref(), &initial).unwrap();

    // Refresh response without a new refresh_token must keep the original one
    let refreshed = GoogleTokenResponse {
        access_token: "ya29.access_2".to_string(),
        refresh_token: None,
        expires_in: 3600,
        id_token: None,
    };
    save_google_tokens_in_conn(&mut conn, store.as_ref(), &refreshed).unwrap();

    let loaded = load_google_tokens_from_db(&conn, store.as_ref())
        .unwrap()
        .expect("token should load");
    assert_eq!(loaded.access_token.as_str(), "ya29.access_2");
    assert_eq!(loaded.refresh_token.as_str(), "1//refresh_original");

    // Corrupted DPAPI blob must clear row cleanly and return Ok(None)
    conn.execute(
        "UPDATE google_auth SET access_blob = 'not-valid-base64!!!' WHERE id = 1",
        [],
    )
    .unwrap();
    let after_corrupt = load_google_tokens_from_db(&conn, store.as_ref()).unwrap();
    assert!(after_corrupt.is_none());
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM google_auth", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 0);
}

#[test]
fn test_sync_payload_preserves_sort_presets_and_excludes_secrets() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, None).unwrap();

    conn.execute(
        "INSERT INTO sources (id, name, url, country_code, time_offset, sort_position) VALUES ('s1', 'HU', 'https://example.com/hu.xml', 'HU', 0, 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO channel_sort_presets (id, name, source_id, channel_order, created_at) VALUES (1, 'MyOrder', 's1', '[\"ch2\",\"ch1\"]', '2026-11-26 12:00:00')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO settings (key, value) VALUES ('google_calendar_target_id', 'cal123'), ('google_secure_secret', 'nope'), ('tmdb_secure_key', 'nope2')",
        [],
    )
    .unwrap();

    let payload = build_sync_payload_from_conn(&conn).unwrap();
    let json = serde_json::to_string(&payload).unwrap();
    let parsed: SyncPayload = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.backup.channel_sort_presets.len(), 1);
    assert_eq!(parsed.backup.channel_sort_presets[0].name, "MyOrder");
    assert_eq!(
        parsed.backup.settings.get("google_calendar_target_id").map(String::as_str),
        Some("cal123")
    );
    assert!(!parsed.backup.settings.contains_key("google_secure_secret"));
    assert!(!parsed.backup.settings.contains_key("tmdb_secure_key"));

    // Roundtrip import into a clean DB preserves presets and target calendar
    let mut conn2 = Connection::open_in_memory().unwrap();
    run_migrations(&conn2, None).unwrap();
    import_settings_into_conn(&mut conn2, parsed.backup).unwrap();

    let preset_name: String = conn2
        .query_row("SELECT name FROM channel_sort_presets WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(preset_name, "MyOrder");

    // Verify prune_http_cache deletes expired entries only
    conn2
        .execute(
            "INSERT INTO google_http_cache (key, etag, body, fetched_at, ttl_seconds) VALUES
             ('old', NULL, '[]', '2020-01-01T00:00:00Z', 60),
             ('fresh', NULL, '[]', '2099-01-01T00:00:00Z', 300)",
            [],
        )
        .unwrap();
    let pruned = prune_http_cache(&conn2);
    assert_eq!(pruned, 1);
    let left: String = conn2
        .query_row("SELECT key FROM google_http_cache", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, "fresh");
}

#[test]
fn test_calendar_event_tracking_sync_parsing_and_cleanup_filter() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn, None).unwrap();

    // Verify DELETE URL encoding
    let del_url = build_calendar_event_url("cal#1@group.calendar.google.com", "evt_123").unwrap();
    assert!(del_url.as_str().contains("%23"));
    assert!(del_url.as_str().ends_with("/events/evt_123"));

    // Non-EPG-Viewer events must ALWAYS be ignored
    let foreign = serde_json::json!({
        "id": "foreign_evt",
        "status": "confirmed",
        "summary": "Dentist appointment"
    });
    assert!(matches!(
        parse_synced_event_item("primary", None, &foreign),
        ParsedSyncedEvent::Ignored
    ));

    // Cancelled tombstones produce Cancelled
    let cancelled = serde_json::json!({
        "id": "evt_old",
        "status": "cancelled"
    });
    assert!(matches!(
        parse_synced_event_item("primary", None, &cancelled),
        ParsedSyncedEvent::Cancelled { ref event_id } if event_id == "evt_old"
    ));

    // Valid EPG Viewer event parses cleanly and upserts into DB
    let epg_item = serde_json::json!({
        "id": "evt_1",
        "status": "confirmed",
        "summary": "Inception (Eredet)",
        "htmlLink": "https://calendar.google.com/event?eid=1",
        "extendedProperties": {
            "private": {
                "epgViewer": "1",
                "epgKey": "porthu|398|1000",
                "sourceId": "porthu",
                "channelId": "398",
                "startTs": "1000",
                "stopTs": "4600",
                "createdBy": "EPG Viewer",
                "appVersion": "1.1.0"
            }
        }
    });
    match parse_synced_event_item("primary", Some("Primary"), &epg_item) {
        ParsedSyncedEvent::Upsert(ev) => {
            upsert_tracked_event_in_conn(&conn, &ev).unwrap();
        }
        _ => panic!("Expected Upsert"),
    }

    // Add a newer event (stop_ts = 2_000_000)
    let newer = TrackedCalendarEvent {
        epg_key: "porthu|398|1996400".to_string(),
        source_id: "porthu".to_string(),
        channel_id: "398".to_string(),
        start_ts: 1_996_400,
        stop_ts: 2_000_000,
        title: "Future Show".to_string(),
        calendar_id: "primary".to_string(),
        calendar_name: None,
        event_id: "evt_2".to_string(),
        html_link: None,
        created_at: "2026-11-26T12:00:00Z".to_string(),
    };
    upsert_tracked_event_in_conn(&conn, &newer).unwrap();

    assert_eq!(count_tracked_events_in_conn(&conn), 2);
    assert_eq!(get_all_tracked_event_keys_in_conn(&conn).len(), 2);
    assert_eq!(
        get_tracked_event_in_conn(&conn, "porthu|398|1000")
            .unwrap()
            .event_id,
        "evt_1"
    );

    // Cleanup older than 7 days at now_ts = 2_000_000 selects ONLY the old event (stop_ts = 4600)
    let older_7d = select_cleanup_candidates_in_conn(&conn, Some(7), 2_000_000);
    assert_eq!(older_7d.len(), 1);
    assert_eq!(older_7d[0].0, "porthu|398|1000");

    // Cleanup all (None) selects both tracked EPG Viewer events
    let all = select_cleanup_candidates_in_conn(&conn, None, 2_000_000);
    assert_eq!(all.len(), 2);
}
