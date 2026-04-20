mod common;

use galvolite::api::events::{DomainEvent, EventBroadcaster};
use galvolite::state::SessionState;

#[tokio::test]
async fn broadcaster_fans_session_state_changed_to_subscribers() {
    let bus = EventBroadcaster::new(64);
    let mut rx = bus.subscribe();
    let sid = uuid::Uuid::new_v4();

    bus.publish(DomainEvent::SessionStateChanged {
        session_id: sid,
        prev: Some(SessionState::Idle),
        next: SessionState::Working,
        last_event_ts: "2026-04-19T00:00:00Z".into(),
    });

    let got = rx.recv().await.unwrap();
    match got {
        DomainEvent::SessionStateChanged { session_id, next, .. } => {
            assert_eq!(session_id, sid);
            assert_eq!(next, SessionState::Working);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[tokio::test]
async fn broadcaster_serializes_to_tagged_json_frames() {
    let _bus = EventBroadcaster::new(64);
    let sid = uuid::Uuid::new_v4();
    let evt = DomainEvent::OperatorToast {
        session_id: sid,
        level: "warn".into(),
        message: "low fuel".into(),
    };
    let json = serde_json::to_value(&evt).unwrap();
    assert_eq!(json["type"], "operator.toast");
    assert_eq!(json["level"], "warn");
    assert_eq!(json["session_id"], sid.to_string());
}

#[tokio::test]
async fn session_manager_exposes_event_broadcaster() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db,
        dir.path().to_path_buf(),
        bus.clone(),
    ));
    let mut rx = mgr.subscribe_events();
    bus.publish(galvolite::api::events::DomainEvent::OperatorToast {
        session_id: uuid::Uuid::new_v4(),
        level: "info".into(),
        message: "hi".into(),
    });
    let evt = rx.recv().await.unwrap();
    assert!(matches!(evt, galvolite::api::events::DomainEvent::OperatorToast { .. }));
}
