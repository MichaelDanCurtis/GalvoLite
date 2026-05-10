//! Plan 10b — git push helper for PR creation.
//!
//! Uses HTTPS push with the PAT in the URL (`x-access-token` pattern)
//! per GitHub's officially supported PAT-push idiom. The token is
//! redacted from any error stderr before propagating, so it never
//! reaches the dashboard or logs.

use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    /// `git push` exited non-zero. The string here is the captured
    /// stderr with the PAT redacted.
    #[error("git push failed: {0}")]
    GitFailed(String),

    /// The worktree path doesn't exist or isn't readable.
    #[error("worktree path invalid: {0}")]
    InvalidWorktree(String),

    /// We failed to launch `git` (binary missing, etc.).
    #[error("git invocation failed: {0}")]
    Spawn(String),
}

/// Push `branch` from `worktree_path` to `https://github.com/<owner>/<name>`
/// using `token` for HTTPS auth. Returns Ok on success; Err with a
/// token-redacted message on any failure.
///
/// Refspec is `<branch>:<branch>` (push local branch to same name on
/// remote). The remote is constructed inline rather than relying on
/// the worktree's `origin` to avoid surprises if the operator has
/// configured a non-GitHub origin.
pub fn push_branch(
    worktree_path: &Path,
    branch: &str,
    owner: &str,
    name: &str,
    token: &str,
) -> Result<(), PushError> {
    if !worktree_path.exists() {
        return Err(PushError::InvalidWorktree(
            worktree_path.display().to_string(),
        ));
    }
    let url = format!("https://x-access-token:{token}@github.com/{owner}/{name}.git");
    let refspec = format!("{branch}:{branch}");
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_path)
        .arg("push")
        .arg(&url)
        .arg(&refspec)
        .output()
        .map_err(|e| PushError::Spawn(format!("{e}")))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let redacted = stderr.replace(token, "***");
    Err(PushError::GitFailed(redacted))
}
