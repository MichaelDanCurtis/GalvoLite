//! Bus subscriber that maintains a rolling `ContextSnapshot` for one project.
//! Read-only on the bus.

use crate::api::events::DomainEvent;
use std::collections::VecDeque;
use tokio::sync::broadcast;
use uuid::Uuid;

pub const EVENT_RING_CAP: usize = 200;
pub const PTY_TAIL_LINE_CAP: usize = 50;

/// Plan 9 Task 4 — cap per-session PTY tail length at insertion time so
/// the snapshot's heap footprint scales with min(N×cap), not with the
/// rendered screen size. The packer's 800-char cap is a second line of
/// defence; this cap protects peak memory before the packer runs.
pub const PTY_TAIL_HYDRATE_CAP: usize = 1200;

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
    pub tail_per_session: std::collections::HashMap<Uuid, String>,
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
            tail_per_session: std::collections::HashMap::new(),
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

    /// Plan 9 Task 4 — populate `tail_per_session` for each running session
    /// in this Observer's project. Reads the rendered PTY screen via
    /// `SessionManager::pty_snapshot`. Sessions not currently running are
    /// omitted (no entry in the map). Best-effort: a UTF-8 decode failure
    /// is replaced with the lossy form rather than dropping the session.
    /// Each tail is truncated to `PTY_TAIL_HYDRATE_CAP` bytes at a UTF-8
    /// boundary so the snapshot's heap footprint scales with min(N×cap).
    pub fn hydrate_pty_tails(
        &self,
        snap: &mut ContextSnapshot,
        mgr: &crate::session_manager::SessionManager,
    ) {
        let sessions = mgr.list_sessions().unwrap_or_default();
        for s in sessions.into_iter().filter(|s| s.project_id == self.project_id) {
            if let Some(bytes) = mgr.pty_snapshot(s.id) {
                let mut text = String::from_utf8_lossy(&bytes).into_owned();
                if text.len() > PTY_TAIL_HYDRATE_CAP {
                    // Truncate at a UTF-8 boundary at or below the cap.
                    let mut cut = PTY_TAIL_HYDRATE_CAP;
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    text.truncate(cut);
                }
                snap.tail_per_session.insert(s.id, text);
            }
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
