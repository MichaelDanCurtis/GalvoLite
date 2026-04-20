//! End-to-end smoke test for the hook surface (Sub-plan 2 Task 10).
//!
//! Spins up the real axum router on a real TCP port, then exec's the
//! compiled `galvolite-hook` binary (via `CARGO_BIN_EXE_galvolite-hook`)
//! against it for each of the 7 Claude Code hooks in a realistic
//! lifecycle sequence. Verifies state transitions, event-log shape, and
//! `inject_text` responses end-to-end without needing a real claude PTY
//! (Sub-plan 5's scope).

use galvolite::db::{Db, NewProject, NewSession};
use galvolite::session_manager::SessionManager;
use galvolite::state::SessionState;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tempfile::TempDir;

struct SmokeApp {
    db: Db,
    url: String,
    _tmp: TempDir,
    _server: tokio::task::JoinHandle<()>,
}

async fn smoke_app() -> SmokeApp {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let router = galvolite::api::router(mgr);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");

    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    SmokeApp {
        db,
        url,
        _tmp: tmp,
        _server: server,
    }
}

fn seed_session(db: &Db, role: &str) -> (uuid::Uuid, String) {
    let path = format!("/tmp/galvolite-smoke-{}", uuid::Uuid::new_v4());
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: path.clone(),
            remote_url: None,
        })
        .unwrap();
    let s = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: role.into(),
            branch: None,
            worktree_path: path,
            task_summary: "smoke".into(),
            parent_session_id: None,
        })
        .unwrap();
    let token = format!("tok-{}", uuid::Uuid::new_v4());
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET api_token = ?1 WHERE id = ?2",
        rusqlite::params![token, s.id.to_string()],
    )
    .unwrap();
    (s.id, token)
}

fn state_of(db: &Db, id: uuid::Uuid) -> SessionState {
    let conn = db.pool().get().unwrap();
    let s: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    s.parse().unwrap()
}

fn run_hook(
    bin: &str,
    url: &str,
    session: &uuid::Uuid,
    token: &str,
    hook: &str,
    payload: &str,
) -> std::process::Output {
    let mut child = Command::new(bin)
        .arg(hook)
        .env("GALVOLITE_API_BASE", url)
        .env("GALVOLITE_SESSION_ID", session.to_string())
        .env("GALVOLITE_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

// Restore cwd at drop — tests run in parallel so we serialize via the
// CWD_LOCK below.
struct ChDir {
    prev: std::path::PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl ChDir {
    fn new(p: &std::path::Path) -> Self {
        let _guard = CWD_LOCK.lock().unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(p).unwrap();
        Self { prev, _guard }
    }
}
impl Drop for ChDir {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.prev);
    }
}
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_hook_lifecycle_via_real_binary() {
    let app = smoke_app().await;
    let (session_id, token) = seed_session(&app.db, "implementation");

    // SessionEnd's auto-handoff writer uses cwd. Per-test cwd to avoid
    // stomping on parallel tests.
    let cwd = TempDir::new().unwrap();
    let _guard = ChDir::new(cwd.path());

    let bin = env!("CARGO_BIN_EXE_galvolite-hook");

    // 1. SessionStart — should return seed text and land Idle.
    let out = run_hook(bin, &app.url, &session_id, &token, "SessionStart", "{}");
    assert!(
        out.status.success(),
        "SessionStart failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.stdout.is_empty(),
        "SessionStart should produce inject_text"
    );
    assert_eq!(state_of(&app.db, session_id), SessionState::Idle);

    // 2. UserPromptSubmit — Idle → Working, no inject (under 80%).
    let out = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "UserPromptSubmit",
        r#"{"prompt_preview":"hi","est_tokens":100}"#,
    );
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "UserPromptSubmit at 100 tokens shouldn't inject; got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(state_of(&app.db, session_id), SessionState::Working);

    // 3. PostToolUse(Bash) git commit — no state change.
    let _ = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "PostToolUse",
        r#"{"tool":"Bash","command":"git commit -m x","exit_code":0}"#,
    );
    assert_eq!(state_of(&app.db, session_id), SessionState::Working);

    // 4. Notification — log only.
    let _ = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "Notification",
        r#"{"level":"info","message":"compile ok"}"#,
    );

    // 5. Stop ×3 — first 2 inject, 3rd transitions Working → Idle.
    let r1 = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "Stop",
        r#"{"turn_id":"t1"}"#,
    );
    assert!(!r1.stdout.is_empty(), "1st Stop should inject");
    let r2 = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "Stop",
        r#"{"turn_id":"t2"}"#,
    );
    assert!(!r2.stdout.is_empty(), "2nd Stop should inject");
    let r3 = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "Stop",
        r#"{"turn_id":"t3"}"#,
    );
    assert!(
        r3.stdout.is_empty(),
        "3rd Stop should NOT inject (system.stop_max_retries fires); got: {}",
        String::from_utf8_lossy(&r3.stdout)
    );
    assert_eq!(state_of(&app.db, session_id), SessionState::Idle);

    // 6. PreCompact — should inject (no write_handoff yet).
    let r = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "PreCompact",
        r#"{"trigger_reason":"context_full"}"#,
    );
    let body = String::from_utf8_lossy(&r.stdout).to_string();
    assert!(
        body.contains("write_handoff"),
        "PreCompact should mention write_handoff; got: {body}"
    );

    // 7. SessionEnd clean → Ended, auto-handoff written.
    let _ = run_hook(
        bin,
        &app.url,
        &session_id,
        &token,
        "SessionEnd",
        r#"{"exit_code":0,"reason":"ok"}"#,
    );
    assert_eq!(state_of(&app.db, session_id), SessionState::Ended);
    let pointer = cwd
        .path()
        .join(format!(".galvolite/handoffs/{session_id}/pointer.json"));
    assert!(
        pointer.exists(),
        "auto-handoff pointer missing at {}",
        pointer.display()
    );

    // Event log shape: ≥7 hook.* events for the full lifecycle.
    let conn = app.db.pool().get().unwrap();
    let hook_events: i64 = conn
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind LIKE 'hook.%'",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        hook_events >= 7,
        "expected ≥7 hook.* events, got {hook_events}"
    );
}
