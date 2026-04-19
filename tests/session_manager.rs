use galvolite::db::{Db, SessionState};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;

async fn setup_mgr() -> (Arc<SessionManager>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.db")).unwrap();
    let mgr = Arc::new(SessionManager::new(db, dir.path().to_path_buf()));
    (mgr, dir)
}

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

#[tokio::test]
async fn delete_project_purges_rows_and_kills_running_sessions() {
    let (mgr, _dir) = setup_mgr().await;

    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    let session = mgr
        .spawn_session(
            project.id,
            "task".into(),
            "implementation".into(),
            "echo".into(),
            vec!["hello".into()],
        )
        .await
        .unwrap();

    assert_eq!(mgr.list_sessions().unwrap().len(), 1);

    mgr.delete_project(project.id).await.unwrap();

    assert!(mgr.get_project(project.id).is_err(), "project removed");
    assert_eq!(mgr.list_sessions().unwrap().len(), 0, "sessions purged");
    assert!(!mgr.running_ids().contains(&session.id));
}

#[tokio::test]
async fn delete_project_unknown_returns_not_found() {
    let (mgr, _dir) = setup_mgr().await;
    let res = mgr.delete_project(uuid::Uuid::new_v4()).await;
    assert!(matches!(
        res,
        Err(galvolite::error::Error::ProjectNotFound(_))
    ));
}

#[tokio::test]
async fn set_autopilot_toggles_session_flag() {
    let (mgr, _dir) = setup_mgr().await;
    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(project.id, "t".into(), "implementation".into(), "echo".into(), vec![])
        .await
        .unwrap();

    assert!(!session.autopilot);
    mgr.set_autopilot(session.id, true).await.unwrap();
    assert!(mgr.get_session(session.id).unwrap().autopilot);
    mgr.set_autopilot(session.id, false).await.unwrap();
    assert!(!mgr.get_session(session.id).unwrap().autopilot);
}

#[tokio::test]
async fn set_autopilot_unknown_session_returns_not_found() {
    let (mgr, _dir) = setup_mgr().await;
    let res = mgr.set_autopilot(uuid::Uuid::new_v4(), true).await;
    assert!(matches!(
        res,
        Err(galvolite::error::Error::SessionNotFound(_))
    ));
}
