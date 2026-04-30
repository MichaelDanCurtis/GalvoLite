//! `git worktree` plumbing for Sub-plan 6. Each child session gets its own
//! worktree under `<repo>/.galvolite/worktrees/<session-id>/` on a branch
//! derived from the session UUID. Collisions retry with `-2`, `-3` suffixes.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

/// `galvolite/<first-8-of-uuid>`.
pub fn derive_branch_name(session_id: Uuid) -> String {
    let s = session_id.simple().to_string();
    format!("galvolite/{}", &s[..8])
}

/// `git worktree add <repo>/.galvolite/worktrees/<id>/ -b <branch> <base_ref>`.
pub fn add_worktree(
    repo_root: &Path,
    session_id: Uuid,
    base_ref: &str,
) -> Result<(PathBuf, String)> {
    let wt_dir = repo_root.join(".galvolite").join("worktrees").join(session_id.to_string());
    if let Some(parent) = wt_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let base_branch = derive_branch_name(session_id);
    let mut last_stderr = String::new();
    for attempt in 1..=3 {
        let branch = if attempt == 1 {
            base_branch.clone()
        } else {
            format!("{base_branch}-{attempt}")
        };
        let out = Command::new("git")
            .args([
                "-C", repo_root.to_str().ok_or_else(|| Error::Config("repo path not utf-8".into()))?,
                "worktree", "add",
                wt_dir.to_str().ok_or_else(|| Error::Config("worktree path not utf-8".into()))?,
                "-b", &branch, base_ref,
            ])
            .output()
            .map_err(|e| Error::io(repo_root, e))?;
        if out.status.success() {
            return Ok((wt_dir, branch));
        }
        last_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !last_stderr.contains("already exists") {
            return Err(Error::Config(format!("git worktree add failed: {last_stderr}")));
        }
    }
    Err(Error::Config(format!(
        "git worktree add: branch collision unresolved after 3 attempts: {last_stderr}"
    )))
}
