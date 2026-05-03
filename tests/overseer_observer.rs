use galvolite::api::events::{DomainEvent, EventBroadcaster};
use galvolite::overseer::observer::{ContextSnapshot, Observer};
use uuid::Uuid;

#[tokio::test]
async fn captures_session_event_appended_into_ring() {
    let bus = EventBroadcaster::new(64);
    let pid = Uuid::new_v4();
    let mut obs = Observer::new(pid, bus.subscribe());
    let sid = Uuid::new_v4();
    bus.publish(DomainEvent::SessionEventAppended {
        session_id: sid, event_id: 1, kind: "mcp.status_report".into(),
        source: "mcp".into(), ts: "2026-05-02T00:00:00Z".into(),
        payload: serde_json::json!({"kind":"stuck"}),
    });
    obs.drain_pending(50).await;
    let snap: ContextSnapshot = obs.snapshot();
    assert_eq!(snap.events.len(), 1);
    assert_eq!(snap.events[0].kind, "mcp.status_report");
}

#[tokio::test]
async fn ring_caps_at_capacity() {
    let bus = EventBroadcaster::new(1024);
    let pid = Uuid::new_v4();
    let mut obs = Observer::new(pid, bus.subscribe());
    for i in 0..250 {
        bus.publish(DomainEvent::SessionEventAppended {
            session_id: Uuid::new_v4(), event_id: i, kind: "k".into(),
            source: "system".into(), ts: "t".into(), payload: serde_json::json!({}),
        });
    }
    obs.drain_pending(500).await;
    let snap = obs.snapshot();
    assert!(snap.events.len() <= 200, "events ring should cap at 200");
}
