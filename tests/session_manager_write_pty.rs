use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;

async fn make_manager() -> (tempfile::TempDir, Arc<SessionManager>) {
    let dir = tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.db")).unwrap();
    let mgr = Arc::new(SessionManager::new(db, dir.path().to_path_buf()));
    (dir, mgr)
}

#[tokio::test]
async fn write_pty_sends_input_to_running_session() {
    let (_tmp_data, mgr) = make_manager().await;

    // Use a real tmp dir so add_project's canonicalize check passes.
    let proj_dir = tempdir().unwrap();
    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: proj_dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn `cat` — it echoes stdin back to stdout.
    let session = mgr
        .spawn_session(
            project.id,
            "test write_pty".into(),
            "implementation".into(),
            "cat".into(),
            vec![],
        )
        .await
        .unwrap();

    let mut rx = mgr.subscribe_output(session.id).unwrap();

    mgr.write_pty(session.id, b"hello\n".to_vec()).await.unwrap();

    let found = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let bytes = rx.recv().await.unwrap();
            if bytes.windows(5).any(|w| w == b"hello") {
                return true;
            }
        }
    })
    .await
    .unwrap_or(false);

    assert!(found, "expected 'hello' to appear in pty output within 2s");
}

#[tokio::test]
async fn write_pty_returns_session_not_running() {
    let (_tmp_data, mgr) = make_manager().await;
    let res = mgr.write_pty(uuid::Uuid::new_v4(), b"x".to_vec()).await;
    assert!(
        matches!(res, Err(galvolite::error::Error::SessionNotRunning(_))),
        "expected SessionNotRunning, got {res:?}"
    );
}
