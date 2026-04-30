mod common;

use galvolite::api::events::DomainEvent;
use galvolite::db::NewProject;
use std::time::Duration;
use tempfile::TempDir;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orphan_session_publishes_session_orphaned_event() {
    let app = common::test_app_with_bus();
    let mut rx = app.mgr.subscribe_events();

    let proj_dir = TempDir::new().unwrap();
    let proj = app
        .mgr
        .add_project(NewProject {
            name: "p".into(),
            path: proj_dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .expect("add_project should succeed for a fresh tempdir");

    // Force pty_spawn_failed by pointing at a non-existent binary.
    let res = app
        .mgr
        .spawn_session(
            proj.id,
            "t".into(),
            "implementation".into(),
            "/no-such-bin-galvolite-test".into(),
            vec![],
        )
        .await;
    assert!(res.is_err(), "spawn_session should fail when claude bin missing");

    // Drain the bus for up to 1s looking for system.session_orphaned.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let mut saw_orphan = false;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Ok(DomainEvent::SessionEventAppended { kind, .. }))
                if kind == "system.session_orphaned" =>
            {
                saw_orphan = true;
                break;
            }
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => break, // sender dropped
            Err(_) => continue,  // 50ms tick, keep going
        }
    }
    assert!(
        saw_orphan,
        "did not observe DomainEvent::SessionEventAppended for system.session_orphaned"
    );
}
