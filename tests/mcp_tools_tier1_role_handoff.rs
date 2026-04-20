//! Tier-1 role + handoff tools (spec §5.1).

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

const VALID_HANDOFF: &str = r#"## Task Assignment
Continue refactoring the auth middleware.

## Current State
src/auth.rs line 42 — pending decision.

## Decisions Made
Chose subtle::ConstantTimeEq over manual loop because timing oracle.
Rejected ring::constant_time::verify_slices_are_equal because heavier dep.

## Modified Files
src/auth.rs, tests/auth_resolver.rs.

## Open Questions / Blockers
None.

## Next Steps
1. Wire token rotation into spawn flow.
2. Add hook ingest path.

## Critical Context
Token rotation must keep at least one valid window for in-flight requests.

## Git State
On branch plan-mcp-surface; clean working tree.

## Continuation Prompt
You are picking up the GalvoLite MCP surface. Read src/auth.rs in full;
then continue from src/mcp/transport.rs which already calls auth::resolve_token.
The next task is to wire the spawn flow so a fresh per-session token is
written into .mcp.json at session start. All tests pass currently.
"#;

#[tokio::test]
async fn set_role_updates_db_and_rewrites_claude_md() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &proj,
        SessionState::Working,
    );

    let r = call(setup.app, &token, "set_role", json!({ "role": "scout" })).await;
    assert!(r["error"].is_null(), "tool errored: {:?}", r["error"]);

    let role: String = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT role FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(role, "scout");

    let claude_md = proj.join("CLAUDE.md");
    let body = std::fs::read_to_string(claude_md).unwrap();
    assert!(
        body.to_lowercase().contains("scout"),
        "rewritten CLAUDE.md should reflect new role"
    );
}

#[tokio::test]
async fn set_role_invalid_value_returns_invalid_params() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call(setup.app, &token, "set_role", json!({ "role": "wizard" })).await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn write_handoff_creates_file_event_and_handoff_path() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session_in_state(&setup.db, &proj, SessionState::Working);

    let r = call(
        setup.app,
        &token,
        "write_handoff",
        json!({ "body": VALID_HANDOFF }),
    )
    .await;
    assert!(r["error"].is_null(), "{:?}", r["error"]);
    let path = r["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    // The MCP envelope text is JSON of the tool's Ok payload — parse it back.
    let inner: Value = serde_json::from_str(path).unwrap();
    let on_disk = inner["path"].as_str().unwrap();
    assert!(std::path::Path::new(on_disk).exists());

    // Event row.
    let count: i64 = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.write_handoff'",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // sessions.handoff_path mirrors the file path.
    let stored: String = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT handoff_path FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, on_disk);
}

#[tokio::test]
async fn write_handoff_rejects_invalid_body() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call(
        setup.app,
        &token,
        "write_handoff",
        json!({ "body": "this is not a valid handoff" }),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn read_handoff_returns_most_recent_for_session() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session_in_state(&setup.db, &proj, SessionState::Working);

    let _ = call(
        setup.app.clone(),
        &token,
        "write_handoff",
        json!({ "body": VALID_HANDOFF }),
    )
    .await;
    // Sleep 1s so the second handoff has a distinct second-resolution filename.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let body2 = VALID_HANDOFF.replace("Continue refactoring the auth middleware.",
        "Continue refactoring — second pass with token rotation considered.");
    let _ = call(
        setup.app.clone(),
        &token,
        "write_handoff",
        json!({ "body": body2 }),
    )
    .await;

    let r = call(setup.app, &token, "read_handoff", json!({})).await;
    let inner: Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let content = inner["content"].as_str().unwrap();
    assert!(
        content.contains("second pass"),
        "expected most-recent handoff body; got: {content}"
    );
}

#[tokio::test]
async fn read_handoff_with_no_files_returns_null() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call(setup.app, &token, "read_handoff", json!({})).await;
    let inner: Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(inner["content"].is_null());
}
