mod common;

use galvolite::api::events::DomainEvent;
use galvolite::state::{apply_event_with_bus, EventInput, Source, SessionState};

#[tokio::test]
async fn set_plan_writes_plan_steps_and_publishes_replaced_with_per_step_changes() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;
    let mut rx = app.mgr.subscribe_events();

    apply_event_with_bus(
        &app.db, sid, app.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.set_plan".into(),
            payload: serde_json::json!({
                "markdown": "- [ ] Step one\n- [x] Step two\n"
            }),
            dedup_key: None,
        },
    ).unwrap();

    let rows = app.db.read_plan_steps(sid).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["text"], "Step one");
    assert_eq!(rows[0]["status"], "pending");
    assert_eq!(rows[1]["status"], "done");

    let mut saw_event_appended = false;
    let mut saw_replaced       = false;
    for _ in 0..6 {
        match tokio::time::timeout(
            std::time::Duration::from_millis(200),
            rx.recv(),
        ).await {
            Ok(Ok(DomainEvent::SessionEventAppended { kind, .. })) if kind == "mcp.set_plan" => {
                saw_event_appended = true;
            }
            Ok(Ok(DomainEvent::SessionPlanReplaced { steps, .. })) => {
                saw_replaced = true;
                assert_eq!(steps.as_array().unwrap().len(), 2);
            }
            _ => {}
        }
    }
    assert!(saw_event_appended);
    assert!(saw_replaced);
}

#[tokio::test]
async fn set_plan_with_one_status_change_emits_one_step_changed_event() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;

    // First plan: A pending, B pending.
    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.set_plan".into(),
        payload: serde_json::json!({ "markdown": "- [ ] A\n- [ ] B\n" }),
        dedup_key: None,
    }).unwrap();

    // Second plan: A done, B pending.
    let mut rx = app.mgr.subscribe_events();
    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.set_plan".into(),
        payload: serde_json::json!({ "markdown": "- [x] A\n- [ ] B\n" }),
        dedup_key: None,
    }).unwrap();

    let mut saw_change = false;
    for _ in 0..8 {
        if let Ok(Ok(DomainEvent::SessionPlanStepChanged { prev_status, next_status, .. }))
            = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            assert_eq!(prev_status, "pending");
            assert_eq!(next_status, "done");
            saw_change = true;
            break;
        }
    }
    assert!(saw_change);
}
