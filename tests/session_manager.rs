use galvolite::db::{Db, SessionState};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;

fn setup() -> (Arc<SessionManager>, uuid::Uuid) {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();

    let proj = db
        .insert_project(galvolite::db::NewProject {
            name: "test".into(),
            path: "/tmp/test-project".into(),
            remote_url: None,
        })
        .unwrap();

    // Leak the tempdir so it outlives the test
    let _ = Box::leak(Box::new(dir));

    let mgr = Arc::new(SessionManager::new(db, data_dir));
    (mgr, proj.id)
}

#[tokio::test]
async fn spawn_session_and_list() {
    let (mgr, project_id) = setup();

    let session = mgr
        .spawn_session(
            project_id,
            "test task".into(),
            "implementation".into(),
            "echo".into(),
            vec![],
        )
        .await
        .unwrap();

    assert_eq!(session.task_summary, "test task");
    assert_eq!(session.role, "implementation");

    // Session appears in list
    let sessions = mgr.list_sessions().unwrap();
    assert!(!sessions.is_empty());
    assert!(sessions.iter().any(|s| s.id == session.id));
}

#[tokio::test]
async fn kill_session_marks_killed() {
    let (mgr, project_id) = setup();

    // Long-running child so the kill path wins the race against a natural
    // process exit. `sleep 60` is a real argument now that spawn_session
    // accepts args.
    let session = mgr
        .spawn_session(
            project_id,
            "test task".into(),
            "implementation".into(),
            "sleep".into(),
            vec!["60".into()],
        )
        .await
        .unwrap();

    mgr.kill_session(session.id).await.unwrap();

    // Give a moment for async cleanup
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let s = mgr.get_session(session.id).unwrap();
    assert_eq!(s.state, SessionState::Killed);
    assert!(s.ended_at.is_some());
    // Running-map entry should be cleared (either by kill_session directly
    // or by the bg task after the child process was terminated).
    assert_eq!(mgr.running_count(), 0);
}
