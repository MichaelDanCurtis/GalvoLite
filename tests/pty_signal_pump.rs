use galvolite::db::{Db, NewProject, NewSession};
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use galvolite::signals::Signal;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn signals_emitted_on_dedicated_channel_and_persisted() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: tmp.path().to_string_lossy().into(), remote_url: None,
    }).unwrap();
    let sess = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: tmp.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    // Use `printf` to emit a signal line, a status line, then exit.
    let handle = PtyHandle::spawn(PtyOpts {
        cwd: tmp.path().to_path_buf(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf '[STATUS:WORKING]\\n[HANDOFF:READY]\\nregular output\\n'".into()],
        env: vec![],
    }).unwrap();

    let mut pump = SignalPump::start(handle, db.clone(), sess.id);

    // Collect 2 signals.
    let mut got = Vec::new();
    for _ in 0..2 {
        let s = timeout(Duration::from_secs(3), pump.next_signal())
            .await
            .expect("timeout")
            .expect("signal");
        got.push(s);
    }

    assert_eq!(got, vec![Signal::StatusWorking, Signal::HandoffReady]);

    // Persisted in DB
    let rows = db.list_signals_for_session(sess.id).unwrap();
    let kinds: Vec<String> = rows.iter().map(|r| r.kind.clone()).collect();
    assert!(kinds.contains(&"STATUS:WORKING".to_string()));
    assert!(kinds.contains(&"HANDOFF:READY".to_string()));
}
