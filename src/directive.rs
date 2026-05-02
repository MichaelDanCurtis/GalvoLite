//! Parser for `.galvolite/directive.md` — the orchestration contract.
//!
//! This module is pure: `parse()` operates on a `&str` with no I/O.
//! Atomic file writing is handled by Task 2.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// ── Public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct Directive {
    pub name: String,
    pub plan_path: PathBuf,
    pub phases: Vec<Phase>,
    pub escalation_rules: Vec<EscalationRule>,
    pub notes: String,
    pub raw_markdown: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Phase {
    pub id: String,
    pub parts: Vec<u32>,
    pub autonomy: Autonomy,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Autonomy {
    Full,
    Review,
    Manual,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EscalationRule {
    pub trigger: String,
    pub action: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("missing required section: {0}")]
    MissingSection(&'static str),
    #[error("malformed phase row: {0}")]
    BadPhaseRow(String),
    #[error("unknown autonomy level: {0} (expected: full | review | manual)")]
    BadAutonomy(String),
    #[error("phase {phase} depends on undefined phase {dep}")]
    UndefinedDep { phase: String, dep: String },
    #[error("circular dependency detected involving {0}")]
    CircularDep(String),
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Parse a directive markdown string into a [`Directive`].
///
/// No I/O is performed; the caller supplies the full file contents as `s`.
pub fn parse(s: &str) -> Result<Directive, ParseError> {
    let name = extract_title(s)?;
    let plan_path = extract_plan_path(s)?;
    let phases = parse_phases(s)?;
    let escalation_rules = parse_escalation(s);
    let notes = extract_section(s, "## Notes").trim().to_string();

    validate_deps(&phases)?;

    Ok(Directive {
        name,
        plan_path,
        phases,
        escalation_rules,
        notes,
        raw_markdown: s.to_string(),
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract the directive name from `# Directive: <name>`.
fn extract_title(s: &str) -> Result<String, ParseError> {
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("# Directive:") {
            return Ok(rest.trim().to_string());
        }
    }
    Err(ParseError::MissingSection("# Directive: <name>"))
}

/// Extract the plan path from `Plan: <path>`.
fn extract_plan_path(s: &str) -> Result<PathBuf, ParseError> {
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("Plan:") {
            return Ok(PathBuf::from(rest.trim()));
        }
    }
    Err(ParseError::MissingSection("Plan:"))
}

/// Return everything after `heading` up to the next `##` section (or EOF).
fn extract_section<'a>(s: &'a str, heading: &str) -> &'a str {
    if let Some(start) = s.find(heading) {
        let after_heading = &s[start + heading.len()..];
        // Advance past the rest of the heading line.
        let body_start = after_heading
            .find('\n')
            .map(|i| i + 1)
            .unwrap_or(after_heading.len());
        let body = &after_heading[body_start..];
        // Stop at the next `##` section.
        if let Some(next_section) = body.find("\n##") {
            &body[..next_section]
        } else {
            body
        }
    } else {
        ""
    }
}

/// Parse the `## Phases` table into a `Vec<Phase>`.
fn parse_phases(s: &str) -> Result<Vec<Phase>, ParseError> {
    let section = extract_section(s, "## Phases");
    if section.is_empty() {
        return Err(ParseError::MissingSection("## Phases"));
    }

    let mut phases = Vec::new();
    let mut rows_seen = 0usize;

    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') || !line.ends_with('|') {
            continue;
        }
        // Split on `|`, drop empty leading/trailing cells.
        let cells: Vec<&str> = line
            .split('|')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();

        if cells.len() < 4 {
            continue;
        }

        rows_seen += 1;
        // Row 1 = header, Row 2 = separator (`---`), Row 3+ = data.
        if rows_seen <= 2 {
            continue;
        }

        let id = cells[0].to_string();
        let parts = parse_parts(cells[1], &id)?;
        let autonomy = parse_autonomy(cells[2])?;
        let depends_on = parse_depends_on(cells[3]);

        phases.push(Phase { id, parts, autonomy, depends_on });
    }

    Ok(phases)
}

/// Parse a comma-separated parts list like `"2, 3"` into `Vec<u32>`.
fn parse_parts(cell: &str, phase_id: &str) -> Result<Vec<u32>, ParseError> {
    cell.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u32>().map_err(|_| {
                ParseError::BadPhaseRow(format!(
                    "phase {}: cannot parse part number {:?}",
                    phase_id, s
                ))
            })
        })
        .collect()
}

/// Map `"full" | "review" | "manual"` to [`Autonomy`].
fn parse_autonomy(cell: &str) -> Result<Autonomy, ParseError> {
    match cell.trim().to_lowercase().as_str() {
        "full" => Ok(Autonomy::Full),
        "review" => Ok(Autonomy::Review),
        "manual" => Ok(Autonomy::Manual),
        other => Err(ParseError::BadAutonomy(other.to_string())),
    }
}

/// Parse the `Depends on` cell: `—`, `-`, or empty → empty vec; otherwise split on `,`.
fn parse_depends_on(cell: &str) -> Vec<String> {
    let trimmed = cell.trim();
    // Em-dash (—), ASCII dash alone, or empty means no deps.
    if trimmed.is_empty() || trimmed == "—" || trimmed == "-" {
        return vec![];
    }
    trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse `## Escalation rules` bullets.
///
/// Each line has the form: `- On <trigger> → <action>`.
/// The delimiter is the UTF-8 en-dash `→` (U+2192, RIGHT ARROW).
fn parse_escalation(s: &str) -> Vec<EscalationRule> {
    let section = extract_section(s, "## Escalation rules");
    let mut rules = Vec::new();

    for line in section.lines() {
        let line = line.trim();
        // Expect lines starting with `- On `.
        let Some(rest) = line.strip_prefix("- On ") else { continue };
        // Split on `→` (U+2192).
        let Some((trigger_raw, action_raw)) = rest.split_once('→') else { continue };
        rules.push(EscalationRule {
            trigger: trigger_raw.trim().to_string(),
            action: action_raw.trim().to_string(),
        });
    }

    rules
}

/// Validate that every `depends_on` ID names a real phase, and detect cycles via DFS.
fn validate_deps(phases: &[Phase]) -> Result<(), ParseError> {
    let ids: HashSet<&str> = phases.iter().map(|p| p.id.as_str()).collect();

    // Build adjacency map for DFS.
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    for phase in phases {
        for dep in &phase.depends_on {
            if !ids.contains(dep.as_str()) {
                return Err(ParseError::UndefinedDep {
                    phase: phase.id.clone(),
                    dep: dep.clone(),
                });
            }
        }
        graph.insert(
            phase.id.as_str(),
            phase.depends_on.iter().map(String::as_str).collect(),
        );
    }

    // DFS cycle detection: grey = in current stack, black = fully visited.
    let mut grey: HashSet<&str> = HashSet::new();
    let mut black: HashSet<&str> = HashSet::new();

    for phase in phases {
        if !black.contains(phase.id.as_str()) {
            dfs_cycle(phase.id.as_str(), &graph, &mut grey, &mut black)?;
        }
    }

    Ok(())
}

/// Recursive DFS helper; returns `Err(CircularDep)` on a back-edge.
fn dfs_cycle<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    grey: &mut HashSet<&'a str>,
    black: &mut HashSet<&'a str>,
) -> Result<(), ParseError> {
    grey.insert(node);
    if let Some(neighbours) = graph.get(node) {
        for &dep in neighbours {
            if grey.contains(dep) {
                return Err(ParseError::CircularDep(dep.to_string()));
            }
            if !black.contains(dep) {
                dfs_cycle(dep, graph, grey, black)?;
            }
        }
    }
    grey.remove(node);
    black.insert(node);
    Ok(())
}
