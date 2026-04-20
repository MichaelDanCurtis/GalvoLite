//! Tier-1 introspection + self-control (spec §5.1).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(app: axum::Router, token: &str, name: &str, args: Value) -> Value {
    let body = json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":name,"arguments":args}
    })
    .to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn inner_text(r: &Value) -> Value {
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn whoami_returns_session_identity() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );

    let r = call(setup.app, &token, "whoami", json!({})).await;
    let body = inner_text(&r);
    assert_eq!(body["id"], sid.to_string());
    assert!(body["role"].is_string());
    assert!(body["worktree"].is_string());
    assert!(body["started_at"].is_string());
    assert_eq!(body["state"], "working");
}

#[tokio::test]
async fn pause_self_transitions_to_awaiting_input() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call(
        setup.app,
        &token,
        "pause_self",
        json!({ "reason": "waiting on operator" }),
    )
    .await;
    assert_eq!(
        common::state_of(&setup.db, sid),
        SessionState::AwaitingInput
    );
}

#[tokio::test]
async fn pause_self_missing_reason_returns_invalid_params() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call(setup.app, &token, "pause_self", json!({})).await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn request_end_transitions_to_ended() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call(
        setup.app,
        &token,
        "request_end",
        json!({ "reason": "task complete" }),
    )
    .await;
    assert_eq!(common::state_of(&setup.db, sid), SessionState::Ended);
}

#[tokio::test]
async fn get_transcript_for_non_running_session_returns_empty() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Idle,
    );
    let r = call(setup.app, &token, "get_transcript", json!({ "turns": 5 })).await;
    let body = inner_text(&r);
    assert_eq!(body["running"], false);
    assert!(body["lines"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_pty_log_for_non_running_session_returns_empty() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Idle,
    );
    let r = call(setup.app, &token, "get_pty_log", json!({ "limit": 10 })).await;
    let body = inner_text(&r);
    assert_eq!(body["running"], false);
    assert_eq!(body["log"], "");
}

#[tokio::test]
async fn introspection_tools_write_no_event_rows() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call(setup.app.clone(), &token, "whoami", json!({})).await;
    let _ = call(setup.app.clone(), &token, "get_transcript", json!({})).await;
    let _ = call(setup.app, &token, "get_pty_log", json!({})).await;

    let count: i64 = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind LIKE 'mcp.%'",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "read-only tools must not write event rows");
}
