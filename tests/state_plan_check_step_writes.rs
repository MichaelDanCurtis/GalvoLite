mod common;

use galvolite::api::events::DomainEvent;
use galvolite::state::{apply_event_with_bus, EventInput, Source, SessionState};

#[tokio::test]
async fn plan_check_step_updates_one_row_and_emits_step_changed() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;

    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.set_plan".into(),
        payload: serde_json::json!({ "markdown": "- [ ] First step\n- [ ] Second step\n" }),
        dedup_key: None,
    }).unwrap();

    let mut rx = app.mgr.subscribe_events();

    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.plan_check_step".into(),
        payload: serde_json::json!({
            "title": "First step",
            "status": "done",
            "note": "ok"
        }),
        dedup_key: None,
    }).unwrap();

    let rows = app.db.read_plan_steps(sid).unwrap();
    let first = rows.iter().find(|r| r["text"] == "First step").unwrap();
    assert_eq!(first["status"], "done");
    assert_eq!(first["note"], "ok");

    let mut saw = false;
    for _ in 0..8 {
        if let Ok(Ok(DomainEvent::SessionPlanStepChanged { prev_status, next_status, note, .. }))
            = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            assert_eq!(prev_status, "pending");
            assert_eq!(next_status, "done");
            assert_eq!(note.as_deref(), Some("ok"));
            saw = true;
            break;
        }
    }
    assert!(saw);
}

#[tokio::test]
async fn plan_check_step_with_unknown_title_is_a_no_op() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;
    let outcome = apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.plan_check_step".into(),
        payload: serde_json::json!({ "title": "missing", "status": "done" }),
        dedup_key: None,
    });
    assert!(outcome.is_ok());
}
