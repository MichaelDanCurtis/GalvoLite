use galvolite::pty::{PtyHandle, PtyOpts};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn spawn_echo_and_read_stdout() {
    let opts = PtyOpts {
        cwd: std::env::temp_dir(),
        command: "/bin/echo".to_string(),
        args: vec!["hello from pty".into()],
        env: vec![],
    };
    let mut handle = PtyHandle::spawn(opts).expect("spawn");

    // Collect all lines until the child exits or we time out.
    let mut collected = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while let Ok(Some(line)) = timeout(Duration::from_millis(500), handle.next_line()).await {
        collected.push_str(&line);
        collected.push('\n');
        if tokio::time::Instant::now() >= deadline {
            break;
        }
    }

    assert!(collected.contains("hello from pty"), "stdout: {collected:?}");
}

#[tokio::test]
async fn write_to_pty_reaches_child() {
    // cat echoes stdin to stdout.
    let opts = PtyOpts {
        cwd: std::env::temp_dir(),
        command: "/bin/cat".to_string(),
        args: vec![],
        env: vec![],
    };
    let mut handle = PtyHandle::spawn(opts).expect("spawn");
    handle.write(b"ping\n").await.expect("write");

    let line = timeout(Duration::from_secs(2), handle.next_line())
        .await
        .expect("timeout")
        .expect("line");
    assert!(line.contains("ping"), "got: {line:?}");

    handle.kill().await.expect("kill");
}

#[tokio::test]
async fn shared_handle_kill_terminates_child() {
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
    use galvolite::db::Db;

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("test.sqlite")).unwrap();

    // Spawn a process that runs forever
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "sleep".into(),
        args: vec!["300".into()],
        env: vec![],
    }).unwrap();

    let pid = pty.pid;
    let pump = SignalPump::start(
        pty,
        db,
        uuid::Uuid::new_v4(),
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    let handle = pump.handle();

    // Verify child is running
    let alive = unsafe { libc::kill(pid as i32, 0) };
    assert_eq!(alive, 0, "child should be alive before kill");

    // Kill via shared handle
    handle.kill();

    // Give the OS a moment to deliver the signal
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify child is dead
    let dead = unsafe { libc::kill(pid as i32, 0) };
    assert_ne!(dead, 0, "child should be dead after kill");
}
