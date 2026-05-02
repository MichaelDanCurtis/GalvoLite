//! Flat-file decision log under `<project.path>/.galvolite/decisions/`.
//!
//! Every `ask_operator` answer is auto-recorded so future spawns can
//! replay prior decisions in their context-injection prompts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub session_id: Uuid,
    pub asked_at: DateTime<Utc>,
    pub question: String,
    pub answer: String,
    pub choices: Option<Vec<String>>,
}

pub fn decisions_dir(project_path: &Path) -> PathBuf {
    project_path.join(".galvolite").join("decisions")
}

pub fn record(project_path: &Path, d: &Decision) -> std::io::Result<PathBuf> {
    let dir = decisions_dir(project_path);
    std::fs::create_dir_all(&dir)?;
    let ts = d.asked_at.format("%Y%m%dT%H%M%SZ").to_string();
    let path = dir.join(format!("{}-{ts}.md", d.session_id));
    std::fs::write(&path, render(d))?;
    Ok(path)
}

pub fn list_recent(project_path: &Path, limit: usize) -> std::io::Result<Vec<Decision>> {
    let dir = decisions_dir(project_path);
    if !dir.exists() { return Ok(vec![]); }
    let mut out = vec![];
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") { continue; }
        match std::fs::read_to_string(&path) {
            Ok(s) => match parse(&s) {
                Some(d) => out.push(d),
                None => tracing::warn!(path = %path.display(), "decision parse failed"),
            },
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "decision read failed"),
        }
    }
    out.sort_by(|a, b| b.asked_at.cmp(&a.asked_at));
    out.truncate(limit);
    Ok(out)
}

pub fn parse(s: &str) -> Option<Decision> {
    let s = s.strip_prefix("---\n")?;
    let (frontmatter, body) = s.split_once("\n---\n")?;
    #[derive(Deserialize)]
    struct Front {
        session_id: Uuid,
        asked_at: DateTime<Utc>,
        choices: Option<Vec<String>>,
    }
    let front: Front = serde_yaml::from_str(frontmatter).ok()?;
    let question = section(body, "Question").unwrap_or_default();
    let answer = section(body, "Answer").unwrap_or_default();
    Some(Decision {
        session_id: front.session_id,
        asked_at: front.asked_at,
        question, answer,
        choices: front.choices,
    })
}

fn section(body: &str, heading: &str) -> Option<String> {
    let needle = format!("## {heading}\n");
    let idx = body.find(&needle)?;
    let rest = &body[idx + needle.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(rest[..end].trim().to_string())
}

fn render(d: &Decision) -> String {
    let choices_yaml = match &d.choices {
        Some(cs) => serde_yaml::to_string(cs).unwrap_or_else(|_| "[]\n".into()),
        None => "null\n".into(),
    };
    format!(
"---
session_id: {sid}
asked_at: {ts}
choices: {choices}---

## Question

{q}

## Answer

{a}
",
        sid = d.session_id,
        ts = d.asked_at.to_rfc3339(),
        choices = choices_yaml,
        q = d.question,
        a = d.answer,
    )
}
