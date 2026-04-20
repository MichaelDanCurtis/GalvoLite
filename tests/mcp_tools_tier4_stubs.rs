//! Tier-4 reserved stubs (spec §5.2–§5.6). Each stub registers a real ToolDef
//! whose handler returns `not_implemented: <name> — owned by …`.

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

#[tokio::test]
async fn every_reserved_stub_returns_not_implemented() {
    let setup = common::test_app();
    let (_sid, token) = common::seed_session_in_state(
        &setup.db,
        &setup._dir.path().join("proj"),
        SessionState::Working,
    );

    let stubs: &[(&str, Value)] = &[
        // Path ownership (Plan 4)
        ("claim_paths", json!({ "globs": ["src/**"] })),
        ("release_paths", json!({ "globs": ["src/**"] })),
        ("list_claims", json!({})),
        ("check_path", json!({ "path": "src/lib.rs" })),
        // Spawn (Plan 4)
        ("spawn_child", json!({ "role": "scout", "task": "look around" })),
        ("wait_for_children", json!({ "ids": [] })),
        // Portfolio (Plan 5+)
        ("list_available_mcps", json!({})),
        ("list_session_mcps", json!({ "session_id": "x" })),
        (
            "configure_session_mcps",
            json!({ "session_id": "x", "servers": [] }),
        ),
        ("gateway_toggle", json!({ "server": "x", "enable": true })),
        // GitHub (Plan 5+)
        ("open_pr", json!({ "title": "x", "body": "y" })),
        ("list_prs", json!({})),
        ("request_review", json!({ "pr": 1 })),
        ("merge_pr", json!({ "pr": 1 })),
        ("get_pr_comments", json!({ "pr": 1 })),
        // Context r/w (capability-blocked)
        ("read_context", json!({})),
        ("write_context", json!({ "text": "hi" })),
    ];

    for (name, args) in stubs {
        let r = call(setup.app.clone(), &token, name, args.clone()).await;
        let result = r["result"].clone();
        assert_eq!(
            result["isError"], true,
            "{name}: must report isError:true; got: {r:?}"
        );
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: missing content[0].text"));
        assert!(
            text.contains("not_implemented"),
            "{name}: error text must contain 'not_implemented', got: {text}"
        );
        assert!(
            text.contains(*name),
            "{name}: error text should name the tool, got: {text}"
        );
    }
}

#[tokio::test]
async fn stubs_appear_in_tools_list() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let r: Value = serde_json::from_slice(&bytes).unwrap();

    let names: Vec<String> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();

    for stub in &[
        "spawn_child",
        "open_pr",
        "claim_paths",
        "list_available_mcps",
        "read_context",
    ] {
        assert!(
            names.contains(&stub.to_string()),
            "reserved stub '{stub}' missing from tools/list"
        );
    }
}

#[tokio::test]
async fn stubs_carry_reserved_descriptor_suffix() {
    let setup = common::test_app();
    let (_, token) = common::seed_session(&setup.db, &setup._dir.path().join("proj"));
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let r: Value = serde_json::from_slice(&bytes).unwrap();

    let arr = r["result"]["tools"].as_array().unwrap();
    for t in arr {
        let name = t["name"].as_str().unwrap();
        if matches!(
            name,
            "spawn_child"
                | "open_pr"
                | "claim_paths"
                | "list_available_mcps"
                | "read_context"
        ) {
            let desc = t["description"].as_str().unwrap();
            assert!(
                desc.contains("RESERVED"),
                "{name} descriptor should be marked RESERVED: {desc}"
            );
        }
    }
}
