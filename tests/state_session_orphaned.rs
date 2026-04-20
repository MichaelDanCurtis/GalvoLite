use galvolite::db::{Db, NewProject, NewSession};
use galvolite::state::{self, EventInput, SessionState, Source};
use serde_json::json;
use tempfile::TempDir;

fn open_db_with_session() -> (Db, TempDir, uuid::Uuid) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let s = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: tmp.path().to_string_lossy().into_owned(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();
    (db, tmp, s.id)
}

#[test]
fn session_orphaned_transitions_starting_to_error_and_writes_event() {
    let (db, _t, id) = open_db_with_session();

    let outcome = state::apply_event(
        &db,
        id,
        EventInput {
            source: Source::System,
            kind: "system.session_orphaned".into(),
            payload: json!({"reason": "worktree_create_failed"}),
            dedup_key: Some(format!("session-orphaned:{id}")),
        },
    )
    .unwrap();

    assert_eq!(outcome.next_state, SessionState::Error);

    let conn = db.pool().get().unwrap();
    let event_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'system.session_orphaned'",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(event_count, 1);
}

#[test]
fn session_orphaned_works_from_idle_too() {
    let (db, _t, id) = open_db_with_session();

    // Move to a non-Starting state via a signal (Starting → Working then →
    // Idle via STATUS:COMPLETE). Today system.session_started lands the
    // session in Working (Sub-plan 1 deviation; Task 3 will normalize this
    // back to Idle). For this test we just want a non-terminal, non-Starting
    // state, so use the signal bridge.
    state::apply_event(
        &db,
        id,
        EventInput {
            source: Source::System,
            kind: "signal.STATUS:WORKING".into(),
            payload: json!({}),
            dedup_key: None,
        },
    )
    .unwrap();
    state::apply_event(
        &db,
        id,
        EventInput {
            source: Source::System,
            kind: "signal.STATUS:COMPLETE".into(),
            payload: json!({}),
            dedup_key: None,
        },
    )
    .unwrap();

    let outcome = state::apply_event(
        &db,
        id,
        EventInput {
            source: Source::System,
            kind: "system.session_orphaned".into(),
            payload: json!({"reason": "post_spawn_failure"}),
            dedup_key: None,
        },
    )
    .unwrap();

    assert_eq!(outcome.next_state, SessionState::Error);
}
