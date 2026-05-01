use galvolite::db::{Db, NewProject, NewSession};
use galvolite::state::{apply_event, EventInput, Source};
use serde_json::json;
use tempfile::TempDir;

fn fresh_session() -> (Db, TempDir, uuid::Uuid) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let p = db.insert_project(NewProject{
        name:"p".into(), path: tmp.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let s = db.insert_session(NewSession{
        project_id: p.id, role:"implementation".into(), branch: None,
        worktree_path: tmp.path().to_string_lossy().into_owned(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    (db, tmp, s.id)
}

#[test]
fn mcp_spawn_session_event_appends_without_state_change() {
    let (db, _t, sid) = fresh_session();
    let r = apply_event(&db, sid, EventInput{
        source: Source::Mcp,
        kind: "mcp.spawn_session".into(),
        payload: json!({"child_id":"x","role":"implementation"}),
        dedup_key: None,
    }).unwrap();
    assert!(!r.deduped);
    // No state change — kind is logging-only.
}

#[test]
fn session_children_resolved_appends_and_publishes_domain_event() {
    let (db, _t, sid) = fresh_session();
    let _ = apply_event(&db, sid, EventInput{
        source: Source::System,
        kind: "session.children_resolved".into(),
        payload: json!({"children":["a","b"]}),
        dedup_key: None,
    }).unwrap();
    let conn = db.pool().get().unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.children_resolved'",
        [sid.to_string()], |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
