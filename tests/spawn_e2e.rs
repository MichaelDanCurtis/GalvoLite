//! End-to-end: a parent (mock-claude) calls spawn_session twice + wait_children;
//! children mock-claude exits cleanly; harness writes pending_rejoin; parent's PTY
//! sees the marker; the hook binary fetches and returns additionalContext.
mod common;

use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use galvolite::db::NewProject;

fn init_git(d: &std::path::Path) {
    Command::new("git").args(["init","-q","-b","main"]).current_dir(d).status().unwrap();
    std::fs::write(d.join("README"),"x").unwrap();
    Command::new("git").args(["add","."]).current_dir(d).status().unwrap();
    Command::new("git").args(["-c","user.email=t@t","-c","user.name=t","commit","-qm","i"])
        .current_dir(d).status().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spawn_e2e_two_children_parent_sees_rejoin() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();

    // Parent uses /bin/cat as a PTY-friendly stand-in (long-running, accepts stdin).
    // Bug fix #1: use "implementation" role — "orchestrator" is not in the DB CHECK constraint.
    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();

    // Spawn two children manually (E2E: spawn via SessionManager directly).
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "task1", None).await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "task2", None).await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    // Drive both to terminal via the same path mcp tools take.
    // Apply events sequentially with a short yield between them so the bus
    // reactor's Immediate transaction for c1 does not race with the apply for
    // c2 (SQLite only allows one writer at a time).
    for cid in [c1.id, c2.id] {
        galvolite::state::apply_event_with_bus(
            &app.db, cid, app.mgr.event_bus(),
            galvolite::state::EventInput {
                source: galvolite::state::Source::Mcp,
                kind: "mcp.status_report".into(),
                payload: serde_json::json!({"kind":"complete"}),
                dedup_key: None,
            }
        ).unwrap();
        // Yield to let the reactor's spawned task acquire + release its
        // Immediate transaction before we apply the next event.
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    // Allow the bus reactor to fire on_child_terminal for the second child.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Parent now has pending_rejoin set. Scope the conn so it is dropped before
    // the GET request (rusqlite Connection is not Send; must not be held across
    // the async oneshot call below).
    {
        let conn = app.db.pool().get().unwrap();
        let pending: Option<String> = conn.query_row(
            "SELECT pending_rejoin FROM sessions WHERE id = ?1",
            [parent.id.to_string()], |r| r.get(0)
        ).unwrap();
        assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
        assert!(pending.as_deref().unwrap().contains(&c1.id.simple().to_string()[..8]));
        assert!(pending.as_deref().unwrap().contains(&c2.id.simple().to_string()[..8]));
    }

    // Bug fix #2: use raw SQL to fetch the token — Db has no read_api_token method.
    let token: String = app.db.pool().get().unwrap().query_row(
        "SELECT api_token FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get(0),
    ).unwrap();

    // GET /pending-rejoin clears it.
    let req = axum::http::Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", parent.id))
        .header("Authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty()).unwrap();
    use tower::ServiceExt;
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // Worktrees of complete children are GC'd.
    assert!(!std::path::Path::new(&c1.worktree_path).exists());
    assert!(!std::path::Path::new(&c2.worktree_path).exists());
}
