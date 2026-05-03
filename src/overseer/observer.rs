//! Bus subscriber that maintains a rolling `ContextSnapshot` for one project.
//! Read-only on the bus.

use crate::api::events::DomainEvent;
use std::collections::VecDeque;
use tokio::sync::broadcast;
use uuid::Uuid;

pub const EVENT_RING_CAP: usize = 200;
pub const PTY_TAIL_LINE_CAP: usize = 50;

#[derive(Debug, Clone)]
pub struct EventRecord {
    pub session_id: Uuid,
    pub kind: String,
    pub source: String,
    pub ts: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct ContextSnapshot {
    pub project_id: Option<Uuid>,
    pub events: Vec<EventRecord>,
    pub plan_md: Option<String>,
    pub directive_md: Option<String>,
    pub recent_postmortems: Vec<crate::postmortem::Postmortem>,
    pub recent_decisions: Vec<crate::decisions::Decision>,
}

pub struct Observer {
    project_id: Uuid,
    rx: broadcast::Receiver<DomainEvent>,
    events: VecDeque<EventRecord>,
}

impl Observer {
    pub fn new(project_id: Uuid, rx: broadcast::Receiver<DomainEvent>) -> Self {
        Self { project_id, rx, events: VecDeque::with_capacity(EVENT_RING_CAP) }
    }

    pub fn project_id(&self) -> Uuid { self.project_id }

    /// Drain up to `max` queued events from the broadcast receiver into the ring.
    /// Non-blocking; returns count drained. Lagged frames are logged and skipped.
    pub async fn drain_pending(&mut self, max: usize) -> usize {
        use tokio::sync::broadcast::error::TryRecvError;
        let mut n = 0;
        for _ in 0..max {
            match self.rx.try_recv() {
                Ok(ev) => {
                    if let Some(rec) = filter_event(ev, self.project_id) {
                        if self.events.len() == EVENT_RING_CAP { self.events.pop_front(); }
                        self.events.push_back(rec);
                        n += 1;
                    }
                }
                Err(TryRecvError::Lagged(dropped)) => {
                    tracing::warn!(dropped = dropped, "overseer observer: bus lagged, skipping {dropped} events");
                    continue;
                }
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        n
    }

    pub fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot {
            project_id: Some(self.project_id),
            events: self.events.iter().cloned().collect(),
            plan_md: None,
            directive_md: None,
            recent_postmortems: vec![],
            recent_decisions: vec![],
        }
    }

    /// Hydrate snapshot with on-disk artifacts. Best-effort: missing files
    /// leave the corresponding snapshot field empty.
    pub fn hydrate_artifacts(&self, snap: &mut ContextSnapshot, project_path: &std::path::Path) {
        snap.plan_md = std::fs::read_to_string(project_path.join(".galvolite/plan.md")).ok();
        snap.directive_md = std::fs::read_to_string(project_path.join(".galvolite/directive.md")).ok();
        if let Ok(mut pms) = crate::postmortem::list_all(project_path) {
            pms.truncate(5);
            snap.recent_postmortems = pms;
        }
        if let Ok(decs) = crate::decisions::list_recent(project_path, 10) {
            snap.recent_decisions = decs;
        }
    }
}

fn filter_event(ev: DomainEvent, _project_id: Uuid) -> Option<EventRecord> {
    // The bus is global; we cannot per-project filter here without a project_id
    // on every event. For now, accept all event variants the Overseer cares about.
    match ev {
        DomainEvent::SessionEventAppended { session_id, kind, source, ts, payload, .. } =>
            Some(EventRecord { session_id, kind, source, ts, payload }),
        _ => None,
    }
}
