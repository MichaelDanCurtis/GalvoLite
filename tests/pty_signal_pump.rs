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

    let mut pump = SignalPump::start(
        handle,
        db.clone(),
        sess.id,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );

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

#[tokio::test]
async fn signals_recorded_with_cycle_id() {
    use galvolite::db::{Db, NewProject, NewSession};
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("test.sqlite")).unwrap();

    let proj = db.insert_project(NewProject {
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
    }).unwrap();

    let sess = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: "/tmp/test".into(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let cycle_id = db.start_cycle(sess.id).unwrap();

    let pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "echo".into(),
        args: vec!["[STATUS:WORKING]".into()],
        env: vec![],
    }).unwrap();

    let mut pump = SignalPump::start(
        pty,
        db.clone(),
        sess.id,
        std::sync::Arc::new(std::sync::Mutex::new(Some(cycle_id))),
    );

    // Wait for signal
    let sig = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        pump.next_signal(),
    ).await.unwrap();
    assert!(sig.is_some());

    // Verify signal was recorded with cycle_id
    let signals = db.list_signals_for_session(sess.id).unwrap();
    assert!(!signals.is_empty());
    assert_eq!(signals[0].cycle_id, Some(cycle_id));
}

#[tokio::test]
async fn multiple_subscribers_receive_output() {
    use galvolite::db::{Db, NewProject, NewSession};
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("test.sqlite")).unwrap();

    let proj = db.insert_project(NewProject {
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
    }).unwrap();

    let sess = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: "/tmp/test".into(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "echo".into(),
        args: vec!["hello world".into()],
        env: vec![],
    }).unwrap();

    let pump = SignalPump::start(
        pty,
        db,
        sess.id,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );

    let mut rx1 = pump.subscribe_output();
    let mut rx2 = pump.subscribe_output();

    let line1 = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rx1.recv(),
    ).await.unwrap().unwrap();

    let line2 = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rx2.recv(),
    ).await.unwrap().unwrap();

    assert!(line1.contains("hello world"));
    assert_eq!(line1, line2);
}
