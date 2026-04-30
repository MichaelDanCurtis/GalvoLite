use galvolite::worktree::{add_worktree, derive_branch_name};
use std::process::Command;
use tempfile::TempDir;
use uuid::Uuid;

fn init_repo(dir: &std::path::Path) {
    Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir).status().unwrap();
    std::fs::write(dir.join("README"), "x").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
    Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"])
        .current_dir(dir).status().unwrap();
}

#[test]
fn derive_branch_name_uses_first_8_uuid_chars() {
    let id = Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap();
    assert_eq!(derive_branch_name(id), "galvolite/01234567");
}

#[test]
fn add_worktree_creates_dir_branch_and_returns_paths() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let id = Uuid::new_v4();
    let (wt, branch) = add_worktree(tmp.path(), id, "HEAD").expect("add_worktree");
    assert!(wt.exists(), "worktree dir created: {wt:?}");
    assert!(wt.starts_with(tmp.path().join(".galvolite/worktrees")));
    assert!(branch.starts_with("galvolite/"));
    let out = Command::new("git")
        .args(["rev-parse", "--verify", &branch])
        .current_dir(tmp.path())
        .output().unwrap();
    assert!(out.status.success(), "branch must resolve: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn add_worktree_retries_on_branch_collision() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
    Command::new("git")
        .args(["branch", "galvolite/aaaaaaaa"])
        .current_dir(tmp.path())
        .status().unwrap();
    let (_wt, branch) = add_worktree(tmp.path(), id, "HEAD").expect("retry must succeed");
    assert_eq!(branch, "galvolite/aaaaaaaa-2");
}
