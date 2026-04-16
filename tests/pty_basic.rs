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
