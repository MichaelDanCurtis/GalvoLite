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

    let (_snap, mut rx) = mgr.subscribe_output(session.id).unwrap();

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
async fn subscribe_output_replays_scrollback_to_late_subscribers() {
    let (_tmp_data, mgr) = make_manager().await;
    let proj_dir = tempdir().unwrap();
    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: proj_dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    let session = mgr
        .spawn_session(
            project.id,
            "test scrollback replay".into(),
            "implementation".into(),
            "cat".into(),
            vec![],
        )
        .await
        .unwrap();

    // First client: subscribe BEFORE input is written, drain a known marker.
    let (_snap0, mut rx0) = mgr.subscribe_output(session.id).unwrap();
    mgr.write_pty(session.id, b"REPLAYME\n".to_vec()).await.unwrap();

    let saw_live = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let bytes = rx0.recv().await.unwrap();
            if bytes.windows(8).any(|w| w == b"REPLAYME") {
                return true;
            }
        }
    })
    .await
    .unwrap_or(false);
    assert!(saw_live, "first subscriber must see live bytes");

    // Second client subscribes AFTER the bytes were broadcast. With the
    // scrollback ring buffer they should appear in the snapshot.
    let (snap1, _rx1) = mgr.subscribe_output(session.id).unwrap();
    assert!(
        snap1.windows(8).any(|w| w == b"REPLAYME"),
        "late subscriber's snapshot must contain replayed bytes; got {} bytes",
        snap1.len()
    );
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
