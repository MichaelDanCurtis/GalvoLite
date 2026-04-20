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
    /// TODO(plan-4 follow-up): if the awaiting future is dropped (e.g. the
    /// MCP client disconnects mid-call) the `waiters` entry leaks until a
    /// late answer arrives. Not exercised by current tests.
    pub async fn register_and_wait(
        &self,
        bus: &EventBroadcaster,
        session_id: Uuid,
        question: String,
        choices: Option<Vec<String>>,
        timeout_secs: Option<u64>,
    ) -> serde_json::Value {
        let prompt_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().await.insert(prompt_id, tx);
        bus.publish(DomainEvent::OperatorPrompt {
            prompt_id,
            session_id,
            question,
            choices,
            timeout_secs,
        });
        let dur = std::time::Duration::from_secs(timeout_secs.unwrap_or(30));
        match tokio::time::timeout(dur, rx).await {
            Ok(Ok(v)) => v,
            _ => {
                self.waiters.lock().await.remove(&prompt_id);
                serde_json::json!({ "answer": serde_json::Value::Null, "timed_out": true })
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
