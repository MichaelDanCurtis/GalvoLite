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
