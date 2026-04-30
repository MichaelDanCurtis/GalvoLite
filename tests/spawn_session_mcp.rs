mod common;
use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;

fn init_git(dir: &std::path::Path) {
    Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir).status().unwrap();
    std::fs::write(dir.join("README"), "x").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
    Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"])
        .current_dir(dir).status().unwrap();
}

#[tokio::test]
async fn spawn_child_creates_worktree_and_db_row_with_parent() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());

    let proj = app.mgr.add_project(NewProject {
        name: "p".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(),
        "/bin/cat".into(), vec![],
    ).await.expect("parent spawn");

    let child = app.mgr.spawn_child(parent.id, "implementation", "child task")
        .await.expect("spawn_child");

    assert_eq!(child.parent_session_id, Some(parent.id));
    assert!(std::path::Path::new(&child.worktree_path).exists());
    assert!(child.branch.as_deref().unwrap().starts_with("galvolite/"));
    assert_ne!(child.worktree_path, parent.worktree_path);
}
