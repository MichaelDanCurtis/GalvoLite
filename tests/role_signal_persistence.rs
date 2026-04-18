use galvolite::db::{Db, NewProject, NewSession};
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[tokio::test]
async fn role_signal_writes_to_session_row() {
    let tmp = tempdir().unwrap();
    let db = Db::open(tmp.path().join("test.sqlite")).unwrap();

    let project = db
        .insert_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    let session = db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: tmp.path().to_string_lossy().into_owned(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();

    let pty = PtyHandle::spawn(PtyOpts {
        cwd: tmp.path().to_path_buf(),
        command: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "printf '[ROLE:SET role=\"brainstorm\"]\\n'".into(),
        ],
        env: vec![],
    })
    .unwrap();

    let cycle: Arc<Mutex<Option<uuid::Uuid>>> = Arc::new(Mutex::new(None));
    let mut pump = SignalPump::start(pty, db.clone(), session.id, cycle);

    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        pump.next_signal(),
    )
    .await;

    // Give the DB write a moment to land.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let updated = db.get_session(session.id).unwrap();
    assert_eq!(updated.role, "brainstorm");
}
