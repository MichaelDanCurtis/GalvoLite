//! Operator prompt registry + HTTP answer handler.
//!
//! The MCP tool `ask_operator` parks the calling model turn on a oneshot
//! channel keyed by a fresh `prompt_id`, publishes an `operator.prompt`
//! domain event so every dashboard WS hears about it, and awaits the
//! operator's answer (or a per-call timeout).
//!
//! The dashboard (or anything with the session's API token) posts the answer
//! back to `POST /api/operator/prompt/:id/answer`. That handler looks up the
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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use uuid::Uuid;

/// Full context for a pending `ask_operator` call. Stored in the waiters map
/// so that `answer()` can persist the decision without an extra session lookup.
struct Pending {
    tx: oneshot::Sender<serde_json::Value>,
    session_id: Uuid,
    project_id: Uuid,
    project_path: PathBuf,
    question: String,
    choices: Option<Vec<String>>,
    asked_at: chrono::DateTime<chrono::Utc>,
}

/// Pending `ask_operator` calls indexed by their freshly-minted `prompt_id`.
#[derive(Default)]
pub struct OperatorPromptRegistry {
    waiters: Mutex<HashMap<Uuid, Pending>>,
}

/// RAII guard that removes a `prompt_id` from the waiters map when dropped.
/// This makes prompt-lifecycle bookkeeping robust to dropped futures (e.g. an
/// MCP client disconnects mid-call, or a timeout cancels the await), without
/// any explicit cleanup branches. On the success path the entry has already
/// been removed by `answer()`, and the guard's `remove` is a no-op.
struct PromptGuard {
    reg: Arc<OperatorPromptRegistry>,
    id: Uuid,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        // std::sync::Mutex so Drop is synchronous. Tokio runtime not required.
        let mut map = self.reg.waiters.lock().expect("waiters mutex poisoned");
        map.remove(&self.id);
    }
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
    /// Lifetime of the `prompt_id` entry in the waiters map is owned by a
    /// `PromptGuard` RAII value: it's removed either by `answer()` on the
    /// happy path, or by the guard's `Drop` on timeout / future-cancel.
    #[allow(clippy::too_many_arguments)]
    pub async fn register_and_wait(
        self: &Arc<Self>,
        bus: &EventBroadcaster,
        session_id: Uuid,
        project_id: Uuid,
        project_path: PathBuf,
        question: String,
        choices: Option<Vec<String>>,
        timeout_secs: Option<u64>,
    ) -> serde_json::Value {
        let prompt_id = Uuid::new_v4();
        let asked_at = chrono::Utc::now();
        let (tx, mut rx) = oneshot::channel();
        {
            let mut map = self.waiters.lock().expect("waiters mutex poisoned");
            map.insert(prompt_id, Pending {
                tx,
                session_id,
                project_id,
                project_path,
                question: question.clone(),
                choices: choices.clone(),
                asked_at,
            });
        }
        let _guard = PromptGuard {
            reg: self.clone(),
            id: prompt_id,
        };
        bus.publish(DomainEvent::OperatorPrompt {
            prompt_id,
            session_id,
            question,
            choices,
            timeout_secs,
        });
        let dur = std::time::Duration::from_secs(timeout_secs.unwrap_or(30));
        match tokio::time::timeout(dur, &mut rx).await {
            Ok(Ok(v)) => v,
            _ => serde_json::json!({ "answer": serde_json::Value::Null, "timed_out": true }),
        }
        // _guard drops here, removing prompt_id from waiters if still present.
    }

    /// Resolve a pending prompt with the operator's answer. Returns `Err(())`
    /// if no waiter is registered under this id (already answered, already
    /// timed out, or never existed).
    ///
    /// Side effects (in order):
    ///  1. Persist the decision to `<project.path>/.galvolite/decisions/`.
    ///  2. Publish `DomainEvent::DecisionRecorded` on the bus.
    ///  3. Unpark the waiting MCP turn via the oneshot.
    pub async fn answer(
        &self,
        prompt_id: Uuid,
        answer: serde_json::Value,
        mgr: &Arc<crate::session_manager::SessionManager>,
    ) -> Result<(), ()> {
        // Take the sender out under the lock, then release the lock BEFORE
        // calling `tx.send`. This avoids any chance of deadlock against the
        // PromptGuard's Drop, which also locks `waiters`.
        let entry = {
            let mut map = self.waiters.lock().expect("waiters mutex poisoned");
            map.remove(&prompt_id)
        };
        match entry {
            Some(p) => {
                // 1. Persist before unparking — if write fails, log + continue.
                let answer_str = answer.as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| serde_json::to_string(&answer).unwrap_or_default());
                let d = crate::decisions::Decision {
                    session_id: p.session_id,
                    asked_at: p.asked_at,
                    question: p.question,
                    answer: answer_str,
                    choices: p.choices,
                };
                match crate::decisions::record(&p.project_path, &d) {
                    Ok(path) => {
                        mgr.event_bus().publish(DomainEvent::DecisionRecorded {
                            project_id: p.project_id,
                            session_id: p.session_id,
                            path: path.to_string_lossy().into_owned(),
                        });
                    }
                    Err(e) => tracing::warn!(error = %e, "record decision failed"),
                }
                // 2. Unpark MCP turn.
                let _ = p.tx.send(serde_json::json!({ "answer": answer, "timed_out": false }));
                Ok(())
            }
            None => Err(()),
        }
    }

    /// Test-visible: how many prompts are currently registered.
    pub async fn pending_count(&self) -> usize {
        self.waiters
            .lock()
            .expect("waiters mutex poisoned")
            .len()
    }

    /// Test-visible: return any one pending prompt id, if any exist.
    pub async fn first_pending_id(&self) -> Option<Uuid> {
        self.waiters.lock().expect("waiters mutex poisoned").keys().next().copied()
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
        .answer(prompt_id, body.answer, &mgr)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|_| StatusCode::NOT_FOUND)
}
