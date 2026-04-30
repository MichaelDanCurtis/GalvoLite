use galvolite::pty::{PtyHandle, PtyOpts};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn spawn_echo_appears_in_scrollback() {
    let opts = PtyOpts {
        cwd: std::env::temp_dir(),
        command: "/bin/echo".to_string(),
        args: vec!["hello from pty".into()],
        env: vec![],
    };
    let handle = PtyHandle::spawn(opts).expect("spawn");

    // Drain raw bytes from the broadcast subscriber until we see the
    // expected text or time out.
    let (snap, mut rx) = handle.snapshot_and_subscribe();
    let mut collected: Vec<u8> = snap;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(bytes)) => collected.extend_from_slice(&bytes),
            _ => break,
        }
    }
    let text = String::from_utf8_lossy(&collected);
    assert!(text.contains("hello from pty"), "stdout: {text:?}");
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
    let (_snap, mut rx) = handle.snapshot_and_subscribe();
    handle.write(b"ping\n").await.expect("write");

    let mut got = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(bytes)) => {
                got.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&got).contains("ping") {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(
        String::from_utf8_lossy(&got).contains("ping"),
        "got: {:?}",
        String::from_utf8_lossy(&got)
    );

    handle.kill().await.expect("kill");
}

#[tokio::test]
async fn shared_handle_kill_unblocks_wait_exit() {
    // Mirrors the production flow: bg task owns PtyHandle and parks on
    // wait_exit; a shared handle kill flips the kill flag so wait_exit
    // returns and PtyHandle::drop fires child.kill().
    let dir = tempfile::tempdir().unwrap();

    let mut pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "sleep".into(),
        args: vec!["300".into()],
        env: vec![],
    })
    .unwrap();

    let pid = pty.pid;
    let shared = pty.shared();

    let alive = unsafe { libc::kill(pid as i32, 0) };
    assert_eq!(alive, 0, "child should be alive before kill");

    let waiter = tokio::spawn(async move {
        pty.wait_exit().await;
        // Drop fires here; child gets killed.
    });

    shared.kill();

    // wait_exit must return promptly once the kill flag is raised.
    tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .expect("wait_exit did not return after kill")
        .unwrap();

    // Give the kernel a moment to reap the child after Drop.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let dead = unsafe { libc::kill(pid as i32, 0) };
    assert_ne!(dead, 0, "child should be dead after Drop fires kill()");
}
