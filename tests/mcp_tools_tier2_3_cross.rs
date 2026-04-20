//! Tier-2/3 cross-session + operator MCP tools (spec §5.3, §5.4).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

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

/// Seed two sessions in the SAME project so list_siblings has something to
/// return. Returns (sibling_id, self_id, self_token).
fn seed_two_in_same_project(
    setup: &common::TestApp,
    proj_dir: &std::path::Path,
) -> (Uuid, Uuid, String) {
    use galvolite::db::{NewProject, NewSession};
    std::fs::create_dir_all(proj_dir).unwrap();
    let project = setup
        .db
        .insert_project(NewProject {
            name: "shared-proj".into(),
            path: proj_dir.to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let sib = setup
        .db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "sib".into(),
            parent_session_id: None,
        })
        .unwrap();
    common::force_state(&setup.db, sib.id, SessionState::Working);
    let me = setup
        .db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "me".into(),
            parent_session_id: None,
        })
        .unwrap();
    common::force_state(&setup.db, me.id, SessionState::Idle);
    let token = format!("tok_{}", Uuid::new_v4().simple());
    setup.db.set_api_token(me.id, &token).unwrap();
    (sib.id, me.id, token)
}

#[tokio::test]
async fn list_siblings_returns_other_sessions_in_same_project() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sib, _me, token) = seed_two_in_same_project(&setup, &proj);

    let r = call(setup.app, &token, "list_siblings", json!({})).await;
    let body = inner_text(&r);
    let arr = body["siblings"].as_array().unwrap();
    let ids: Vec<String> = arr
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.contains(&sib.to_string()),
        "expected sibling {sib} in {ids:?}"
    );
}

#[tokio::test]
async fn list_siblings_excludes_caller() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sib, me, token) = seed_two_in_same_project(&setup, &proj);

    let r = call(setup.app, &token, "list_siblings", json!({})).await;
    let body = inner_text(&r);
    let arr = body["siblings"].as_array().unwrap();
    let ids: Vec<String> = arr
        .iter()
        .map(|s| s["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        !ids.contains(&me.to_string()),
        "caller {me} must not appear in siblings list: {ids:?}"
    );
}

#[tokio::test]
async fn status_of_returns_full_snapshot() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sib, _me, token) = seed_two_in_same_project(&setup, &proj);

    let r = call(
        setup.app,
        &token,
        "status_of",
        json!({ "session_id": sib.to_string() }),
    )
    .await;
    let body = inner_text(&r);
    assert_eq!(body["id"], sib.to_string());
    assert_eq!(body["state"], "Working");
    assert!(body["worktree"].is_string());
    assert!(body["role"].is_string());
}

#[tokio::test]
async fn status_of_missing_session_returns_app_error() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let bogus = Uuid::new_v4().to_string();
    let r = call(
        setup.app,
        &token,
        "status_of",
        json!({ "session_id": bogus }),
    )
    .await;
    // App errors come back as a successful JSON-RPC envelope with the tool
    // result flagged isError=true (per MCP spec — protocol errors only when
    // the tool itself can't be dispatched).
    assert_eq!(
        r["result"]["isError"], true,
        "expected isError=true, got: {r:?}"
    );
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("session not found"), "got: {text}");
}

#[tokio::test]
async fn read_events_returns_event_rows_for_session() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );

    // Generate two events via tools we already have.
    let _ = call(
        setup.app.clone(),
        &token,
        "status_report",
        json!({ "kind": "working" }),
    )
    .await;
    let _ = call(
        setup.app.clone(),
        &token,
        "request_attention",
        json!({ "reason": "x" }),
    )
    .await;

    let r = call(setup.app, &token, "read_events", json!({ "limit": 10 })).await;
    let body = inner_text(&r);
    let events = body["events"].as_array().unwrap();
    assert!(
        events.len() >= 2,
        "expected ≥2 events, got {}",
        events.len()
    );
    // Each row must carry the canonical shape.
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"mcp.status_report"));
    assert!(kinds.contains(&"mcp.request_attention"));
}

#[tokio::test]
async fn send_to_session_writes_event_targeting_other_session() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sib, _me, token) = seed_two_in_same_project(&setup, &proj);

    let r = call(
        setup.app,
        &token,
        "send_to_session",
        json!({
            "session_id": sib.to_string(),
            "text": "continue with plan",
            "submit": true
        }),
    )
    .await;
    assert!(r["error"].is_null(), "tool errored: {:?}", r["error"]);
    let body = inner_text(&r);
    assert_eq!(body["ok"], true);
    // Target has no live PTY in this test (DB-only seed); handler degrades to
    // event-only and reports as much.
    assert_eq!(body["delivered"], "event_only");

    // The event row landed against the TARGET session.
    let count: i64 = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.send_to_session'",
            [sib.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "event row written against the TARGET session");
}

#[tokio::test]
async fn ask_operator_returns_timed_out_immediately_in_degraded_mode() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );

    let start = std::time::Instant::now();
    let r = call(
        setup.app,
        &token,
        "ask_operator",
        json!({ "question": "approve?", "timeout_secs": 60 }),
    )
    .await;
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_secs() < 2,
        "degraded mode must return immediately, took {:?}",
        elapsed
    );

    let body = inner_text(&r);
    assert!(body["answer"].is_null());
    assert_eq!(body["timed_out"], true);
    assert_eq!(body["degraded"], true);
}

#[tokio::test]
async fn notify_operator_writes_event_no_state_change() {
    let setup = common::test_app();
    let (sid, token) = common::seed_session_in_state_at(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );

    let _ = call(
        setup.app,
        &token,
        "notify_operator",
        json!({ "level": "warn", "message": "hey look" }),
    )
    .await;
    assert_eq!(common::state_of(&setup.db, sid), SessionState::Working);

    let count: i64 = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.notify_operator'",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn notify_operator_invalid_level_returns_invalid_params() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );
    let r = call(
        setup.app,
        &token,
        "notify_operator",
        json!({ "level": "panic", "message": "boom" }),
    )
    .await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn notify_operator_publishes_toast_event() {
    use galvolite::api::events::DomainEvent;
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session(&setup.db, &proj);

    // Subscribe BEFORE calling the tool so we don't miss the broadcast frame.
    let mut rx = setup.mgr.subscribe_events();

    let _ = call(
        setup.app.clone(),
        &token,
        "notify_operator",
        json!({ "level": "warn", "message": "low fuel" }),
    )
    .await;

    // apply_event_with_bus fires SessionEventAppended first; skip non-toast
    // frames until the OperatorToast we care about arrives.
    let deadline = std::time::Duration::from_secs(1);
    let found = tokio::time::timeout(deadline, async {
        loop {
            match rx.recv().await.unwrap() {
                DomainEvent::OperatorToast {
                    session_id,
                    level,
                    message,
                } => break (session_id, level, message),
                _ => continue,
            }
        }
    })
    .await
    .expect("timed out waiting for OperatorToast");

    assert_eq!(found.0, sid);
    assert_eq!(found.1, "warn");
    assert_eq!(found.2, "low fuel");
}
