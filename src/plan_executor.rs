//! Plan Executor — pure derivation of spawn graph from plan.md + directive.md.
//! Spec: docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md §3, §4 Flow A.

use crate::directive::{self, Autonomy, Directive, ParseError as DirectiveError};
use crate::plan_steps::parse_plan_markdown;

#[derive(Debug, thiserror::Error)]
pub enum DeriveError {
    #[error("directive parse: {0}")]
    Directive(#[from] DirectiveError),
    #[error("parts referenced in directive but absent from plan.md: {0:?}")]
    OrphanParts(Vec<u32>),
}

#[derive(Debug, Clone)]
pub struct SpawnGraph {
    pub phases: Vec<PhasePlan>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PhasePlan {
    pub id: String,
    pub autonomy: Autonomy,
    pub depends_on: Vec<String>,
    /// `(part_number, step_title)` pairs in the order they appear in the directive.
    pub part_titles: Vec<(u32, String)>,
}

pub fn derive_spawn_graph(plan_md: &str, directive_md: &str) -> Result<SpawnGraph, DeriveError> {
    let dir: Directive = directive::parse(directive_md)?;
    let steps = parse_plan_markdown(plan_md);

    // 1-based part lookup: ordinal+1 → title.
    let title_for_part: std::collections::HashMap<u32, &str> = steps
        .iter()
        .map(|s| ((s.ordinal as u32) + 1, s.title.as_str()))
        .collect();

    // Validate every referenced part exists.
    let mut orphans: Vec<u32> = Vec::new();
    for phase in &dir.phases {
        for p in &phase.parts {
            if !title_for_part.contains_key(p) {
                orphans.push(*p);
            }
        }
    }
    if !orphans.is_empty() {
        orphans.sort_unstable();
        orphans.dedup();
        return Err(DeriveError::OrphanParts(orphans));
    }

    // Topological sort over phase deps. (Directive parser already rejected cycles.)
    let ordered = topo_sort(&dir.phases);
    let phases = ordered
        .into_iter()
        .map(|ph| PhasePlan {
            id: ph.id.clone(),
            autonomy: ph.autonomy,
            depends_on: ph.depends_on.clone(),
            part_titles: ph
                .parts
                .iter()
                .map(|p| (*p, title_for_part[p].to_string()))
                .collect(),
        })
        .collect();

    Ok(SpawnGraph { phases })
}

// ─── execute_plan driver ───────────────────────────────────────────────────

use crate::api::events::DomainEvent;
use crate::db::Db;
use crate::session_manager::SessionManager;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error("missing artifact: {0}")]
    MissingArtifact(&'static str),
    #[error("derive: {0}")]
    Derive(#[from] DeriveError),
    #[error("no orchestrator session for project {0}")]
    NoOrchestrator(Uuid),
    #[error("orchestrator already has running children — refusing to re-execute")]
    AlreadyRunning,
    #[error("spawn failed: {0}")]
    Spawn(#[from] crate::error::Error),
}

pub async fn execute_plan(
    mgr: Arc<SessionManager>,
    project_id: Uuid,
) -> Result<Vec<Uuid>, ExecuteError> {
    let project = mgr.db().get_project(project_id)?;
    let proj_path = std::path::Path::new(&project.path);
    let plan = std::fs::read_to_string(proj_path.join(".galvolite/plan.md"))
        .map_err(|_| ExecuteError::MissingArtifact("plan.md"))?;
    let directive = std::fs::read_to_string(proj_path.join(".galvolite/directive.md"))
        .map_err(|_| ExecuteError::MissingArtifact("directive.md"))?;

    let graph = derive_spawn_graph(&plan, &directive)?;

    let orch = mgr.find_orchestrator_for_project(project_id)
        .ok_or(ExecuteError::NoOrchestrator(project_id))?;

    // Idempotency: refuse if any non-terminal child already exists for this orch.
    // Note: `list_sessions()` filters to non-terminal rows — once all children
    // have ended (terminal_status set), execute-plan can be re-issued to spawn
    // a fresh wave from the same plan.md / directive.md. Spec §4 Flow A treats
    // this as a feature: re-running the same plan after completion is supported.
    let has_running = mgr.db().list_sessions()?.iter()
        .any(|s| s.parent_session_id == Some(orch.id) && s.ended_at.is_none());
    if has_running { return Err(ExecuteError::AlreadyRunning); }

    let bus = mgr.event_bus().clone();
    let mut spawned: Vec<Uuid> = Vec::new();

    // Spawn ONLY the first phase synchronously here; subsequent phases are
    // advanced by the phase reactor (Task 6) on rejoin or operator approval.
    if let Some(first) = graph.phases.first() {
        let kids = spawn_phase(&mgr, &orch, first).await?;
        spawned.extend(kids.iter().copied());
        mgr.set_waiting_on(orch.id, &kids).await?;
        store_pending_phases(mgr.db(), orch.id, &graph.phases[1..])?;
        bus.publish(DomainEvent::PhaseAdvanced { project_id, phase_id: first.id.clone() });
        if matches!(first.autonomy, Autonomy::Review | Autonomy::Manual) {
            bus.publish(DomainEvent::PhaseReviewNeeded {
                project_id, phase_id: first.id.clone(),
            });
        }
    }
    Ok(spawned)
}

async fn spawn_phase(
    mgr: &Arc<SessionManager>,
    orch: &crate::db::Session,
    phase: &PhasePlan,
) -> Result<Vec<Uuid>, crate::error::Error> {
    let mut kids = Vec::new();
    for (part, title) in &phase.part_titles {
        let s = mgr.spawn_child(orch.id, "implementation", title, Some(*part)).await?;
        kids.push(s.id);
    }
    Ok(kids)
}

/// Persist remaining phases as JSON in the sessions table for the phase reactor.
fn store_pending_phases(
    db: &Db,
    orch_id: Uuid,
    remaining: &[PhasePlan],
) -> Result<(), crate::error::Error> {
    let json = serde_json::to_string(remaining)
        .map_err(crate::error::Error::EventPayload)?;
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE sessions SET pending_phases_json = ?1 WHERE id = ?2",
        rusqlite::params![json, orch_id.to_string()],
    )?;
    Ok(())
}

// ─── Phase reactor ────────────────────────────────────────────────────────────

/// Advance the next phase from `pending_phases_json`. Pops the head, spawns it,
/// re-saves the tail. Emits `PhaseReviewNeeded` if autonomy is non-`full`.
/// Returns `Ok(true)` if a phase was advanced, `Ok(false)` if the queue was
/// empty or the session row was not found.
pub async fn advance_next_phase(
    mgr: &Arc<SessionManager>,
    orch_id: Uuid,
    project_id: Uuid,
) -> Result<bool, crate::error::Error> {
    let lock = mgr.phase_advancement_lock();
    let _guard = lock.lock().await;

    let bus = mgr.event_bus().clone();
    let raw: Option<String> = {
        let conn = mgr.db().pool().get()?;
        conn.query_row(
            "SELECT pending_phases_json FROM sessions WHERE id = ?1",
            [orch_id.to_string()],
            |r| r.get(0),
        )
        .ok()
        .flatten()
    };
    let Some(json) = raw else { return Ok(false); };
    let mut queue: Vec<PhasePlan> = serde_json::from_str(&json).unwrap_or_default();
    if queue.is_empty() {
        return Ok(false);
    }
    let phase = queue.remove(0);
    let orch = mgr.db().get_session(orch_id)?;
    let kids = spawn_phase(mgr, &orch, &phase).await?;

    // State writes BEFORE bus.publish — invariant from execute_plan driver.
    mgr.set_waiting_on(orch_id, &kids).await?;

    let tail_json = serde_json::to_string(&queue)
        .map_err(crate::error::Error::EventPayload)?;
    {
        let conn = mgr.db().pool().get()?;
        conn.execute(
            "UPDATE sessions SET pending_phases_json = ?1 WHERE id = ?2",
            rusqlite::params![tail_json, orch_id.to_string()],
        )?;
    }

    bus.publish(DomainEvent::PhaseAdvanced {
        project_id,
        phase_id: phase.id.clone(),
    });
    if matches!(phase.autonomy, Autonomy::Review | Autonomy::Manual) {
        bus.publish(DomainEvent::PhaseReviewNeeded {
            project_id,
            phase_id: phase.id,
        });
    }
    Ok(true)
}

/// Background reactor: subscribes to `SessionChildrenResolved` events.
/// When the resolved session is the root orchestrator for a project (no parent,
/// not ended) and the next pending phase has `autonomy=full`, auto-advance.
/// For `review`/`manual` phases it leaves the queue intact and lets
/// `approve_phase` advance on the operator's explicit command.
pub fn start_phase_reactor(mgr: Arc<SessionManager>) {
    let mut rx = mgr.event_bus().subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let DomainEvent::SessionChildrenResolved { session_id, .. } = ev {
                        // Fetch the session and apply the orchestrator heuristic:
                        // top-level (no parent) + still alive (ended_at IS NULL).
                        // The DB CHECK constraint does not allow "orchestrator" as a
                        // role value — this is a runtime profile, not a stored role.
                        let orch = match mgr.db().get_session(session_id) {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if !(orch.parent_session_id.is_none() && orch.ended_at.is_none()) {
                            continue;
                        }
                        // Peek at queue head to decide whether to auto-advance.
                        let raw: Option<String> = {
                            let conn = match mgr.db().pool().get() {
                                Ok(c) => c,
                                Err(_) => continue,
                            };
                            conn.query_row(
                                "SELECT pending_phases_json FROM sessions WHERE id = ?1",
                                [session_id.to_string()],
                                |r| r.get(0),
                            )
                            .ok()
                            .flatten()
                        };
                        let Some(json) = raw else { continue };
                        let queue: Vec<PhasePlan> =
                            serde_json::from_str(&json).unwrap_or_default();
                        let Some(head) = queue.first() else { continue };
                        if head.autonomy == Autonomy::Full {
                            if let Err(e) = advance_next_phase(&mgr, session_id, orch.project_id).await {
                                tracing::error!(error = ?e, orch_id = %session_id, "phase reactor: advance_next_phase failed");
                            }
                        }
                        // review/manual: leave for operator — approve_phase will advance.
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(n, "phase reactor: lagged, dropped {n} events");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::warn!("phase reactor: bus closed, exiting");
                    break;
                }
            }
        }
    });
}

fn topo_sort(phases: &[directive::Phase]) -> Vec<&directive::Phase> {
    use std::collections::{HashMap, HashSet};
    let by_id: HashMap<&str, &directive::Phase> =
        phases.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut out: Vec<&directive::Phase> = Vec::new();
    fn visit<'a>(
        p: &'a directive::Phase,
        by_id: &HashMap<&'a str, &'a directive::Phase>,
        visited: &mut HashSet<&'a str>,
        out: &mut Vec<&'a directive::Phase>,
    ) {
        if visited.contains(p.id.as_str()) {
            return;
        }
        visited.insert(p.id.as_str());
        for dep in &p.depends_on {
            if let Some(dp) = by_id.get(dep.as_str()) {
                visit(dp, by_id, visited, out);
            }
        }
        out.push(p);
    }
    for p in phases {
        visit(p, &by_id, &mut visited, &mut out);
    }
    out
}
