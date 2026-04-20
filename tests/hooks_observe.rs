mod common;

use common::{post_hook, seed_session_in_state, state_of, test_app};
use galvolite::hooks::types::HookKind;
use galvolite::state::SessionState;
use serde_json::json;

#[tokio::test]
async fn post_tool_use_git_push_raises_attention() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::PostToolUse,
        json!({
            "tool": "Bash",
            "command": "git push origin main",
            "exit_code": 0,
            "stdout_preview": "..."
        }),
    )
    .await;

    assert_eq!(state_of(&app.db, id), SessionState::NeedsAttention);
}

#[tokio::test]
async fn post_tool_use_git_commit_logs_but_no_attention() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::PostToolUse,
        json!({"tool": "Bash", "command": "git commit -m 'x'", "exit_code": 0}),
    )
    .await;

    assert_eq!(state_of(&app.db, id), SessionState::Working);

    let conn = app.db.pool().get().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'hook.PostToolUse'",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn post_tool_use_unrelated_command_logs_minimally() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::PostToolUse,
        json!({"tool": "Bash", "command": "ls -la", "exit_code": 0}),
    )
    .await;

    assert_eq!(state_of(&app.db, id), SessionState::Working);
}

#[tokio::test]
async fn notification_records_level_and_message() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::Notification,
        json!({"level": "warn", "message": "test passed but lints failed"}),
    )
    .await;

    let conn = app.db.pool().get().unwrap();
    let payload: String = conn
        .query_row(
            "SELECT payload_json FROM events WHERE session_id = ?1 AND kind = 'hook.Notification'",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(payload.contains("warn"));
    assert!(payload.contains("test passed"));
}
