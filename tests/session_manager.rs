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

    let session = mgr
        .spawn_session(
            project_id,
            "test task".into(),
            "implementation".into(),
            "sleep".into(),
        )
        .await
        .unwrap();

    mgr.kill_session(session.id).await.unwrap();

    // Give a moment for async cleanup
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let s = mgr.get_session(session.id).unwrap();
    assert_eq!(s.state, SessionState::Killed);
}
