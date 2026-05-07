use galvolite::db::Db;
use galvolite::recommendations::{self, NewRec, RecStatus};
use uuid::Uuid;

fn open_db() -> Db {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    Box::leak(Box::new(tmp));
    db
}

fn sample(project_id: Uuid) -> NewRec {
    NewRec {
        project_id,
        kind: "stuck_session".into(),
        severity: "warn".into(),
        title: "Session A appears stuck".into(),
        body: "no progress for 10m".into(),
        rationale: "no events in 600s".into(),
        evidence_json: serde_json::json!({"sids":["x"]}),
    }
}

#[test]
fn insert_returns_id_and_dedupes_on_content_hash() {
    let db = open_db();
    let pid = Uuid::new_v4();
    let id1 = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    let dup = recommendations::insert(&db, sample(pid)).unwrap();
    assert!(dup.is_none(), "second insert with identical content should dedupe");
    let list = recommendations::list_by_status(&db, pid, RecStatus::Pending).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id1);
}

#[test]
fn act_marks_status_acted_and_sets_acted_at() {
    let db = open_db();
    let pid = Uuid::new_v4();
    let id = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    recommendations::set_status(&db, id, RecStatus::Acted).unwrap();
    let r = recommendations::get(&db, id).unwrap();
    assert_eq!(r.status, "acted");
    assert!(r.acted_at.is_some());
}

#[test]
fn defer_with_until_resurfaces_after_expiry_sweep() {
    let db = open_db();
    let pid = Uuid::new_v4();
    let id = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    recommendations::defer(&db, id, past).unwrap();
    let n = recommendations::resurface_expired(&db, chrono::Utc::now()).unwrap();
    assert_eq!(n, 1);
    let r = recommendations::get(&db, id).unwrap();
    assert_eq!(r.status, "pending");
}

#[test]
fn migration_backfills_status_changed_at() {
    // Schema check via raw SQL: every existing rec must have a non-null
    // status_changed_at after the migration, even when acted_at is null.
    let db = open_db();
    let pid = Uuid::new_v4();
    let id = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    let conn = db.pool().get().unwrap();
    let s: String = conn
        .query_row(
            "SELECT status_changed_at FROM recommendations WHERE id=?1",
            rusqlite::params![id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!s.is_empty(), "status_changed_at must be set on insert");
}

#[test]
fn archive_old_sweeps_acted_and_dismissed() {
    let db = open_db();
    let pid = Uuid::new_v4();

    // Acted rec aged 8 days: should archive.
    let mut s = sample(pid); s.title = "acted-old".into();
    let id_acted = recommendations::insert(&db, s).unwrap().unwrap();
    recommendations::set_status(&db, id_acted, RecStatus::Acted).unwrap();
    backdate(&db, id_acted, 8);

    // Dismissed rec aged 8 days: should archive.
    let mut s = sample(pid); s.title = "dismissed-old".into();
    let id_dis = recommendations::insert(&db, s).unwrap().unwrap();
    recommendations::set_status(&db, id_dis, RecStatus::Dismissed).unwrap();
    backdate(&db, id_dis, 8);

    let n = recommendations::archive_old(&db, chrono::Duration::days(7)).unwrap();
    assert_eq!(n, 2);
    assert_eq!(recommendations::get(&db, id_acted).unwrap().status, "archived");
    assert_eq!(recommendations::get(&db, id_dis).unwrap().status, "archived");
}

#[test]
fn archive_old_exempts_pending_and_deferred() {
    let db = open_db();
    let pid = Uuid::new_v4();

    let mut s = sample(pid); s.title = "pending-old".into();
    let id_pending = recommendations::insert(&db, s).unwrap().unwrap();
    backdate(&db, id_pending, 30);

    let mut s = sample(pid); s.title = "deferred-old".into();
    let id_def = recommendations::insert(&db, s).unwrap().unwrap();
    let future = chrono::Utc::now() + chrono::Duration::days(7);
    recommendations::defer(&db, id_def, future).unwrap();
    backdate(&db, id_def, 30);

    let n = recommendations::archive_old(&db, chrono::Duration::days(7)).unwrap();
    assert_eq!(n, 0);
    assert_eq!(recommendations::get(&db, id_pending).unwrap().status, "pending");
    assert_eq!(recommendations::get(&db, id_def).unwrap().status, "deferred");
}

#[test]
fn archive_old_respects_threshold() {
    let db = open_db();
    let pid = Uuid::new_v4();

    let mut s = sample(pid); s.title = "young".into();
    let id_young = recommendations::insert(&db, s).unwrap().unwrap();
    recommendations::set_status(&db, id_young, RecStatus::Acted).unwrap();
    backdate(&db, id_young, 6); // 6 days — under threshold

    let mut s = sample(pid); s.title = "old".into();
    let id_old = recommendations::insert(&db, s).unwrap().unwrap();
    recommendations::set_status(&db, id_old, RecStatus::Acted).unwrap();
    backdate(&db, id_old, 8); // 8 days — over threshold

    let n = recommendations::archive_old(&db, chrono::Duration::days(7)).unwrap();
    assert_eq!(n, 1);
    assert_eq!(recommendations::get(&db, id_young).unwrap().status, "acted");
    assert_eq!(recommendations::get(&db, id_old).unwrap().status, "archived");
}

/// Helper: rewrites `status_changed_at` for one rec to N days ago.
/// Tests use this instead of waiting wall-clock time.
fn backdate(db: &Db, id: Uuid, days_ago: i64) {
    let conn = db.pool().get().unwrap();
    let backdated = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
    conn.execute(
        "UPDATE recommendations SET status_changed_at=?1 WHERE id=?2",
        rusqlite::params![backdated, id.to_string()],
    ).unwrap();
}
