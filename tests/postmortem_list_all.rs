mod common;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;
use axum::http::StatusCode;

async fn call_mcp(app: axum::Router, token: &str, name: &str, args: Value) -> Value {
    let body = json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":name,"arguments":args}
    });
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn list_endpoint_returns_all_parts_when_part_omitted() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session_in_state_at(
        &setup.db, &proj, SessionState::Working,
    );

    // Write two postmortems for different parts via MCP tool.
    let r = call_mcp(setup.app.clone(), &token, "write_postmortem", json!({
        "plan_part": 3,
        "outcome": "shipped",
        "cycles": 1,
        "duration_seconds": 900,
    })).await;
    assert!(r["error"].is_null(), "tool errored: {:?}", r["error"]);

    let r = call_mcp(setup.app.clone(), &token, "write_postmortem", json!({
        "plan_part": 7,
        "outcome": "stuck",
        "cycles": 4,
        "duration_seconds": 3600,
    })).await;
    assert!(r["error"].is_null(), "tool errored: {:?}", r["error"]);

    // Fetch the project id from the DB.
    let pid: String = setup.db.pool().get().unwrap().query_row(
        "SELECT project_id FROM sessions LIMIT 1", [], |r| r.get(0)
    ).unwrap();

    // GET without ?part — both postmortems should come back.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/projects/{pid}/postmortems"))
        .body(Body::empty()).unwrap();
    let resp = setup.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let list: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.len(), 2);
    let parts: Vec<u64> = list.iter()
        .map(|pm| pm["plan_part"].as_u64().unwrap())
        .collect();
    assert!(parts.contains(&3), "expected part 3 in {:?}", parts);
    assert!(parts.contains(&7), "expected part 7 in {:?}", parts);
}
