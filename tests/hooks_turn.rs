mod common;

use common::{
    bump_tokens, counters_of, post_hook, record_event, seed_session_in_state, state_of, test_app,
};
use galvolite::hooks::types::HookKind;
use galvolite::state::SessionState;
use serde_json::json;

#[tokio::test]
async fn user_prompt_submit_transitions_idle_to_working_increments_counter() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Idle);

    let _ = post_hook(
        &app,
        &token,
        &id,
        HookKind::UserPromptSubmit,
        json!({"prompt_preview": "Hello", "est_tokens": 50}),
    )
    .await;

    assert_eq!(state_of(&app.db, id), SessionState::Working);
    let (turns, tokens) = counters_of(&app.db, id);
    assert_eq!(turns, 1);
    assert_eq!(tokens, 50);
}

#[tokio::test]
async fn user_prompt_submit_at_80_percent_injects_warning() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Idle);
    bump_tokens(&app.db, id, 25_000); // < 80%

    let resp = post_hook(
        &app,
        &token,
        &id,
        HookKind::UserPromptSubmit,
        json!({"prompt_preview": "x", "est_tokens": 1500}),
    )
    .await;
    // 26500 / 32000 = 82.8% — should inject
    assert!(resp.inject_text.is_some(), "should warn at 80% context");
    assert!(resp
        .inject_text
        .unwrap()
        .to_lowercase()
        .contains("context"));
}

#[tokio::test]
async fn stop_first_two_inject_report_status_or_continue() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let r1 = post_hook(&app, &token, &id, HookKind::Stop, json!({"turn_id": "t1"})).await;
    assert!(r1.inject_text.is_some());

    let r2 = post_hook(&app, &token, &id, HookKind::Stop, json!({"turn_id": "t2"})).await;
    assert!(r2.inject_text.is_some());
}

#[tokio::test]
async fn stop_third_consecutive_transitions_working_to_idle_no_inject() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    for n in 1..=3 {
        let _ = post_hook(
            &app,
            &token,
            &id,
            HookKind::Stop,
            json!({"turn_id": format!("t{n}")}),
        )
        .await;
    }
    let final_state = state_of(&app.db, id);
    assert_eq!(final_state, SessionState::Idle);
}

#[tokio::test]
async fn precompact_with_no_recent_write_handoff_injects_handoff_prompt() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);

    let resp = post_hook(
        &app,
        &token,
        &id,
        HookKind::PreCompact,
        json!({"trigger_reason": "context_full"}),
    )
    .await;
    let inject = resp.inject_text.expect("should inject handoff prompt");
    assert!(inject.to_lowercase().contains("write_handoff"));
}

#[tokio::test]
async fn precompact_with_recent_write_handoff_does_not_inject() {
    let app = test_app();
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working);
    record_event(&app.db, id, "mcp.write_handoff", json!({"summary": "done"})).await;

    let resp = post_hook(
        &app,
        &token,
        &id,
        HookKind::PreCompact,
        json!({"trigger_reason": "manual"}),
    )
    .await;
    assert!(resp.inject_text.is_none());
}
