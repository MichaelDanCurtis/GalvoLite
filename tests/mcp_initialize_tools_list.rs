//! `initialize` and `tools/list` happy-path tests.
//!
//! Tools registry is empty until Tasks 6-10 land. The Task 11 smoke test
//! re-tightens this and asserts every spec §5 tool by name.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

async fn rpc(
    app: &axum::Router,
    token: &str,
    method: &str,
) -> Value {
    let body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"{method}","params":{{}}}}"#
    );
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn initialize_returns_protocol_version_and_capabilities() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let resp = rpc(&setup.app, &token, "initialize").await;
    let result = resp["result"].clone();
    assert_eq!(result["protocolVersion"], "2025-03-26");
    assert_eq!(
        result["capabilities"]["tools"]["listChanged"],
        false,
        "static catalog — never sends tools/list_changed"
    );
    assert_eq!(result["serverInfo"]["name"], "galvolite");
    assert!(result["serverInfo"]["version"].is_string());
}

#[tokio::test]
async fn tools_list_returns_array_with_required_descriptor_shape() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let resp = rpc(&setup.app, &token, "tools/list").await;
    let tools = resp["result"]["tools"].as_array().expect("tools array");

    // Tools registry is empty until Tasks 6-10 land. After Task 10, the smoke
    // test (Task 11) asserts each named tool from spec §5 is present.
    // For now: assert structural shape on whatever is there.
    for t in tools {
        assert!(t["name"].is_string());
        assert!(
            t["description"].is_string(),
            "tool {} lacks description",
            t["name"]
        );
        assert!(
            t["inputSchema"].is_object(),
            "tool {} lacks inputSchema",
            t["name"]
        );
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[tokio::test]
async fn tool_names_are_unique() {
    use std::collections::HashSet;
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let resp = rpc(&setup.app, &token, "tools/list").await;
    let names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let unique: HashSet<_> = names.iter().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate tool names — registry table broken"
    );
}

#[tokio::test]
async fn tools_list_is_sorted_by_name() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let resp = rpc(&setup.app, &token, "tools/list").await;
    let names: Vec<String> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "tools/list must be sorted by name");
}
