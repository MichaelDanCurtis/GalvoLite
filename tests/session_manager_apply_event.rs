use galvolite::db::{Db, NewProject};
use galvolite::session_manager::SessionManager;
use galvolite::state::SessionState;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn spawn_then_kill_writes_lifecycle_events() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let data_dir = tmp.path().to_path_buf();

    // Create a project pointing at a real existing dir (canonicalize succeeds).
    let proj_dir = TempDir::new().unwrap();
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let proj = mgr
        .add_project(NewProject {
            name: "p".into(),
            path: proj_dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Use `echo` as the spawn command (matches existing session_manager.rs tests).
    let session = mgr
        .spawn_session(
            proj.id,
            "task".into(),
            "implementation".into(),
            "echo".into(),
            vec![],
        )
        .await
        .unwrap();

    // Give the bg task a moment if echo races to exit before kill_session
    // — kill_session itself must still emit system.session_ended via apply_event,
    // but the dedup_key collapses any race between bg-task pty_exit + kill.
    let _ = mgr.kill_session(session.id).await;

    // Assert events log contains a session_started and session_ended row.
    let conn = db.pool().get().unwrap();
    let mut stmt = conn
        .prepare("SELECT kind, next_state FROM events WHERE session_id = ?1 ORDER BY id ASC")
        .unwrap();
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([session.id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(
        rows.iter().any(|(k, _)| k == "system.session_started"),
        "expected system.session_started in events; have {rows:?}"
    );
    let last_ended = rows.iter().rfind(|(k, _)| k == "system.session_ended");
    assert!(
        last_ended.is_some(),
        "expected system.session_ended; have {rows:?}"
    );
    assert_eq!(
        last_ended.unwrap().1.as_deref(),
        Some("ended"),
        "ended event should set next_state to 'ended'"
    );

    // Final sessions.state row matches.
    let final_state: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        SessionState::from_str(&final_state).unwrap(),
        SessionState::Ended
    );
}
