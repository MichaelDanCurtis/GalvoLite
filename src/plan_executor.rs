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
