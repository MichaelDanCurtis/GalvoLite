//! Task 13 — GET /api/sessions/:id/events?limit=N
//!
//! Dashboard-side reverse-chronological events feed. Mirrors the MCP
//! `read_events` shape (id/ts/source/kind/payload/prev_state/next_state) but
//! orders by id DESC so the newest event lands at index 0.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn get_session_events_returns_reverse_chrono() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session(&setup.db, &proj);
    for kind in ["a", "b"] {
        galvolite::state::apply_event(
            &setup.db,
            sid,
            galvolite::state::EventInput {
                source: galvolite::state::Source::System,
                kind: format!("system.{kind}"),
                payload: serde_json::json!({}),
                dedup_key: None,
            },
        )
        .unwrap();
    }
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/events?limit=10"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().expect("events endpoint returns a JSON array");
    assert_eq!(arr[0]["kind"], "system.b");
    assert_eq!(arr[1]["kind"], "system.a");
    // Row shape matches MCP read_events (id, not event_id).
    assert!(arr[0]["id"].is_number());
    assert!(arr[0]["ts"].is_string());
    assert_eq!(arr[0]["source"], "system");
    assert!(arr[0].get("payload").is_some());
    assert!(arr[0].get("prev_state").is_some());
    assert!(arr[0].get("next_state").is_some());
}

#[tokio::test]
async fn get_session_events_default_limit_when_missing() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj2");
    let (sid, _token) = common::seed_session(&setup.db, &proj);
    // Insert 3 events; no ?limit on the request — handler must default and
    // not 400 on a missing query param.
    for i in 0..3 {
        galvolite::state::apply_event(
            &setup.db,
            sid,
            galvolite::state::EventInput {
                source: galvolite::state::Source::System,
                kind: format!("system.e{i}"),
                payload: serde_json::json!({}),
                dedup_key: None,
            },
        )
        .unwrap();
    }
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/events"))
        .body(Body::empty())
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.as_array().map(|a| a.len()), Some(3));
}

#[tokio::test]
async fn get_session_events_clamps_limit() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj3");
    let (sid, _token) = common::seed_session(&setup.db, &proj);
    // Over-large limit should be clamped (no 400). We don't care about the
    // exact max here — only that the request succeeds.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/events?limit=100000"))
        .body(Body::empty())
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
