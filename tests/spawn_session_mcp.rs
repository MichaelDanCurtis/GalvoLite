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

#[tokio::test]
async fn set_waiting_on_persists_json_array() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();

    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let waiting: String = conn.query_row(
        "SELECT waiting_on FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get(0)
    ).unwrap();
    let parsed: Vec<String> = serde_json::from_str(&waiting).unwrap();
    assert_eq!(parsed.len(), 2);
    assert!(parsed.contains(&c1.id.to_string()));
    assert!(parsed.contains(&c2.id.to_string()));
}

#[tokio::test]
async fn build_rejoin_markdown_lists_each_child() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    let conn = app.db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET terminal_status = 'complete', handoff_path = '/tmp/h.md' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    drop(conn);

    let md = app.mgr.build_rejoin_markdown(parent.id, &[c.id]).await.unwrap();
    assert!(md.starts_with("Children resolved. Resume."));
    assert!(md.contains(&c.id.simple().to_string()[..8]));
    assert!(md.contains("status=complete"));
    assert!(md.contains("handoff=/tmp/h.md"));
}

#[tokio::test]
async fn on_child_terminal_clears_one_of_two_children() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    app.mgr.on_child_terminal(c1.id, "complete").await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let (waiting, pending): (Option<String>, Option<String>) = conn.query_row(
        "SELECT waiting_on, pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| Ok((r.get(0)?, r.get(1)?))
    ).unwrap();
    let arr: Vec<String> = serde_json::from_str(waiting.as_deref().unwrap()).unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr.contains(&c2.id.to_string()));
    assert!(pending.is_none(), "no rejoin yet — c2 still pending");
}

#[tokio::test]
async fn on_child_terminal_writes_pending_rejoin_when_last() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();

    app.mgr.on_child_terminal(c.id, "complete").await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let (waiting, pending, term_status): (Option<String>, Option<String>, Option<String>) =
        conn.query_row(
            "SELECT s.waiting_on, s.pending_rejoin, c.terminal_status \
             FROM sessions s JOIN sessions c ON c.id = ?2 WHERE s.id = ?1",
            [parent.id.to_string(), c.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
    assert!(waiting.is_none(), "waiting_on cleared");
    assert!(pending.as_deref().unwrap().contains("Children resolved"));
    assert_eq!(term_status.as_deref(), Some("complete"));
}
