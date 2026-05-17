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
    #[serde(rename = "postmortem.written")]
    PostmortemWritten {
        project_id: Uuid,
        plan_part: u32,
        path: String,
    },
    #[serde(rename = "decision.recorded")]
    DecisionRecorded {
        project_id: Uuid,
        session_id: Uuid,
        path: String,
    },
    #[serde(rename = "phase.review_needed")]
    PhaseReviewNeeded { project_id: Uuid, phase_id: String },
    #[serde(rename = "phase.advanced")]
    PhaseAdvanced { project_id: Uuid, phase_id: String },
    #[serde(rename = "recommendation.created")]
    RecommendationCreated { rec_id: Uuid, project_id: Uuid, severity: String },
    #[serde(rename = "recommendation.acted")]
    RecommendationActed { rec_id: Uuid, action: String, route: Option<Uuid> },
    #[serde(rename = "overseer.degraded")]
    OverseerDegraded { project_id: Uuid, reason: String },
    #[serde(rename = "overseer.recovered")]
    OverseerRecovered { project_id: Uuid },
    #[serde(rename = "session.pr_created")]
    SessionPrCreated {
        session_id: Uuid,
        project_id: Uuid,
        pr_url: String,
        pr_number: u64,
    },
    #[serde(rename = "project.added")]
    ProjectAdded { project_id: Uuid },
    #[serde(rename = "project.deleted")]
    ProjectDeleted { project_id: Uuid },
    #[serde(rename = "github.pull_request_merged")]
    PullRequestMerged {
        project_id: Uuid,
        pr_url: String,
        pr_number: u64,
    },
    #[serde(rename = "github.issue_opened")]
    IssueOpened {
        project_id: Uuid,
        issue_url: String,
        issue_number: u64,
        title: String,
    },
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
            | DomainEvent::SessionChildrenResolved { session_id, .. }
            | DomainEvent::SessionPrCreated { session_id, .. } => *session_id == sid,
            DomainEvent::DirectiveChanged { .. }
            | DomainEvent::PostmortemWritten { .. }
            | DomainEvent::DecisionRecorded { .. }
            | DomainEvent::PhaseReviewNeeded { .. }
            | DomainEvent::PhaseAdvanced { .. }
            | DomainEvent::RecommendationCreated { .. }
            | DomainEvent::RecommendationActed { .. }
            | DomainEvent::OverseerDegraded { .. }
            | DomainEvent::OverseerRecovered { .. }
            | DomainEvent::ProjectAdded { .. }
            | DomainEvent::ProjectDeleted { .. }
            | DomainEvent::PullRequestMerged { .. }
            | DomainEvent::IssueOpened { .. } => false,
        }
    }

    /// True if this event is scoped to (or fan-outs to) the given project.
    /// The per-project dashboard WebSocket uses this to filter the global
    /// event bus down to the per-project text-frame channel. The match is
    /// exhaustive (no wildcard arm) so future variants force a compile error
    /// here — opt in by extracting `project_id` from the payload.
    pub fn applies_to_project(&self, pid: Uuid) -> bool {
        match self {
            DomainEvent::IssueOpened { project_id, .. }
            | DomainEvent::PullRequestMerged { project_id, .. }
            | DomainEvent::SessionPrCreated { project_id, .. }
            | DomainEvent::ProjectAdded { project_id, .. }
            | DomainEvent::ProjectDeleted { project_id, .. }
            | DomainEvent::OverseerDegraded { project_id, .. }
            | DomainEvent::OverseerRecovered { project_id, .. }
            | DomainEvent::DirectiveChanged { project_id, .. }
            | DomainEvent::PostmortemWritten { project_id, .. }
            | DomainEvent::DecisionRecorded { project_id, .. }
            | DomainEvent::PhaseReviewNeeded { project_id, .. }
            | DomainEvent::PhaseAdvanced { project_id, .. }
            | DomainEvent::RecommendationCreated { project_id, .. } => *project_id == pid,
            DomainEvent::SessionStateChanged { .. }
            | DomainEvent::SessionEventAppended { .. }
            | DomainEvent::SessionRoleChanged { .. }
            | DomainEvent::SessionMetaChanged { .. }
            | DomainEvent::SessionPlanReplaced { .. }
            | DomainEvent::SessionPlanStepChanged { .. }
            | DomainEvent::OperatorPrompt { .. }
            | DomainEvent::OperatorToast { .. }
            | DomainEvent::SessionResumed { .. }
            | DomainEvent::SessionChildrenResolved { .. }
            | DomainEvent::RecommendationActed { .. } => false,
        }
    }
}

#[cfg(test)]
mod applies_to_project_tests {
    use super::*;

    fn p() -> Uuid { Uuid::new_v4() }
    fn s() -> Uuid { Uuid::new_v4() }

    #[test]
    fn matches_issue_opened_on_same_project_id() {
        let pid = p();
        let evt = DomainEvent::IssueOpened {
            project_id: pid, issue_url: "u".into(), issue_number: 1, title: "t".into(),
        };
        assert!(evt.applies_to_project(pid));
        assert!(!evt.applies_to_project(Uuid::new_v4()));
    }

    #[test]
    fn matches_pull_request_merged_on_same_project_id() {
        let pid = p();
        let evt = DomainEvent::PullRequestMerged {
            project_id: pid, pr_url: "u".into(), pr_number: 1,
        };
        assert!(evt.applies_to_project(pid));
        assert!(!evt.applies_to_project(Uuid::new_v4()));
    }

    #[test]
    fn matches_session_pr_created_dual_scoped() {
        let pid = p();
        let sid = s();
        let evt = DomainEvent::SessionPrCreated {
            session_id: sid, project_id: pid, pr_url: "u".into(), pr_number: 1,
        };
        // Project-scoped match
        assert!(evt.applies_to_project(pid));
        // Session-scoped match (existing behavior preserved)
        assert!(evt.applies_to(sid));
        // Neither matches
        assert!(!evt.applies_to_project(Uuid::new_v4()));
        assert!(!evt.applies_to(Uuid::new_v4()));
    }

    #[test]
    fn matches_project_lifecycle_variants() {
        let pid = p();
        assert!(DomainEvent::ProjectAdded { project_id: pid }.applies_to_project(pid));
        assert!(DomainEvent::ProjectDeleted { project_id: pid }.applies_to_project(pid));
        assert!(DomainEvent::OverseerDegraded { project_id: pid, reason: "r".into() }
            .applies_to_project(pid));
        assert!(DomainEvent::OverseerRecovered { project_id: pid }.applies_to_project(pid));
    }

    #[test]
    fn does_not_match_session_only_variants() {
        let pid = p();
        let sid = s();
        let evt = DomainEvent::SessionRoleChanged { session_id: sid, role: "implementation".into() };
        assert!(!evt.applies_to_project(pid));
        let evt2 = DomainEvent::OperatorToast { session_id: sid, level: "info".into(), message: "x".into() };
        assert!(!evt2.applies_to_project(pid));
    }
}
