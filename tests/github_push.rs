//! Plan 10b Task 3 — push helper tests.
//!
//! The token-redaction test deliberately triggers a push failure to
//! a bogus remote and asserts the token literal never appears in the
//! error string.

use galvolite::github::{push::push_branch, PushError};
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"))
}

/// Set up a temp working repo with one commit on a `galvolite/test` branch.
/// Returns (worktree_dir, branch_name).
fn setup_repo() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "main"]);
    run_git(dir.path(), &["config", "user.email", "test@example"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    std::fs::write(dir.path().join("README.md"), "hello").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-q", "-m", "init"]);
    run_git(dir.path(), &["checkout", "-q", "-b", "galvolite/test"]);
    std::fs::write(dir.path().join("file.txt"), "content").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-q", "-m", "add file"]);
    (dir, "galvolite/test".into())
}

#[test]
fn push_fails_with_redacted_token_when_remote_unreachable() {
    let (worktree, branch) = setup_repo();
    // Use a token literal that's distinctive enough we'll spot it if
    // it leaks. The remote (github.com) won't accept the bogus owner.
    let token = "ghp_DEFINITELY_A_SECRET_TOKEN_VALUE_xyz123";
    let err = push_branch(
        worktree.path(),
        &branch,
        "octocat-bogus-nonexistent-owner-12345",
        "hello",
        token,
    )
    .unwrap_err();
    let msg = format!("{err}");
    // Must NOT contain the token literal anywhere in the error.
    assert!(
        !msg.contains(token),
        "token leaked into error message: {msg}"
    );
    // Sanity check: the error string must be non-trivial — i.e. git
    // actually produced stderr that we then redacted. Without this,
    // the no-leak assertion above could pass vacuously (empty stderr).
    assert!(
        msg.len() > "git push failed: ".len(),
        "error message suspiciously short — possible vacuous pass: {msg}"
    );
}

#[test]
fn push_fails_with_clear_error_when_worktree_path_invalid() {
    let bogus = std::path::PathBuf::from("/this/path/should/not/exist/anywhere");
    let err = push_branch(&bogus, "any-branch", "owner", "name", "any-token").unwrap_err();
    assert!(matches!(err, PushError::InvalidWorktree(_)));
}
