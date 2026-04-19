use galvolite::db::{Db, NewProject, NewSession};
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use std::sync::{Arc, Mutex as StdMutex};
use tempfile::TempDir;

/// Spawn a PTY that prints `[STATUS:WORKING]` and `[STATUS:COMPLETE]`,
/// run them through SignalPump, then assert both events land in the new
/// events table with the correct kind prefix.
#[tokio::test]
async fn signal_pump_emits_events_through_apply_event() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: tmp.path().to_string_lossy().into_owned(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();

    // Spawn /bin/sh that prints two signal lines and exits.
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: tmp.path().to_path_buf(),
        command: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "echo '[STATUS:WORKING]'; sleep 0.05; echo '[STATUS:COMPLETE]'".into(),
        ],
        env: vec![],
    })
    .unwrap();

    let cycle_slot: galvolite::pty::CycleSlot = Arc::new(StdMutex::new(None));
    let mut pump = SignalPump::start(pty, db.clone(), session.id, cycle_slot);

    // Drain signals so the pump background task processes both lines.
    while let Some(_sig) =
        tokio::time::timeout(std::time::Duration::from_millis(500), pump.next_signal())
            .await
            .ok()
            .flatten()
    {}

    let conn = db.pool().get().unwrap();
    let mut stmt = conn
        .prepare("SELECT kind FROM events WHERE session_id = ?1 ORDER BY id ASC")
        .unwrap();
    let kinds: Vec<String> = stmt
        .query_map([session.id.to_string()], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(
        kinds.iter().any(|k| k == "signal.STATUS:WORKING"),
        "expected signal.STATUS:WORKING in events; got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "signal.STATUS:COMPLETE"),
        "expected signal.STATUS:COMPLETE in events; got {kinds:?}"
    );
}
