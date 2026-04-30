//! Plan-markdown parser and diff used by the `set_plan` / `plan_check_step`
//! MCP tools (spec §8.4). Pure functions — no DB or I/O.

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
}

impl PlanStepStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Pending     => "pending",
            Self::InProgress  => "in_progress",
            Self::Done        => "done",
            Self::Blocked     => "blocked",
        }
    }

    pub fn from_input_str(s: &str) -> Option<Self> {
        Some(match s {
            "pending"                  => Self::Pending,
            "working" | "in_progress"  => Self::InProgress,
            "done"                     => Self::Done,
            "blocked"                  => Self::Blocked,
            _                          => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlanStepInput {
    pub step_hash: String,
    pub title: String,
    pub status: PlanStepStatus,
    pub ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct DbPlanStep {
    pub step_hash: String,
    pub title: String,
    pub status: String,
    pub ordinal: i64,
}

#[derive(Debug, Default)]
pub struct PlanStepDiff {
    pub added:   Vec<PlanStepInput>,
    pub removed: Vec<String>, // step_hash
    pub changed: Vec<PlanStepStatusChange>,
}

#[derive(Debug, Clone)]
pub struct PlanStepStatusChange {
    pub step_hash:    String,
    pub prev_status:  String,
    pub next_status:  String,
}

/// Hash a normalized title — collapse runs of whitespace, trim, lowercase
/// not applied (titles are case-sensitive). 16 hex chars from sha256.
pub fn step_hash_for(title: &str) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    let bytes = h.finalize();
    hex::encode(&bytes[..8])
}

/// Parse markdown for `- [ ]` / `- [x]` / `- [X]` checkbox items. Each match
/// becomes one `PlanStepInput` with stable hash and source-order ordinal.
pub fn parse_plan_markdown(md: &str) -> Vec<PlanStepInput> {
    let mut out = Vec::new();
    let mut ord: i64 = 0;
    for line in md.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("- [") else { continue };
        if rest.len() < 2 { continue }
        let mark = rest.as_bytes()[0] as char;
        if rest.as_bytes()[1] != b']' { continue }
        let after = &rest[2..];
        let title = after.trim();
        if title.is_empty() { continue }
        let status = match mark {
            ' '       => PlanStepStatus::Pending,
            'x' | 'X' => PlanStepStatus::Done,
            _         => continue,
        };
        out.push(PlanStepInput {
            step_hash: step_hash_for(title),
            title:     title.to_string(),
            status,
            ordinal:   ord,
        });
        ord += 1;
    }
    out
}

/// Diff prior DB rows against parsed input. `added`/`removed` are by hash;
/// `changed` covers any step whose status differs from prior. Ordinal moves
/// are NOT reported as `changed` — only the WS event for status shifts.
pub fn diff_plan_steps(prior: &[DbPlanStep], next: &[PlanStepInput]) -> PlanStepDiff {
    let mut diff = PlanStepDiff::default();
    let prior_idx: std::collections::HashMap<&str, &DbPlanStep> =
        prior.iter().map(|p| (p.step_hash.as_str(), p)).collect();
    let next_idx: std::collections::HashSet<&str> =
        next.iter().map(|n| n.step_hash.as_str()).collect();

    for n in next {
        match prior_idx.get(n.step_hash.as_str()) {
            None => diff.added.push(n.clone()),
            Some(p) => {
                let next_db = n.status.as_db_str();
                if p.status != next_db {
                    diff.changed.push(PlanStepStatusChange {
                        step_hash:   n.step_hash.clone(),
                        prev_status: p.status.clone(),
                        next_status: next_db.to_string(),
                    });
                }
            }
        }
    }
    for p in prior {
        if !next_idx.contains(p.step_hash.as_str()) {
            diff.removed.push(p.step_hash.clone());
        }
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unchecked_and_checked_items_in_order() {
        let md = "# Plan\n\n- [ ] First step\n- [x] Second step\n- [X] Third step\nignored line\n";
        let steps = parse_plan_markdown(md);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].title, "First step");
        assert_eq!(steps[0].status, PlanStepStatus::Pending);
        assert_eq!(steps[0].ordinal, 0);
        assert_eq!(steps[1].status, PlanStepStatus::Done);
        assert_eq!(steps[1].ordinal, 1);
        assert_eq!(steps[2].status, PlanStepStatus::Done);
        assert_eq!(steps[2].ordinal, 2);
    }

    #[test]
    fn step_hash_is_stable_across_whitespace_normalization() {
        let a = parse_plan_markdown("- [ ]   Hello   world  \n");
        let b = parse_plan_markdown("- [ ] Hello world\n");
        assert_eq!(a[0].step_hash, b[0].step_hash);
        assert_eq!(a[0].step_hash.len(), 16); // sha256[:16]
    }

    #[test]
    fn diff_detects_added_removed_status_changed() {
        let prior = vec![
            DbPlanStep { step_hash: "h1".into(), title: "Keep".into(), status: "pending".into(), ordinal: 0 },
            DbPlanStep { step_hash: "h2".into(), title: "Drop".into(), status: "pending".into(), ordinal: 1 },
        ];
        let next  = vec![
            PlanStepInput { step_hash: "h1".into(), title: "Keep".into(), status: PlanStepStatus::Done, ordinal: 0 },
            PlanStepInput { step_hash: "h3".into(), title: "Add".into(),  status: PlanStepStatus::Pending, ordinal: 1 },
        ];
        let diff = diff_plan_steps(&prior, &next);
        let changed: Vec<_> = diff.changed.iter().map(|c| (&c.step_hash[..], &c.prev_status[..], &c.next_status[..])).collect();
        assert!(changed.contains(&("h1", "pending", "done")));
        assert!(diff.added.iter().any(|s| s.step_hash == "h3"));
        assert!(diff.removed.iter().any(|s| s == "h2"));
    }
}
