mod common;

use futures_util::StreamExt;
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

#[tokio::test]
async fn apply_event_publishes_state_changed_and_event_appended() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db.clone(), dir.path().to_path_buf(), bus.clone(),
    ));

    let proj = db.insert_project(galvolite::db::NewProject {
        name: "p".into(),
        path: dir.path().to_string_lossy().into(),
        remote_url: None,
    }).unwrap();
    let s = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: dir.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    db.pool().get().unwrap().execute(
        "UPDATE sessions SET state = 'idle' WHERE id = ?1",
        rusqlite::params![s.id.to_string()],
    ).unwrap();

    let mut rx = mgr.subscribe_events();
    galvolite::state::apply_event_with_bus(
        &db, s.id, mgr.event_bus(),
        galvolite::state::EventInput {
            source: galvolite::state::Source::Hook,
            kind: "hook.UserPromptSubmit".into(),
            payload: serde_json::json!({}),
            dedup_key: None,
        },
    ).unwrap();

    let first = rx.recv().await.unwrap();
    let second = rx.recv().await.unwrap();
    assert!(matches!(first, galvolite::api::events::DomainEvent::SessionEventAppended { .. }));
    assert!(matches!(second, galvolite::api::events::DomainEvent::SessionStateChanged {
        next: galvolite::state::SessionState::Working, ..
    }));
}

#[tokio::test]
async fn ws_emits_domain_events_as_text_frames() {
    use tokio_tungstenite::tungstenite::Message;
    // Start the real router on an ephemeral port.
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db.clone(),
        dir.path().to_path_buf(),
        bus.clone(),
    ));
    let app = galvolite::api::router(mgr.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    // Spawn a real session so the WS route accepts the subscribe (the
    // terminal route requires a live PTY-backed session).
    let proj = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(
            proj.id,
            "echo".into(),
            "implementation".into(),
            "cat".into(),
            vec![],
        )
        .await
        .unwrap();
    let sid = session.id;
    let token = format!("tok_{}", uuid::Uuid::new_v4().simple());
    db.set_api_token(sid, &token).unwrap();

    let url = format!("ws://127.0.0.1:{port}/api/sessions/{sid}/terminal?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    bus.publish(galvolite::api::events::DomainEvent::OperatorToast {
        session_id: sid,
        level: "info".into(),
        message: "hi".into(),
    });

    // Skip any binary frames (PTY output) until we see the JSON text frame.
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(t) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    assert_eq!(v["type"], "operator.toast");
                    break;
                }
                _ => continue,
            }
        }
    })
    .await;
    assert!(got.is_ok(), "did not receive operator.toast text frame in time");
}

#[tokio::test]
async fn ws_funnel_serializes_pty_and_event_writes_through_one_writer() {
    use tokio_tungstenite::tungstenite::Message;
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db.clone(),
        dir.path().to_path_buf(),
        bus.clone(),
    ));
    let app = galvolite::api::router(mgr.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    let proj = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(proj.id, "echo".into(), "implementation".into(), "cat".into(), vec![])
        .await
        .unwrap();
    let sid = session.id;
    let token = format!("tok_{}", uuid::Uuid::new_v4().simple());
    db.set_api_token(sid, &token).unwrap();

    let url = format!("ws://127.0.0.1:{port}/api/sessions/{sid}/terminal?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // Publish 10 events in fast succession.
    for i in 0..10 {
        bus.publish(galvolite::api::events::DomainEvent::OperatorToast {
            session_id: sid,
            level: "info".into(),
            message: format!("msg-{i}"),
        });
    }

    // Drain text frames until we see all 10 in order.
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut seen: Vec<String> = Vec::new();
        while seen.len() < 10 {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(t) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if v["type"] == "operator.toast" {
                        seen.push(v["message"].as_str().unwrap().to_string());
                    }
                }
                _ => continue, // skip binary PTY frames
            }
        }
        seen
    })
    .await
    .expect("did not receive all 10 toast frames in time");

    let expected: Vec<String> = (0..10).map(|i| format!("msg-{i}")).collect();
    assert_eq!(got, expected, "frames lost or reordered through funnel");
}
