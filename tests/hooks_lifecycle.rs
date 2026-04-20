mod common;

use common::{post_hook, seed_session_in_state, seed_session_with_role, state_of, test_app};
use galvolite::hooks::types::{HookAction, HookKind};
use galvolite::state::SessionState;
use serde_json::json;

#[tokio::test]
async fn session_start_returns_seed_text_with_role_profile() {
    let app = test_app();
    let (id, token) = seed_session_with_role(&app.db, "implementation");

    let resp = post_hook(
        &app,
        &token,
        &id,
        HookKind::SessionStart,
        json!({"is_resume": false}),
    )
    .await;
    assert_eq!(resp.action, HookAction::Proceed);
    let inject = resp.inject_text.expect("seed text required");
    assert!(
        inject.to_lowercase().contains("implementation"),
        "should mention role: {inject}"
    );
}

#[tokio::test]
async fn session_end_clean_exit_transitions_to_ended() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::SessionEnd,
        json!({"exit_code": 0, "reason": "user_quit"}),
    )
    .await;

    assert_eq!(state_of(&app.db, id), SessionState::Ended);
}

#[tokio::test]
async fn session_end_unclean_transitions_to_error() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::SessionEnd,
        json!({"exit_code": 137, "reason": "killed"}),
    )
    .await;

    assert_eq!(state_of(&app.db, id), SessionState::Error);
}

#[tokio::test]
async fn session_end_auto_writes_handoff_pointer_if_no_recent_write_handoff() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    // Run from a per-test cwd so concurrent tests don't stomp on .galvolite.
    let cwd = tempfile::tempdir().unwrap();
    let _guard = ChDir::new(cwd.path());

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::SessionEnd,
        json!({"exit_code": 0, "reason": "ok"}),
    )
    .await;

    let pointer = cwd.path().join(format!(".galvolite/handoffs/{id}/pointer.json"));
    assert!(pointer.exists(), "auto-handoff pointer not written at {}", pointer.display());
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
