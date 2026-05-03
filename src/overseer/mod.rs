//! Overseer subsystem — per-project background task using Kimi K2.
//! See docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md §3.

pub mod config;
pub mod kimi;
pub mod observer;
pub mod recommender;

use crate::session_manager::SessionManager;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Default cadence for the lifecycle reconcile loop.
///
/// The reconcile loop polls `list_projects()` and aborts task handles for
/// projects that have been deleted. There is no `DomainEvent::ProjectDeleted`
/// today (intentional — see plan §12), so this polling fallback is the
/// authoritative lifecycle source. 30s is fine for production; tests use
/// `with_reconcile_interval` to drop it to ~50ms.
const DEFAULT_RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

/// Owns one background task per registered project. Each task runs the
/// Overseer review loop (observer → recommender → bus). The supervisor itself
/// does not subscribe to the bus; lifecycle is handled by a polling reconcile
/// loop because we have no `ProjectDeleted` domain event.
pub struct OverseerSupervisor {
    mgr: Arc<SessionManager>,
    tasks: Arc<Mutex<HashMap<Uuid, JoinHandle<()>>>>,
    reconcile_interval: Duration,
}

impl OverseerSupervisor {
    pub fn new(mgr: Arc<SessionManager>) -> Self {
        Self {
            mgr,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            reconcile_interval: DEFAULT_RECONCILE_INTERVAL,
        }
    }

    /// Override the reconcile interval. Tests use this to fire reconciliation
    /// in milliseconds instead of waiting 30 seconds.
    pub fn with_reconcile_interval(mut self, interval: Duration) -> Self {
        self.reconcile_interval = interval;
        self
    }

    pub fn task_count(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }

    /// Walk the projects table and spawn one task per project, then start the
    /// lifecycle reconcile loop. No-op if `GALVOLITE_OVERSEER` is unset or if
    /// `KIMI_API_KEY` is missing — the supervisor stays inert in either case.
    pub async fn spawn_for_existing_projects(&self) {
        if !crate::overseer::config::overseer_enabled() {
            return;
        }
        let Some(cfg) = crate::overseer::config::OverseerConfig::from_env() else {
            tracing::warn!("Overseer enabled but KIMI_API_KEY missing — skipping spawn");
            return;
        };
        let projects = self.mgr.db().list_projects().unwrap_or_default();
        for p in projects {
            self.spawn_one(p.id, cfg.clone());
        }
        self.start_lifecycle_listener();
    }

    /// Spawn an Overseer task for `project_id` if one is not already running.
    /// Synchronous: only the spawned future is async; the lock is held briefly.
    fn spawn_one(&self, project_id: Uuid, cfg: crate::overseer::config::OverseerConfig) {
        let mut t = self.tasks.lock().unwrap();
        if t.contains_key(&project_id) {
            return;
        }
        let mgr = Arc::clone(&self.mgr);
        let handle = tokio::spawn(run_overseer_loop(mgr, project_id, cfg));
        t.insert(project_id, handle);
    }

    /// Spawn the reconcile loop. The loop owns clones of `mgr` + `tasks` and
    /// runs for the lifetime of the process. Compares the live project set
    /// against tracked tasks and aborts handles for vanished projects.
    fn start_lifecycle_listener(&self) {
        let mgr = Arc::clone(&self.mgr);
        let tasks = Arc::clone(&self.tasks);
        let interval = self.reconcile_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // First tick fires immediately; skip it so we don't reconcile
            // right after spawn (no chance for any project to have been
            // deleted yet).
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let live: std::collections::HashSet<Uuid> = mgr
                    .db()
                    .list_projects()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|p| p.id)
                    .collect();
                let mut t = tasks.lock().unwrap();
                t.retain(|pid, h| {
                    if !live.contains(pid) {
                        h.abort();
                        false
                    } else {
                        true
                    }
                });
            }
        });
    }
}

/// One project's Overseer review loop. Drains bus events into a rolling
/// snapshot, periodically asks Kimi to review, and writes recommendations +
/// bus events back through the recommender.
async fn run_overseer_loop(
    mgr: Arc<SessionManager>,
    project_id: Uuid,
    cfg: crate::overseer::config::OverseerConfig,
) {
    let mut state = crate::overseer::recommender::OverseerState::new(cfg.daily_token_budget);
    let mut obs = crate::overseer::observer::Observer::new(
        project_id,
        mgr.event_bus().subscribe(),
    );
    let kimi = crate::overseer::kimi::KimiClient::new(&cfg.api_key, &cfg.base_url);

    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let mut periodic = tokio::time::interval(Duration::from_secs(600));
    // Skip the immediate first tick on both intervals — they fire instantly
    // on construction otherwise.
    interval.tick().await;
    periodic.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                obs.drain_pending(500).await;
                if obs.snapshot().events.is_empty() { continue; }
            }
            _ = periodic.tick() => {
                obs.drain_pending(500).await;
            }
        }
        let mut snap = obs.snapshot();
        if let Ok(p) = mgr.db().get_project(project_id) {
            obs.hydrate_artifacts(&mut snap, std::path::Path::new(&p.path));
        }
        let bus = mgr.event_bus().clone();
        let kc = &kimi;
        let _ = crate::overseer::recommender::review(
            mgr.db(),
            &bus,
            project_id,
            &snap,
            &mut state,
            move |s, u| async move { kc.chat(&s, &u).await },
        )
        .await;
    }
}
