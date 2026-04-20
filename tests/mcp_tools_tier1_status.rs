//! Tier-1 status & lifecycle tools — handler + state-funnel integration.
//! Spec §5.1.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

async fn call_tool(
    app: axum::Router,
    token: &str,
    name: &str,
    args: Value,
) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": args }
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

fn count_events(db: &galvolite::db::Db, sid: Uuid, kind: &str) -> i64 {
    db.pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = ?2",
            rusqlite::params![sid.to_string(), kind],
            |r| r.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn status_report_working_writes_event_no_state_change() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );

    let r = call_tool(
        setup.app,
        &token,
        "status_report",
        json!({ "kind": "working", "note": "still chewing" }),
    )
    .await;
    assert!(r["error"].is_null(), "tool errored: {:?}", r["error"]);
    assert_eq!(common::state_of(&setup.db, sid), SessionState::Working);
    assert_eq!(count_events(&setup.db, sid, "mcp.status_report"), 1);
}

#[tokio::test]
async fn status_report_blocked_transitions_to_needs_attention() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call_tool(
        setup.app,
        &token,
        "status_report",
        json!({ "kind": "blocked", "note": "need creds" }),
    )
    .await;
    assert_eq!(
        common::state_of(&setup.db, sid),
        SessionState::NeedsAttention
    );
}

#[tokio::test]
async fn status_report_complete_transitions_to_idle() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call_tool(
        setup.app,
        &token,
        "status_report",
        json!({ "kind": "complete" }),
    )
    .await;
    assert_eq!(common::state_of(&setup.db, sid), SessionState::Idle);
}

#[tokio::test]
async fn request_attention_transitions_and_records_reason() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call_tool(
        setup.app,
        &token,
        "request_attention",
        json!({ "reason": "urgent decision needed" }),
    )
    .await;
    assert_eq!(
        common::state_of(&setup.db, sid),
        SessionState::NeedsAttention
    );

    let payload: String = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT payload_json FROM events WHERE session_id = ?1 AND kind = 'mcp.request_attention'",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(payload.contains("urgent decision needed"));

    // last_status_note column also captures the reason for the dashboard.
    let note: String = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT last_status_note FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(note, "urgent decision needed");
}

#[tokio::test]
async fn set_plan_writes_plan_markdown_column() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let md = "- [ ] step one\n- [x] step two done\n";
    let _ = call_tool(
        setup.app,
        &token,
        "set_plan",
        json!({ "markdown": md }),
    )
    .await;
    let stored: String = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT plan_markdown FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, md);
}

#[tokio::test]
async fn set_seed_writes_seed_markdown_column() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Idle,
    );
    let _ = call_tool(
        setup.app,
        &token,
        "set_seed",
        json!({ "markdown": "# Seed\nuse rust" }),
    )
    .await;
    let stored: String = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT seed_markdown FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.contains("# Seed"));
}

#[tokio::test]
async fn status_report_invalid_kind_returns_invalid_params() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call_tool(
        setup.app,
        &token,
        "status_report",
        json!({ "kind": "ascending", "note": "?" }),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn plan_check_step_writes_event() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let _ = call_tool(
        setup.app.clone(),
        &token,
        "set_plan",
        json!({ "markdown": "- [ ] alpha\n- [ ] beta\n" }),
    )
    .await;
    let _ = call_tool(
        setup.app,
        &token,
        "plan_check_step",
        json!({ "title": "alpha", "status": "done" }),
    )
    .await;
    assert_eq!(count_events(&setup.db, sid, "mcp.plan_check_step"), 1);
}

#[tokio::test]
async fn plan_check_step_without_title_or_hash_returns_invalid_params() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call_tool(
        setup.app,
        &token,
        "plan_check_step",
        json!({ "status": "done" }),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);
}
