//! `write_postmortem` MCP tool — round-trip + REST listing.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(app: axum::Router, token: &str, name: &str, args: Value) -> Value {
    let body = json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":name,"arguments":args}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn write_postmortem_persists_to_disk() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db, &proj, SessionState::Working,
    );

    let r = call(setup.app, &token, "write_postmortem", json!({
        "plan_part": 5,
        "outcome": "shipped",
        "cycles": 3,
        "duration_seconds": 2820,
        "what_worked": "TDD kept us honest",
        "gotchas": "flake on macOS",
        "follow_ups": "see SEED-007",
    })).await;
    assert!(r["error"].is_null(), "tool errored: {:?}", r["error"]);

    // File on disk under <proj>/.galvolite/postmortems/part-5-<sid>.md
    let dir = proj.join(".galvolite/postmortems");
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
    let body = std::fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
    assert!(body.contains("plan_part: 5"));
    assert!(body.contains("outcome: shipped"));
    assert!(body.contains("TDD kept us honest"));
    assert!(body.contains("flake on macOS"));
    assert!(body.contains("SEED-007"));
}

#[tokio::test]
async fn list_endpoint_returns_postmortem() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db, &proj, SessionState::Working,
    );

    // Write via MCP tool.
    let _ = call(setup.app.clone(), &token, "write_postmortem", json!({
        "plan_part": 7,
        "outcome": "stuck",
        "cycles": 2,
        "duration_seconds": 1500,
    })).await;

    // Read via REST.
    let pid: String = setup.db.pool().get().unwrap().query_row(
        "SELECT project_id FROM sessions LIMIT 1", [], |r| r.get(0)
    ).unwrap();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/projects/{pid}/postmortems?part=7"))
        .body(Body::empty()).unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let list: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["outcome"], "stuck");
    assert_eq!(list[0]["plan_part"], 7);
    assert_eq!(list[0]["cycles"], 2);
}
