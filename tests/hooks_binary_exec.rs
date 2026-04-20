use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_exec_against_real_server_writes_inject_text_to_stdout() {
    let mut mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock
        .mock("POST", "/hooks/ingest")
        .with_status(200)
        .with_body(r#"{"action":"proceed","inject_text":"injected!"}"#)
        .create();

    let bin = env!("CARGO_BIN_EXE_galvolite-hook");
    let mut child = Command::new(bin)
        .arg("SessionStart")
        .env("GALVOLITE_API_BASE", &url)
        .env("GALVOLITE_SESSION_ID", "s-1")
        .env("GALVOLITE_TOKEN", "tok")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"{}").unwrap();
    let out = child.wait_with_output().unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "injected!");
}

#[test]
fn binary_exec_fails_soft_when_server_unreachable() {
    let bin = env!("CARGO_BIN_EXE_galvolite-hook");
    let mut child = Command::new(bin)
        .arg("Stop")
        .env("GALVOLITE_API_BASE", "http://127.0.0.1:1") // refused
        .env("GALVOLITE_SESSION_ID", "s-1")
        .env("GALVOLITE_TOKEN", "tok")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"{}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "should exit 0 even on refused connection"
    );
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error"),
        "should log the error to stderr: {stderr}"
    );
}

#[test]
fn binary_exec_with_block_action_writes_no_stdout() {
    let mut mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock
        .mock("POST", "/hooks/ingest")
        .with_status(200)
        .with_body(r#"{"action":"block"}"#)
        .create();

    let bin = env!("CARGO_BIN_EXE_galvolite-hook");
    let mut child = Command::new(bin)
        .arg("PreCompact")
        .env("GALVOLITE_API_BASE", &url)
        .env("GALVOLITE_SESSION_ID", "s-1")
        .env("GALVOLITE_TOKEN", "tok")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"{}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.stdout.is_empty());
}
