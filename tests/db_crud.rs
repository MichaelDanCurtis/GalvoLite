use galvolite::db::{Db, NewProject, NewSession, Project, Session, SessionState};
use tempfile::TempDir;

fn open_db() -> (TempDir, Db) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    (tmp, db)
}

#[test]
fn insert_and_fetch_project() {
    let (_tmp, db) = open_db();
    let p = db.insert_project(NewProject {
        name: "Galvo".into(),
        path: "/tmp/galvo".into(),
        remote_url: None,
    }).unwrap();

    let fetched: Project = db.get_project(p.id).unwrap();
    assert_eq!(fetched.name, "Galvo");
    assert_eq!(fetched.path, "/tmp/galvo");
}

#[test]
fn insert_session_starts_in_starting() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();

    let s = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: Some("main".into()),
        worktree_path: "/tmp/p".into(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    assert_eq!(s.state, SessionState::Starting);
    let fetched = db.get_session(s.id).unwrap();
    assert_eq!(fetched.task_summary, "test");
}

#[test]
fn update_session_state_persists() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();
    let s = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: "/tmp/p".into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    db.update_session_state(s.id, SessionState::Working).unwrap();
    let fetched: Session = db.get_session(s.id).unwrap();
    assert_eq!(fetched.state, SessionState::Working);
}

// `record_signal_and_list` test removed in Sub-plan 5 Task 12 — the
// `record_signal` / `list_signals_for_session` helpers no longer exist.
// The `signals` SQL table is preserved in 0001_initial.sql for backward
// compat but nothing writes to it.

#[test]
fn list_sessions_returns_full_data_without_n_plus_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("test.sqlite")).unwrap();

    let proj = db.insert_project(galvolite::db::NewProject {
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
    }).unwrap();

    let s1 = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: Some("feat/one".into()),
        worktree_path: "/tmp/test".into(),
        task_summary: "first task".into(),
        parent_session_id: None,
    }).unwrap();

    let s2 = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id,
        role: "brainstorm".into(),
        branch: None,
        worktree_path: "/tmp/test".into(),
        task_summary: "second task".into(),
        parent_session_id: None,
    }).unwrap();

    let sessions = db.list_sessions().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, s1.id);
    assert_eq!(sessions[0].task_summary, "first task");
    assert_eq!(sessions[0].branch, Some("feat/one".into()));
    assert_eq!(sessions[1].id, s2.id);
    assert_eq!(sessions[1].role, "brainstorm");
}

#[test]
fn session_state_serializes_to_snake_case() {
    let json = serde_json::to_string(&galvolite::db::SessionState::NeedsAttention).unwrap();
    assert_eq!(json, "\"needs_attention\"");

    let json = serde_json::to_string(&galvolite::db::SessionState::AwaitingInput).unwrap();
    assert_eq!(json, "\"awaiting_input\"");

    let back: galvolite::db::SessionState = serde_json::from_str("\"working\"").unwrap();
    assert_eq!(back, galvolite::db::SessionState::Working);
}

#[test]
fn end_session_if_unended_is_conditional() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();
    let s = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: "/tmp/p".into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    // First call: session is unended, should update and return true.
    let did = db.end_session_if_unended(s.id, SessionState::Ended).unwrap();
    assert!(did);
    let fetched = db.get_session(s.id).unwrap();
    assert_eq!(fetched.state, SessionState::Ended);
    assert!(fetched.ended_at.is_some());
    let first_ended_at = fetched.ended_at.unwrap();

    // Second call: already ended, should be a no-op returning false.
    // Try to clobber with another terminal write — must not succeed.
    // (v1 distinguished Complete vs Killed; v2 collapses both to Ended,
    // with the events log being the new "why" source of truth.)
    let did_again = db.end_session_if_unended(s.id, SessionState::Ended).unwrap();
    assert!(!did_again);
    let fetched = db.get_session(s.id).unwrap();
    assert_eq!(fetched.state, SessionState::Ended);
    assert_eq!(fetched.ended_at.unwrap(), first_ended_at);
}

#[test]
fn end_cycle_if_unended_is_conditional() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();
    let s = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: "/tmp/p".into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    let cycle_id = db.start_cycle(s.id).unwrap();

    let did = db.end_cycle_if_unended(cycle_id, Some("/tmp/handoff.md")).unwrap();
    assert!(did);

    // Second call with different handoff path — must not overwrite.
    let did_again = db.end_cycle_if_unended(cycle_id, Some("/tmp/other.md")).unwrap();
    assert!(!did_again);
}

#[test]
fn project_serializes_to_json() {
    let p = galvolite::db::Project {
        id: uuid::Uuid::nil(),
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
        created_at: chrono::Utc::now(),
    };
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["name"], "test");
    assert_eq!(json["path"], "/tmp/test");
}
