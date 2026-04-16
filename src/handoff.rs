//! Handoff files live at `<root>/<session-id>/<YYYY-MM-DDTHH-MM-SS>.md`.
//! Root is typically `<project>/.galvolite/handoffs/` but tests inject a temp root.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct HandoffDir(PathBuf);

impl HandoffDir {
    pub fn new(root: PathBuf) -> Self {
        Self(root)
    }

    pub fn root(&self) -> &Path {
        &self.0
    }
}

pub fn handoff_path(dir: &HandoffDir, session_id: Uuid, when: DateTime<Utc>) -> PathBuf {
    let fname = when.format("%Y-%m-%dT%H-%M-%S.md").to_string();
    dir.root().join(session_id.to_string()).join(fname)
}

pub fn ensure_session_dir(dir: &HandoffDir, session_id: Uuid) -> Result<PathBuf> {
    let p = dir.root().join(session_id.to_string());
    std::fs::create_dir_all(&p).map_err(|e| Error::io(&p, e))?;
    Ok(p)
}

pub fn read_handoff(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

pub fn latest_handoff_for(dir: &HandoffDir, session_id: Uuid) -> Result<Option<PathBuf>> {
    let session_dir = dir.root().join(session_id.to_string());
    if !session_dir.exists() {
        return Ok(None);
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&session_dir)
        .map_err(|e| Error::io(&session_dir, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    entries.sort();
    Ok(entries.into_iter().last())
}

pub const REQUIRED_SECTIONS: &[&str] = &[
    "Task Assignment",
    "Current State",
    "Decisions Made",
    "Modified Files",
    "Open Questions",
    "Next Steps",
    "Critical Context",
    "Git State",
    "Continuation Prompt",
];

const PLACEHOLDER_TOKENS: &[&str] = &["TBD", "TODO", "<fill in>", "XXX"];

pub fn validate_handoff_content(body: &str) -> Result<()> {
    // Placeholder scan (runs first so placeholder errors surface before missing-section errors)
    for tok in PLACEHOLDER_TOKENS {
        if body.contains(tok) {
            return Err(Error::HandoffInvalid(format!(
                "contains placeholder token '{tok}'"
            )));
        }
    }

    // Continuation Prompt presence (checked independently so the error always names it)
    if !body.contains("## Continuation Prompt") {
        return Err(Error::HandoffInvalid(
            "missing required section: Continuation Prompt".into(),
        ));
    }

    // Remaining required sections
    for section in REQUIRED_SECTIONS {
        if *section == "Continuation Prompt" {
            continue; // already checked above
        }
        let header = format!("## {section}");
        if !body.contains(&header) {
            return Err(Error::HandoffInvalid(format!(
                "missing required section: {section}"
            )));
        }
    }

    // Continuation Prompt must be non-empty and self-contained
    if let Some(idx) = body.find("## Continuation Prompt") {
        let rest = &body[idx + "## Continuation Prompt".len()..];
        let next_section = rest.find("\n## ").unwrap_or(rest.len());
        let content = rest[..next_section].trim();
        if content.is_empty() {
            return Err(Error::HandoffInvalid(
                "Continuation Prompt is empty".into(),
            ));
        }
        if content.len() < 50 {
            return Err(Error::HandoffInvalid(
                "Continuation Prompt is too short (<50 chars) to be self-contained".into(),
            ));
        }
    }

    Ok(())
}
