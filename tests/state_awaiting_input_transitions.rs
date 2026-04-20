//! Verifies that the `system.awaiting_input` synthetic event (emitted by the
//! `on_stop` watchdog) actually moves the session to `AwaitingInput` from the
//! states the watchdog can fire from. Without these transition arms the
//! watchdog would silently no-op.

use galvolite::db::{Db, NewProject, NewSession};
use galvolite::state::{self, EventInput, SessionState, Source};
use serde_json::json;
use tempfile::TempDir;

fn seed(db: &Db) -> uuid::Uuid {
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: "/tmp/awaiting-input-test".into(),
            remote_url: None,
        })
        .unwrap();
    db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: "/tmp/awaiting-input-test".into(),
        task_summary: "t".into(),
        parent_session_id: None,
    })
    .unwrap()
    .id
}

fn force_state(db: &Db, id: uuid::Uuid, s: SessionState) {
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET state = ?1 WHERE id = ?2",
        rusqlite::params![s.to_string(), id.to_string()],
    )
    .unwrap();
}

fn awaiting_input(db: &Db, id: uuid::Uuid) -> SessionState {
    state::apply_event(
        db,
        id,
        EventInput {
            source: Source::System,
            kind: "system.awaiting_input".into(),
            payload: json!({}),
            dedup_key: Some(format!("awaiting:{id}:{}", uuid::Uuid::new_v4())),
        },
    )
    .unwrap()
    .next_state
}

#[test]
fn working_to_awaiting_input() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let id = seed(&db);
    force_state(&db, id, SessionState::Working);
    assert_eq!(awaiting_input(&db, id), SessionState::AwaitingInput);
}

#[test]
fn needs_attention_to_awaiting_input() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let id = seed(&db);
    force_state(&db, id, SessionState::NeedsAttention);
    assert_eq!(awaiting_input(&db, id), SessionState::AwaitingInput);
}

#[test]
fn idle_to_awaiting_input() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let id = seed(&db);
    force_state(&db, id, SessionState::Idle);
    assert_eq!(awaiting_input(&db, id), SessionState::AwaitingInput);
}
