use galvolite::db::{Db, NewProject, NewSession};
use galvolite::error::Error;
use galvolite::state::{apply_event, EventInput, SessionState, Source};
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

fn ev(kind: &str) -> EventInput {
    EventInput {
        source: Source::System,
        kind: kind.into(),
        payload: json!({}),
        dedup_key: None,
    }
}

#[test]
fn user_prompt_submit_transitions_idle_to_working() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("system.session_started")).unwrap();
    let out = apply_event(&db, sid, ev("hook.UserPromptSubmit")).unwrap();
    assert_eq!(out.next_state, SessionState::Working);
    assert!(!out.deduped);

    let conn = db.pool().get().unwrap();
    let state: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "working", "sessions.state row should be updated");
}

#[test]
fn stop_max_retries_transitions_working_to_idle() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("system.session_started")).unwrap();
    apply_event(&db, sid, ev("hook.UserPromptSubmit")).unwrap();
    let out = apply_event(&db, sid, ev("system.stop_max_retries")).unwrap();
    assert_eq!(out.prev_state, Some(SessionState::Working));
    assert_eq!(out.next_state, SessionState::Idle);
}

#[test]
fn attention_needed_transitions_to_needs_attention() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("system.session_started")).unwrap();
    apply_event(&db, sid, ev("hook.UserPromptSubmit")).unwrap();
    let out = apply_event(&db, sid, ev("system.attention_needed")).unwrap();
    assert_eq!(out.next_state, SessionState::NeedsAttention);
}

#[test]
fn dedup_key_collision_returns_deduped_outcome_no_second_row() {
    let (db, _t, sid) = open_db_with_session();
    let mut e = ev("hook.UserPromptSubmit");
    e.dedup_key = Some("hook-1".into());

    let first = apply_event(&db, sid, e.clone()).unwrap();
    assert!(!first.deduped);

    let second = apply_event(&db, sid, e).unwrap();
    assert!(second.deduped);
    assert_eq!(
        second.event_id, first.event_id,
        "deduped call returns prior event_id"
    );

    let conn = db.pool().get().unwrap();
    let cnt: i64 = conn
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cnt, 1, "deduped event must not write a second row");
}

#[test]
fn event_row_records_prev_and_next_state() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("system.session_started")).unwrap();
    apply_event(&db, sid, ev("hook.UserPromptSubmit")).unwrap();
    apply_event(&db, sid, ev("system.stop_max_retries")).unwrap();

    let conn = db.pool().get().unwrap();
    let mut stmt = conn
        .prepare("SELECT prev_state, next_state FROM events WHERE session_id = ?1 ORDER BY id ASC")
        .unwrap();
    let rows: Vec<(Option<String>, Option<String>)> = stmt
        .query_map([sid.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0.as_deref(), Some("starting"));
    assert_eq!(rows[0].1.as_deref(), Some("idle"));
    assert_eq!(rows[1].0.as_deref(), Some("idle"));
    assert_eq!(rows[1].1.as_deref(), Some("working"));
    assert_eq!(rows[2].0.as_deref(), Some("working"));
    assert_eq!(rows[2].1.as_deref(), Some("idle"));
}

#[test]
fn terminal_state_rejects_further_transitions() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("system.session_ended")).unwrap();
    let err = apply_event(&db, sid, ev("hook.UserPromptSubmit")).unwrap_err();
    match err {
        Error::TerminalState { session_id, kind } => {
            assert_eq!(session_id, sid.to_string());
            assert_eq!(kind, "hook.UserPromptSubmit");
        }
        other => panic!("expected TerminalState, got {other:?}"),
    }
}
