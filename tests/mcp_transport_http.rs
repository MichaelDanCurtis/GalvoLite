//! Streamable HTTP transport for `POST /mcp`.
//!
//! Covers bearer auth, parse failures, envelope validation, and the
//! `Accept: text/event-stream` framing branch. Tool-specific dispatch is
//! exercised in later test files.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt;

fn post_mcp(token: Option<&str>, accept: &str, body: serde_json::Value) -> Request<Body> {
    let mut req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", accept);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    req.body(Body::from(body.to_string())).unwrap()
}

async fn body_string(resp: axum::response::Response) -> (StatusCode, String, String) {
    let status = resp.status();
    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    (status, ct, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn missing_authorization_yields_unauthorized_envelope() {
    let app = common::test_app();
    let req = post_mcp(
        None,
        "application/json",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    );
    let resp = app.app.oneshot(req).await.unwrap();
    let (status, ct, body) = body_string(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.contains("application/json"));
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32001);
}

#[tokio::test]
async fn invalid_token_yields_unauthorized_envelope() {
    let setup = common::test_app();
    let _ = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let req = post_mcp(
        Some("nope-not-a-token"),
        "application/json",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    );
    let resp = setup.app.oneshot(req).await.unwrap();
    let (_, _, body) = body_string(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32001);
}

#[tokio::test]
async fn valid_token_initialize_returns_server_info() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let req = post_mcp(
        Some(&token),
        "application/json",
        json!({"jsonrpc":"2.0","id":42,"method":"initialize"}),
    );
    let resp = setup.app.oneshot(req).await.unwrap();
    let (status, ct, body) = body_string(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.contains("application/json"));
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 42);
    assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(v["result"]["serverInfo"]["name"], "galvolite");
    assert_eq!(v["result"]["capabilities"]["tools"]["listChanged"], false);
}

#[tokio::test]
async fn malformed_json_yields_parse_error_envelope() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from("not-json"))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    let (_, _, body) = body_string(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32700);
}

#[tokio::test]
async fn wrong_jsonrpc_version_yields_invalid_request() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let req = post_mcp(
        Some(&token),
        "application/json",
        json!({"jsonrpc":"1.0","id":1,"method":"initialize"}),
    );
    let resp = setup.app.oneshot(req).await.unwrap();
    let (_, _, body) = body_string(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32600);
}

#[tokio::test]
async fn unknown_method_yields_method_not_found() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let req = post_mcp(
        Some(&token),
        "application/json",
        json!({"jsonrpc":"2.0","id":1,"method":"does/not/exist"}),
    );
    let resp = setup.app.oneshot(req).await.unwrap();
    let (_, _, body) = body_string(resp).await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"]["code"], -32601);
}

#[tokio::test]
async fn accept_event_stream_returns_sse_framing() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let req = post_mcp(
        Some(&token),
        "text/event-stream",
        json!({"jsonrpc":"2.0","id":99,"method":"initialize"}),
    );
    let resp = setup.app.oneshot(req).await.unwrap();
    let (status, ct, body) = body_string(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.contains("text/event-stream"), "got {ct}");
    assert!(body.starts_with("event: message\n"), "body: {body:?}");
    assert!(body.contains("\"id\":99"));
    assert!(body.ends_with("\n\n"));
}
