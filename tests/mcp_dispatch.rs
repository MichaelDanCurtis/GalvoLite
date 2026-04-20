//! `tools/call` dispatcher contract tests — error envelope shapes only.
//! Happy-path tool execution is covered by Tasks 6-11 once tools land.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

async fn rpc(app: axum::Router, token: &str, body: &str) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let r = rpc(
        setup.app,
        &token,
        r#"{"jsonrpc":"2.0","id":1,"method":"nope/x"}"#,
    )
    .await;
    assert_eq!(r["error"]["code"], -32601);
}

#[tokio::test]
async fn tools_call_unknown_tool_returns_method_not_found() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"no_such_tool","arguments":{}}}"#;
    let r = rpc(setup.app, &token, body).await;
    assert_eq!(
        r["error"]["code"], -32601,
        "unknown tool surfaces as method_not_found"
    );
}

#[tokio::test]
async fn tools_call_missing_params_returns_invalid_params() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#;
    let r = rpc(setup.app, &token, body).await;
    assert_eq!(r["error"]["code"], -32602);
}

#[tokio::test]
async fn tools_call_malformed_params_returns_invalid_params() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    // params is present but missing the required "name" field.
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{}}}"#;
    let r = rpc(setup.app, &token, body).await;
    assert_eq!(r["error"]["code"], -32602);
}
