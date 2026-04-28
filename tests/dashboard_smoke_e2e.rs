//! End-to-end smoke for the dashboard event stream.
//!
//! Spec §10.3 parity rule: every state change and operator-relevant event
//! must be observable in the dashboard WebSocket. This test asserts the
//! happy path — seed → ws open → ask_operator via MCP → operator.prompt
//! frame → answer via HTTP → mcp.ask_operator event frame.
//!
//! Mirrors the boot pattern from tests/api_event_broadcast.rs
//! (ws_emits_domain_events_as_text_frames) and the MCP envelope shape
//! from tests/mcp_smoke_endtoend.rs.
//!
//! # Empirical finding: ask_operator does NOT transition session state
//!
//! The `ask_operator` Tier-3 tool writes a `mcp.ask_operator` event row
//! (triggering `session.event_appended` on the bus) and publishes an
//! `operator.prompt` live event, but does NOT call `apply_event` with a
//! state-changing input — it parks on a oneshot and returns the operator's
//! answer. No `session.state_changed` frame is emitted around an ask.
//! The three parity assertions are therefore:
//!   1. `operator.prompt` visible on WS.
//!   2. `session.event_appended { kind: "mcp.ask_operator" }` visible on WS.
//!   3. MCP call resolves with `{ timed_out: false, answer: "yes" }`.

mod common;

use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// Post a JSON-RPC 2.0 envelope to `/mcp` over a live `reqwest::Client` and
/// return the parsed response body.
async fn rpc_live(client: &reqwest::Client, base: &str, token: &str, body: Value) -> Value {
    let resp = client
        .post(format!("{base}/mcp"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("rpc_live: POST /mcp failed");
    let status = resp.status();
    let text = resp.text().await.expect("rpc_live: body read failed");
    assert!(
        status.is_success(),
        "rpc_live: HTTP {status}: {text}"
    );
    serde_json::from_str(&text).expect("rpc_live: JSON parse failed")
}

/// Extract the inner JSON payload from `content[0].text` in a `tools/call`
/// MCP response envelope.
fn inner_text(r: &Value) -> Value {
    let text = r["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("inner_text: result.content[0].text missing or not a string; full response: {r}"));
    serde_json::from_str(text).expect("inner_text: content[0].text is not a JSON string")
}

#[tokio::test]
async fn dashboard_event_stream_ask_operator_lifecycle() {
    // -----------------------------------------------------------------------
    // 1. Boot: DB + EventBroadcaster + SessionManager with_event_bus.
    // -----------------------------------------------------------------------
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(128);
    let mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::with_event_bus(
            db.clone(),
            dir.path().to_path_buf(),
            bus.clone(),
        ),
    );
    let app = galvolite::api::router(mgr.clone());

    // -----------------------------------------------------------------------
    // 2. Bind ephemeral port and serve.
    // -----------------------------------------------------------------------
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{port}");
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // -----------------------------------------------------------------------
    // 3. Seed: project + live PTY session (cat = mock PTY command).
    // -----------------------------------------------------------------------
    let proj = mgr
        .add_project(galvolite::db::NewProject {
            name: "smoke".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(
            proj.id,
            "echo".into(),
            "implementation".into(),
            "cat".into(),
            vec![],
        )
        .await
        .unwrap();
    let sid = session.id;
    let token = format!("tok_{}", uuid::Uuid::new_v4().simple());
    db.set_api_token(sid, &token).unwrap();

    // -----------------------------------------------------------------------
    // 4. Open WebSocket to the terminal endpoint.
    // -----------------------------------------------------------------------
    let ws_url = format!("ws://127.0.0.1:{port}/api/sessions/{sid}/terminal?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("WS connect failed");

    // -----------------------------------------------------------------------
    // 5. Spawn the blocking ask_operator MCP call on a separate task.
    //    Uses timeout_secs: 10 so the test bounds its runtime even if the
    //    answer-POST path is broken.
    // -----------------------------------------------------------------------
    let client = reqwest::Client::new();
    let client2 = client.clone();
    let base2 = base_url.clone();
    let token2 = token.clone();
    let mcp_handle = tokio::spawn(async move {
        rpc_live(
            &client2,
            &base2,
            &token2,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "ask_operator",
                    "arguments": {
                        "question": "approve smoke?",
                        "timeout_secs": 10
                    }
                }
            }),
        )
        .await
    });

    // -----------------------------------------------------------------------
    // 6. Read WS frames until both parity events are observed.
    //    Stop early on operator.prompt so we can answer immediately.
    // -----------------------------------------------------------------------
    let mut prompt_id: Option<String> = None;
    let mut saw_event_appended = false;

    // Phase A: wait for operator.prompt (also collect any event_appended that
    //          arrives before or alongside it).
    let phase_a = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = match serde_json::from_str(&t) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    match v["type"].as_str() {
                        Some("operator.prompt") => {
                            let id = v["prompt_id"]
                                .as_str()
                                .expect("operator.prompt missing prompt_id")
                                .to_string();
                            prompt_id = Some(id);
                            // Check the session_id matches
                            assert_eq!(
                                v["session_id"].as_str().unwrap(),
                                sid.to_string(),
                                "operator.prompt session_id mismatch"
                            );
                            // Exit phase A — we have enough to answer
                            return;
                        }
                        Some("session.event_appended") => {
                            if v["kind"].as_str() == Some("mcp.ask_operator") {
                                saw_event_appended = true;
                            }
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Binary(_))) => continue, // PTY bytes — skip
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("WS error in phase A: {e}"),
                None => panic!("WS closed before operator.prompt"),
            }
        }
    })
    .await;
    assert!(phase_a.is_ok(), "TIMEOUT: did not receive operator.prompt within 10s");

    let pid = prompt_id.unwrap();

    // -----------------------------------------------------------------------
    // 7. POST the answer to /api/operator/prompt/:id/answer — assert 204.
    // -----------------------------------------------------------------------
    let answer_resp = client
        .post(format!("{base_url}/api/operator/prompt/{pid}/answer"))
        .header("content-type", "application/json")
        .json(&json!({ "answer": "yes" }))
        .send()
        .await
        .expect("POST /api/operator/prompt/:id/answer failed");
    assert_eq!(
        answer_resp.status().as_u16(),
        204,
        "expected 204 from operator answer endpoint"
    );

    // -----------------------------------------------------------------------
    // 8. Phase B: if we haven't already seen session.event_appended for
    //    mcp.ask_operator, keep reading until we do (or 5s timeout).
    //    The event row is written BEFORE register_and_wait parks, so it
    //    should arrive before or alongside operator.prompt — but a race
    //    (slow subscriber) means we guard here too.
    // -----------------------------------------------------------------------
    if !saw_event_appended {
        let phase_b = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match ws.next().await {
                    Some(Ok(Message::Text(t))) => {
                        let v: Value = match serde_json::from_str(&t) {
                            Ok(v) => v,
                            Err(_) => continue,
                        };
                        if v["type"].as_str() == Some("session.event_appended")
                            && v["kind"].as_str() == Some("mcp.ask_operator")
                        {
                            saw_event_appended = true;
                            return;
                        }
                    }
                    Some(Ok(Message::Binary(_))) => continue,
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => panic!("WS error in phase B: {e}"),
                    None => panic!("WS closed before session.event_appended(mcp.ask_operator)"),
                }
            }
        })
        .await;
        assert!(
            phase_b.is_ok(),
            "TIMEOUT: did not receive session.event_appended(mcp.ask_operator) within 5s"
        );
    }
    assert!(saw_event_appended, "session.event_appended {{ kind: mcp.ask_operator }} not seen");

    // -----------------------------------------------------------------------
    // 9. Await the MCP join handle and assert the answer.
    // -----------------------------------------------------------------------
    let mcp_resp = tokio::time::timeout(Duration::from_secs(5), mcp_handle)
        .await
        .expect("TIMEOUT: MCP ask_operator task did not resolve within 5s after answer-POST")
        .expect("MCP task panicked");
    let body = inner_text(&mcp_resp);
    assert_eq!(
        body["timed_out"], false,
        "expected timed_out=false; got: {body}"
    );
    assert_eq!(
        body["answer"], "yes",
        "expected answer='yes'; got: {body}"
    );
}
