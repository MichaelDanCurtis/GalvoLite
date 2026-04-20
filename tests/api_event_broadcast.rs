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
