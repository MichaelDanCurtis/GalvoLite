use galvolite::db::{Db, NewProject, NewSession};

fn fresh_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("t.sqlite")).unwrap();
    (db, dir)
}

fn seed(db: &Db) -> uuid::Uuid {
    let p = db.insert_project(NewProject {
        name: "p".into(),
        path: format!("/tmp/{}", uuid::Uuid::new_v4()),
        remote_url: None,
    }).unwrap();
    let s = db.insert_session(NewSession {
        project_id: p.id, role: "implementation".into(),
        branch: None, worktree_path: p.path,
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    s.id
}

#[test]
fn upsert_inserts_new_then_updates_existing() {
    let (db, _d) = fresh_db();
    let sid = seed(&db);
    let mut conn = db.pool().get().unwrap();
    let tx = conn.transaction().unwrap();
    Db::upsert_plan_step_in_tx(&tx, sid, "h1", 0, "First", "pending", None).unwrap();
    Db::upsert_plan_step_in_tx(&tx, sid, "h1", 0, "First", "in_progress", Some("note")).unwrap();
    tx.commit().unwrap();

    let rows = db.read_plan_steps(sid).unwrap();
    assert_eq!(rows.len(), 1);
    let conn2 = db.pool().get().unwrap();
    let s: String = conn2.query_row(
        "SELECT status FROM plan_steps WHERE step_hash = 'h1'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(s, "in_progress");
}

#[test]
fn delete_removes_only_rows_for_session() {
    let (db, _d) = fresh_db();
    let s1 = seed(&db); let s2 = seed(&db);
    let mut conn = db.pool().get().unwrap();
    let tx = conn.transaction().unwrap();
    Db::upsert_plan_step_in_tx(&tx, s1, "h1", 0, "A", "pending", None).unwrap();
    Db::upsert_plan_step_in_tx(&tx, s2, "h2", 0, "B", "pending", None).unwrap();
    Db::delete_plan_steps_for_session_in_tx(&tx, s1).unwrap();
    tx.commit().unwrap();
    assert_eq!(db.read_plan_steps(s1).unwrap().len(), 0);
    assert_eq!(db.read_plan_steps(s2).unwrap().len(), 1);
}

#[test]
fn set_status_updates_one_row_and_returns_prev() {
    let (db, _d) = fresh_db();
    let sid = seed(&db);
    let mut conn = db.pool().get().unwrap();
    let tx = conn.transaction().unwrap();
    Db::upsert_plan_step_in_tx(&tx, sid, "h1", 0, "A", "pending", None).unwrap();
    let prev = Db::set_plan_step_status_in_tx(&tx, sid, "h1", "done", Some("ok")).unwrap();
    tx.commit().unwrap();
    assert_eq!(prev.as_deref(), Some("pending"));
}

#[test]
fn set_status_returns_none_when_step_not_found() {
    let (db, _d) = fresh_db();
    let sid = seed(&db);
    let mut conn = db.pool().get().unwrap();
    let tx = conn.transaction().unwrap();
    let prev = Db::set_plan_step_status_in_tx(&tx, sid, "missing", "done", None).unwrap();
    tx.commit().unwrap();
    assert!(prev.is_none());
}
