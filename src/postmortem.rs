//! Flat-file postmortem storage under `<project.path>/.galvolite/postmortems/`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Postmortem {
    pub plan_part: u32,
    pub session_id: Uuid,
    pub outcome: Outcome,
    pub cycles: u32,
    pub duration_seconds: u64,
    pub written_at: DateTime<Utc>,
    pub what_worked: String,
    pub gotchas: String,
    pub follow_ups: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Outcome { Shipped, Stuck, Escalated }

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Shipped => write!(f, "shipped"),
            Outcome::Stuck => write!(f, "stuck"),
            Outcome::Escalated => write!(f, "escalated"),
        }
    }
}

impl Postmortem {
    pub fn render(&self) -> String {
        format!(
"---
plan_part: {part}
session_id: {sid}
outcome: {outcome}
cycles: {cycles}
duration_seconds: {dur}
written_at: {ts}
---

## What worked

{worked}

## Gotchas

{gotchas}

## Follow-ups

{follow}
",
            part = self.plan_part, sid = self.session_id,
            outcome = self.outcome, cycles = self.cycles,
            dur = self.duration_seconds, ts = self.written_at.to_rfc3339(),
            worked = self.what_worked, gotchas = self.gotchas, follow = self.follow_ups,
        )
    }
}

pub fn postmortems_dir(project_path: &Path) -> PathBuf {
    project_path.join(".galvolite").join("postmortems")
}

pub fn postmortem_path(project_path: &Path, part: u32, sid: Uuid) -> PathBuf {
    postmortems_dir(project_path).join(format!("part-{part}-{sid}.md"))
}

pub fn write(project_path: &Path, pm: &Postmortem) -> std::io::Result<PathBuf> {
    let dir = postmortems_dir(project_path);
    std::fs::create_dir_all(&dir)?;
    let path = postmortem_path(project_path, pm.plan_part, pm.session_id);
    std::fs::write(&path, pm.render())?;
    Ok(path)
}

pub fn list_for_part(project_path: &Path, part: u32) -> std::io::Result<Vec<Postmortem>> {
    let dir = postmortems_dir(project_path);
    if !dir.exists() { return Ok(vec![]); }
    let prefix = format!("part-{part}-");
    let mut out = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(&prefix) && name_str.ends_with(".md") {
            if let Ok(s) = std::fs::read_to_string(entry.path()) {
                if let Some(pm) = parse(&s) { out.push(pm); }
            }
        }
    }
    out.sort_by_key(|pm| pm.written_at);
    Ok(out)
}

/// Parse a postmortem from on-disk markdown. Returns `None` on any malformed
/// frontmatter — listing is best-effort and a corrupt file should not poison
/// the whole listing. Format produced by [`Postmortem::render`].
pub fn parse(s: &str) -> Option<Postmortem> {
    let s = s.strip_prefix("---\n")?;
    let (frontmatter, body) = s.split_once("\n---\n")?;
    #[derive(Deserialize)]
    struct Front {
        plan_part: u32,
        session_id: Uuid,
        outcome: Outcome,
        cycles: u32,
        duration_seconds: u64,
        written_at: DateTime<Utc>,
    }
    let front: Front = serde_yaml::from_str(frontmatter).ok()?;
    let what_worked = section(body, "What worked").unwrap_or_default();
    let gotchas = section(body, "Gotchas").unwrap_or_default();
    let follow_ups = section(body, "Follow-ups").unwrap_or_default();
    Some(Postmortem {
        plan_part: front.plan_part,
        session_id: front.session_id,
        outcome: front.outcome,
        cycles: front.cycles,
        duration_seconds: front.duration_seconds,
        written_at: front.written_at,
        what_worked,
        gotchas,
        follow_ups,
    })
}

fn section(body: &str, heading: &str) -> Option<String> {
    let needle = format!("## {heading}\n");
    let idx = body.find(&needle)?;
    let rest = &body[idx + needle.len()..];
    let end = rest.find("\n## ").map(|i| i).unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}
