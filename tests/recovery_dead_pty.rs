mod common;

#[tokio::test]
async fn dead_pty_session_transitions_to_error_with_auto_handoff() {
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
    // pid 999_999_999 is reliably absent on Unix.
    db.pool()
        .get()
        .unwrap()
        .execute(
            "UPDATE sessions SET state='working', pty_pid=999999999 WHERE id=?1",
            rusqlite::params![s.id.to_string()],
        )
        .unwrap();

    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let _mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::with_event_bus(
            db.clone(),
            dir.path().to_path_buf(),
            bus,
        ),
    );

    let conn = db.pool().get().unwrap();
    let state: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id=?1",
            [s.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "error");

    let handoff: Option<String> = conn
        .query_row(
            "SELECT handoff_path FROM sessions WHERE id=?1",
            [s.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(handoff.is_some(), "auto-recovery handoff path must be stamped");
    assert!(std::path::Path::new(handoff.as_deref().unwrap()).exists());
}
