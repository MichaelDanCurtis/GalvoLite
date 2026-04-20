//! Sub-plan 4 Task 6: end-to-end `ask_operator` round-trip.
//!
//! Two scenarios:
//! * Answer arrives over HTTP before timeout → the MCP call resolves with the
//!   operator's answer payload.
//! * Timeout fires first → the MCP call resolves with `{answer: null,
//!   timed_out: true}`.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn ask_operator_resolves_when_http_answer_lands() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session(&setup.db, &proj);
    let app_for_answer = setup.app.clone();
    let token_for_answer = token.clone();

    // Subscribe to events so we can grab the prompt_id.
    let mut rx = setup.mgr.subscribe_events();

    // Spawn the MCP call (blocks until answered).
    let app_for_ask = setup.app.clone();
    let ask = tokio::spawn(async move {
        let body = serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"ask_operator","arguments":{"question":"go?","timeout_secs":5}}
        });
        common::rpc(app_for_ask, &token, body).await
    });

    // Wait for OperatorPrompt event.
    let prompt_id = loop {
        match rx.recv().await.unwrap() {
            galvolite::api::events::DomainEvent::OperatorPrompt { prompt_id, .. } => {
                break prompt_id
            }
            _ => continue,
        }
    };

    // Answer over HTTP.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/operator/prompt/{prompt_id}/answer"))
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token_for_answer}"))
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({ "answer": "yes" })).unwrap(),
        ))
        .unwrap();
    let resp = app_for_answer.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The MCP call should now resolve with answer:"yes". The transport
    // stringifies the tool's inner JSON into `content[0].text`, so we parse
    // it back out to inspect it.
    let result = ask.await.unwrap();
    let text = result["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["answer"], "yes");
    assert_eq!(inner["timed_out"], false);
}

#[tokio::test]
async fn ask_operator_times_out_with_null_answer() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session(&setup.db, &proj);
    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"ask_operator","arguments":{"question":"q","timeout_secs":1}}
    });
    let r = common::rpc(setup.app, &token, body).await;
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["timed_out"], true);
    assert_eq!(inner["answer"], serde_json::Value::Null);
}

#[tokio::test]
async fn answer_unknown_prompt_returns_404() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session(&setup.db, &proj);
    let bogus = uuid::Uuid::new_v4();
    let req = Request::builder()
        .method("POST")
        .uri(format!("/operator/prompt/{bogus}/answer"))
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({ "answer": "anything" })).unwrap(),
        ))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    // Drain body to avoid axum warnings.
    let _ = to_bytes(resp.into_body(), 1024).await.unwrap();
}
