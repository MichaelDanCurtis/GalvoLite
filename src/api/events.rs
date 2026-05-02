//! Domain-event broadcast hub for the dashboard WebSocket.
//!
//! Spec §10.1: every coordination-layer event (state change, event row append,
//! role change, plan replacement, plan step change, meta change, operator
//! prompt, operator toast) is fanned out to subscribed dashboard sockets as a
//! tagged JSON frame.

use crate::state::SessionState;
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainEvent {
    #[serde(rename = "session.state_changed")]
    SessionStateChanged {
        session_id: Uuid,
        prev: Option<SessionState>,
        next: SessionState,
        last_event_ts: String,
    },
    #[serde(rename = "session.event_appended")]
    SessionEventAppended {
        session_id: Uuid,
        event_id: i64,
        kind: String,
        source: String,
        ts: String,
        payload: serde_json::Value,
    },
    #[serde(rename = "session.role_changed")]
    SessionRoleChanged { session_id: Uuid, role: String },
    #[serde(rename = "session.meta_changed")]
    SessionMetaChanged {
        session_id: Uuid,
        turn_count: i64,
        est_tokens: i64,
        plan_markdown: Option<String>,
        seed_markdown: Option<String>,
        handoff_path: Option<String>,
    },
    #[serde(rename = "session.plan_replaced")]
    SessionPlanReplaced { session_id: Uuid, steps: serde_json::Value },
    #[serde(rename = "session.plan_step_changed")]
    SessionPlanStepChanged {
        session_id: Uuid,
        step_hash: String,
        prev_status: String,
        next_status: String,
        note: Option<String>,
    },
    #[serde(rename = "operator.prompt")]
    OperatorPrompt {
        prompt_id: Uuid,
        session_id: Uuid,
        question: String,
        choices: Option<Vec<String>>,
        timeout_secs: Option<u64>,
    },
    #[serde(rename = "operator.toast")]
    OperatorToast { session_id: Uuid, level: String, message: String },
    #[serde(rename = "session.resumed")]
    SessionResumed { session_id: Uuid, last_events: serde_json::Value },
    #[serde(rename = "session.children_resolved")]
    SessionChildrenResolved {
        session_id: Uuid,
        children: Vec<ChildResolution>,
    },
    #[serde(rename = "directive.changed")]
    DirectiveChanged { project_id: Uuid },
}

#[derive(Debug, Clone, Serialize)]
pub struct ChildResolution {
    pub id: Uuid,
    pub role: String,
    pub status: String,
    pub handoff: Option<String>,
}

#[derive(Clone)]
pub struct EventBroadcaster {
    tx: broadcast::Sender<DomainEvent>,
}

impl EventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.tx.subscribe()
    }
    /// Best-effort publish. Lagged subscribers drop frames; that is acceptable
    /// because the dashboard always re-fetches on reconnect.
    pub fn publish(&self, evt: DomainEvent) {
        let _ = self.tx.send(evt);
    }
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

impl DomainEvent {
    /// True if this event is scoped to (or fan-outs to) the given session.
    /// The dashboard WebSocket uses this to filter the global event bus down
    /// to the per-session text-frame channel. The match is exhaustive (no
    /// wildcard arm) so future variants force a compile error here.
    pub fn applies_to(&self, sid: Uuid) -> bool {
        match self {
            DomainEvent::SessionStateChanged { session_id, .. }
            | DomainEvent::SessionEventAppended { session_id, .. }
            | DomainEvent::SessionRoleChanged { session_id, .. }
            | DomainEvent::SessionMetaChanged { session_id, .. }
            | DomainEvent::SessionPlanReplaced { session_id, .. }
            | DomainEvent::SessionPlanStepChanged { session_id, .. }
            | DomainEvent::OperatorPrompt { session_id, .. }
            | DomainEvent::OperatorToast { session_id, .. }
            | DomainEvent::SessionResumed { session_id, .. }
            | DomainEvent::SessionChildrenResolved { session_id, .. } => *session_id == sid,
            DomainEvent::DirectiveChanged { .. } => false,
        }
    }
}
