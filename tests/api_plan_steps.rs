//! Task 14 — GET /api/sessions/:id/plan_steps
//!
//! Dashboard-side Plan tab feed. Returns the session's structured plan steps
//! ordered by `ordinal ASC`. The DB stores `status = 'in_progress'` but the UI
//! expects `"working"`; the helper normalizes on the way out so the JSON
//! shape matches the UI `PlanStep` type.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn get_session_plan_steps_returns_ordered_rows() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, _token) = common::seed_session(&setup.db, &proj);
    common::seed_plan_steps(
        &setup.db,
        sid,
        &[
            ("h1", 1, "first", "done", None),
            ("h2", 2, "second", "in_progress", Some("midway")),
            ("h3", 3, "third", "pending", None),
        ],
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/plan_steps"))
        .body(Body::empty())
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().expect("plan_steps endpoint returns a JSON array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["hash"], "h1");
    assert_eq!(arr[0]["text"], "first");
    assert_eq!(arr[0]["status"], "done");
    assert!(arr[0]["note"].is_null());
    assert_eq!(arr[1]["hash"], "h2");
    assert_eq!(arr[1]["text"], "second");
    // in_progress → working on the way out.
    assert_eq!(arr[1]["status"], "working");
    assert_eq!(arr[1]["note"], "midway");
    assert_eq!(arr[2]["hash"], "h3");
    assert_eq!(arr[2]["status"], "pending");
    assert!(arr[2]["note"].is_null());
}

#[tokio::test]
async fn get_session_plan_steps_maps_in_progress_to_working() {
    // Dedicated assertion for the one schema mismatch between DB and UI.
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj-map");
    let (sid, _token) = common::seed_session(&setup.db, &proj);
    common::seed_plan_steps(
        &setup.db,
        sid,
        &[("only", 1, "step", "in_progress", None)],
    );
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/plan_steps"))
        .body(Body::empty())
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 4 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["status"], "working");
    // The other statuses must not be rewritten — guard against an overly
    // enthusiastic mapping helper.
}

#[tokio::test]
async fn get_session_plan_steps_empty_when_no_rows() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj-empty");
    let (sid, _token) = common::seed_session(&setup.db, &proj);
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/plan_steps"))
        .body(Body::empty())
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().expect("always a JSON array, even when empty");
    assert!(arr.is_empty());
}
