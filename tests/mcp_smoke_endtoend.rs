//! End-to-end MCP smoke test.
//!
//! Boots the Axum app, mints a session + token, fires raw JSON-RPC at
//! `POST /mcp` covering the initialize handshake, tools/list completeness,
//! and one representative tool from each tier (Tier 1 status, Tier 1 role,
//! Tier 1 introspection, Tier 2 read, Tier 3 degraded operator, Tier 4
//! reserved stub, Tier 1 self-control). Verifies token revocation on Ended
//! per spec §7.2.
//!
//! Proves the wire works without a real claude PTY. Sub-plan 5's real-claude
//! smoke will exercise the same endpoint from a CC-spawned MCP client.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::state::SessionState;
use serde_json::{json, Value};
use tower::ServiceExt;

async fn rpc(
    app: axum::Router,
    token: &str,
    method: &str,
    params: Value,
    id: i64,
) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
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
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    if status != StatusCode::OK {
        panic!("rpc {method}: HTTP {status}: {}", String::from_utf8_lossy(&bytes));
    }
    serde_json::from_slice(&bytes).unwrap()
}

fn inner_text(r: &Value) -> Value {
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn full_mcp_lifecycle_initialize_through_each_tier() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session_in_state(
        &setup.db,
        &proj,
        SessionState::Working,
    );

    // 1. initialize
    let r = rpc(setup.app.clone(), &token, "initialize", json!({}), 1).await;
    let init = r["result"].clone();
    assert_eq!(init["protocolVersion"], "2025-03-26");
    assert_eq!(init["serverInfo"]["name"], "galvolite");

    // 2. tools/list — verify catalog is complete (full §5 catalog).
    let r = rpc(setup.app.clone(), &token, "tools/list", json!({}), 2).await;
    let names: Vec<String> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    let expected = [
        // Tier 1 — status & lifecycle
        "status_report",
        "request_attention",
        "set_plan",
        "plan_check_step",
        "set_seed",
        // Tier 1 — role + handoff
        "set_role",
        "write_handoff",
        "read_handoff",
        // Tier 1 — introspection + self-control
        "whoami",
        "get_transcript",
        "get_pty_log",
        "pause_self",
        "request_end",
        // Tier 2/3
        "list_siblings",
        "status_of",
        "read_events",
        "send_to_session",
        "ask_operator",
        "notify_operator",
        // Tier 4 (sample)
        "spawn_child",
        "claim_paths",
        "open_pr",
        "list_available_mcps",
        "read_context",
    ];
    for name in expected.iter() {
        assert!(
            names.contains(&name.to_string()),
            "tools/list missing required tool: {name}"
        );
    }

    // 3. status_report — Tier 1, writes event, no state change.
    let r = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({
            "name": "status_report",
            "arguments": { "kind": "working", "note": "smoke" }
        }),
        3,
    )
    .await;
    assert!(r["error"].is_null());
    // isError is omitted from the envelope when false (serde
    // skip_serializing_if = !x); only present and `true` on AppError.
    assert!(
        r["result"]["isError"].is_null() || r["result"]["isError"] == false,
        "expected success envelope, got: {r:?}"
    );
    assert_eq!(common::state_of(&setup.db, sid), SessionState::Working);

    // 4. set_role — Tier 1, rewrites CLAUDE.md.
    let _ = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({
            "name": "set_role",
            "arguments": { "role": "scout" }
        }),
        4,
    )
    .await;
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

    // 5. whoami — Tier 1, read-only.
    let r = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({ "name": "whoami", "arguments": {} }),
        5,
    )
    .await;
    let body = inner_text(&r);
    assert_eq!(body["id"], sid.to_string());
    assert_eq!(body["role"], "scout");

    // 6. read_events — Tier 2 read; should include the events written above.
    let r = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({
            "name": "read_events",
            "arguments": { "limit": 50 }
        }),
        6,
    )
    .await;
    let body = inner_text(&r);
    let events = body["events"].as_array().unwrap();
    assert!(
        events.len() >= 2,
        "should have at least status_report + set_role events, got {}",
        events.len()
    );
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"mcp.status_report"));
    assert!(kinds.contains(&"mcp.set_role"));

    // 7. ask_operator — Tier 3 degraded.
    let r = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({
            "name": "ask_operator",
            "arguments": { "question": "approve smoke?" }
        }),
        7,
    )
    .await;
    let body = inner_text(&r);
    assert_eq!(body["timed_out"], true);
    assert_eq!(body["degraded"], true);

    // 8. spawn_child — Tier 4 reserved stub.
    let r = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({
            "name": "spawn_child",
            "arguments": { "role": "scout", "task": "x" }
        }),
        8,
    )
    .await;
    assert_eq!(r["result"]["isError"], true);
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not_implemented"));
    assert!(text.contains("spawn_child"));

    // 9. request_end — final transition.
    let _ = rpc(
        setup.app.clone(),
        &token,
        "tools/call",
        json!({
            "name": "request_end",
            "arguments": { "reason": "smoke complete" }
        }),
        9,
    )
    .await;
    assert_eq!(common::state_of(&setup.db, sid), SessionState::Ended);

    // 10. Event log shape — every successful mcp.* call wrote a row.
    let mcp_events: i64 = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind LIKE 'mcp.%'",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    // Active mcp.* writers in this run: status_report, set_role,
    // ask_operator, request_end (4). whoami / read_events are read-only and
    // spawn_child is a stub that errors before apply_event. The smoke
    // therefore asserts ≥4.
    assert!(
        mcp_events >= 4,
        "expected ≥4 mcp.* events from the active calls, got {mcp_events}"
    );

    // 11. Token revoked on Ended (spec §7.2)? Subsequent call must surface
    // an unauthorized JSON-RPC error envelope (-32001). Per the transport
    // convention used here, the HTTP status stays 200 and the body carries
    // the JSON-RPC error.
    let tok_in_db: Option<String> = setup
        .db
        .pool()
        .get()
        .unwrap()
        .query_row(
            "SELECT api_token FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        tok_in_db.is_none(),
        "api_token should be NULL after session Ended (spec §7.2)"
    );

    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(
            r#"{"jsonrpc":"2.0","id":99,"method":"tools/list","params":{}}"#,
        ))
        .unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        envelope["error"]["code"], -32001,
        "expected unauthorized JSON-RPC error after session Ended; got: {envelope:?}"
    );
}
