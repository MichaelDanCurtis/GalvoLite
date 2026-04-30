mod common;

#[tokio::test]
async fn recovery_replays_last_20_events_for_alive_pty() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let proj = db
        .insert_project(galvolite::db::NewProject {
            name: "p".into(),
            path: dir.path().to_string_lossy().into(),
            remote_url: None,
        })
        .unwrap();
    let s = db
        .insert_session(galvolite::db::NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: dir.path().to_string_lossy().into(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET state='working', pty_pid=?1 WHERE id=?2",
        rusqlite::params![std::process::id() as i64, s.id.to_string()],
    )
    .unwrap();
    for i in 0..25 {
        galvolite::state::apply_event(
            &db,
            s.id,
            galvolite::state::EventInput {
                source: galvolite::state::Source::System,
                kind: "system.tick".into(),
                payload: serde_json::json!({ "i": i }),
                dedup_key: Some(format!("tick-{i}")),
            },
        )
        .unwrap();
    }

    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mut rx = bus.subscribe();
    let _mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db,
        dir.path().to_path_buf(),
        bus,
    ));

    let mut saw_resumed = false;
    let mut saw_marker = false;
    for _ in 0..40 {
        match tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(galvolite::api::events::DomainEvent::SessionResumed {
                session_id,
                last_events,
            })) => {
                assert_eq!(session_id, s.id);
                assert_eq!(last_events.as_array().unwrap().len(), 20);
                saw_resumed = true;
            }
            Ok(Ok(galvolite::api::events::DomainEvent::SessionEventAppended { kind, .. }))
                if kind == "system.recovered" =>
            {
                saw_marker = true;
            }
            _ => {}
        }
    }
    assert!(saw_resumed && saw_marker);
}
