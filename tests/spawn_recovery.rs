mod common;
use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;

fn init_git(d: &std::path::Path) {
    Command::new("git").args(["init","-q","-b","main"]).current_dir(d).status().unwrap();
    std::fs::write(d.join("README"),"x").unwrap();
    Command::new("git").args(["add","."]).current_dir(d).status().unwrap();
    Command::new("git")
        .args(["-c","user.email=t@t","-c","user.name=t","commit","-qm","i"])
        .current_dir(d).status().unwrap();
}

#[tokio::test]
async fn recovery_resolves_already_terminal_children() {
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let app1 = common::test_app_with_bus_at(dir.path());
    let proj = app1.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app1.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app1.mgr.spawn_child(parent.id, "implementation", "x", None).await.unwrap();
    app1.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();
    // Stamp terminal_status directly — simulate the reactor missing the event.
    app1.db.pool().get().unwrap().execute(
        "UPDATE sessions SET terminal_status = 'complete' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    drop(app1);

    // Re-open at the same DB path: recovery pass must turn waiting_on into pending_rejoin.
    let app2 = common::reopen_app_with_bus_at(dir.path());
    app2.mgr.recover_pending_rejoins().await.unwrap();

    let conn = app2.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
}
