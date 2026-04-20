//! Operator prompt registry + HTTP answer handler.
//!
//! The MCP tool `ask_operator` parks the calling model turn on a oneshot
//! channel keyed by a fresh `prompt_id`, publishes an `operator.prompt`
//! domain event so every dashboard WS hears about it, and awaits the
//! operator's answer (or a per-call timeout).
//!
//! The dashboard (or anything with the session's API token) posts the answer
//! back to `POST /operator/prompt/:id/answer`. That handler looks up the
//! registered oneshot and hands the answer JSON across, unparking the MCP
//! turn. Unknown prompt ids yield 404.
//!
//! The registry is owned by the `SessionManager` so all three callers — the
//! MCP handler, the HTTP answer route, and future recovery code — share the
//! same pending-map.

use crate::api::events::{DomainEvent, EventBroadcaster};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

/// Pending `ask_operator` calls indexed by their freshly-minted `prompt_id`.
#[derive(Default)]
pub struct OperatorPromptRegistry {
    waiters: Mutex<HashMap<Uuid, oneshot::Sender<serde_json::Value>>>,
}

impl OperatorPromptRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a new prompt, publish the `operator.prompt` domain event, and
    /// block the caller until either an HTTP answer lands or `timeout_secs`
    /// elapses. Returns `{answer, timed_out}` JSON either way — the MCP tool
    /// layer passes this straight through to `ToolOutcome::Ok`.
    ///
    // TODO(plan-4 follow-up): If this ask_operator future is dropped before
    // register_and_wait resolves (e.g. MCP client disconnect), the waiters-map
    // entry leaks indefinitely. Fix by wrapping the map insertion in a RAII
    // guard whose Drop removes the entry — e.g., a `PromptGuard { reg, id }`
    // that synchronously removes on drop. Combining with the Issue 1 race fix
    // gives a single authoritative ownership model for prompt_id lifetime.
    pub async fn register_and_wait(
        &self,
        bus: &EventBroadcaster,
        session_id: Uuid,
        question: String,
        choices: Option<Vec<String>>,
        timeout_secs: Option<u64>,
    ) -> serde_json::Value {
        let prompt_id = Uuid::new_v4();
        let (tx, mut rx) = oneshot::channel();
        self.waiters.lock().await.insert(prompt_id, tx);
        bus.publish(DomainEvent::OperatorPrompt {
            prompt_id,
            session_id,
            question,
            choices,
            timeout_secs,
        });
        let dur = std::time::Duration::from_secs(timeout_secs.unwrap_or(30));
        let result = tokio::time::timeout(dur, &mut rx).await;
        match result {
            Ok(Ok(v)) => v,
            _ => {
                // Either elapsed, or the sender was dropped without sending.
                // Atomic race resolution: if the waiter is still registered,
                // it's a genuine timeout. If not, a concurrent answer() raced
                // us; drain rx non-blockingly to recover the value it sent.
                let removed = self.waiters.lock().await.remove(&prompt_id).is_some();
                if removed {
                    serde_json::json!({ "answer": serde_json::Value::Null, "timed_out": true })
                } else {
                    match rx.try_recv() {
                        Ok(v) => v,
                        Err(_) => {
                            serde_json::json!({ "answer": serde_json::Value::Null, "timed_out": true })
                        }
                    }
                }
            }
        }
    }

    /// Resolve a pending prompt with the operator's answer. Returns `Err(())`
    /// if no waiter is registered under this id (already answered, already
    /// timed out, or never existed).
    pub async fn answer(
        &self,
        prompt_id: Uuid,
        answer: serde_json::Value,
    ) -> Result<(), ()> {
        if let Some(tx) = self.waiters.lock().await.remove(&prompt_id) {
            let _ = tx.send(serde_json::json!({ "answer": answer, "timed_out": false }));
            Ok(())
        } else {
            Err(())
        }
    }
}

#[derive(serde::Deserialize)]
pub struct AnswerBody {
    pub answer: serde_json::Value,
}

pub async fn answer(
    State(mgr): State<Arc<crate::session_manager::SessionManager>>,
    Path(prompt_id): Path<Uuid>,
    Json(body): Json<AnswerBody>,
) -> Result<StatusCode, StatusCode> {
    mgr.operator_registry()
        .answer(prompt_id, body.answer)
        .await
        .map(|_| StatusCode::OK)
        .map_err(|_| StatusCode::NOT_FOUND)
}
