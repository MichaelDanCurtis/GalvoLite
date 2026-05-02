//! `ask_operator` answers auto-record decisions on disk + publish event.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn rpc(app: axum::Router, token: &str, name: &str, args: Value) -> Value {
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
async fn answer_writes_decision_file() {
    let setup = common::test_app_with_bus();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session_in_state_at(
        &setup.db, &proj, SessionState::Working,
    );

    // Spawn ask_operator in background.
    let app_clone = setup.app.clone();
    let token_owned = token.clone();
    let task = tokio::spawn(async move {
        rpc(app_clone, &token_owned, "ask_operator", json!({
            "question": "Should P5 wait for P4?",
            "choices": ["yes", "no"],
            "timeout_secs": 10,
        })).await
    });

    // Give the MCP call a moment to register.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Subscribe BEFORE answering so we can assert the DecisionRecorded event.
    let mut rx = setup.mgr.subscribe_events();

    // Look up the prompt id from the registry.
    let pending_count = setup.mgr.operator_registry().pending_count().await;
    assert_eq!(pending_count, 1);
    let prompt_id: uuid::Uuid = setup.mgr.operator_registry()
        .first_pending_id()
        .await
        .expect("one prompt should be pending");

    // Answer it via HTTP.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/operator/prompt/{prompt_id}/answer"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"answer": "no"}).to_string()))
        .unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let _ = task.await.unwrap();

    let dir = proj.join(".galvolite/decisions");
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
    let body = std::fs::read_to_string(entries[0].as_ref().unwrap().path()).unwrap();
    assert!(body.contains("Should P5 wait for P4?"), "body missing question: {body}");
    assert!(body.contains(&format!("session_id: {sid}")), "body missing session id: {body}");
    assert!(body.contains("\"no\"") || body.contains("no\n"), "body missing answer: {body}");

    // Drain a couple of events; assert one was DecisionRecorded.
    let mut saw = false;
    for _ in 0..5 {
        if let Ok(evt) = tokio::time::timeout(
            std::time::Duration::from_millis(50), rx.recv()
        ).await {
            if let Ok(galvolite::api::events::DomainEvent::DecisionRecorded { .. }) = evt {
                saw = true;
                break;
            }
        }
    }
    assert!(saw, "DecisionRecorded event not seen");
}
