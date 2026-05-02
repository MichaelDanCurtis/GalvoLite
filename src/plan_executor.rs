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

#[derive(Debug, Clone)]
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
    let serializable: Vec<_> = remaining.iter().map(|p| serde_json::json!({
        "id": p.id,
        "autonomy": match p.autonomy {
            Autonomy::Full => "full",
            Autonomy::Review => "review",
            Autonomy::Manual => "manual",
        },
        "depends_on": p.depends_on,
        "part_titles": p.part_titles,
    })).collect();
    let json = serde_json::to_string(&serializable)
        .map_err(crate::error::Error::EventPayload)?;
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE sessions SET pending_phases_json = ?1 WHERE id = ?2",
        rusqlite::params![json, orch_id.to_string()],
    )?;
    Ok(())
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
