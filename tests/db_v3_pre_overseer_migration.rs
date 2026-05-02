use galvolite::db::Db;

#[test]
fn migration_0004_adds_recommendations_and_pending_overseer_recs() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let conn = db.pool().get().unwrap();

    let cnt: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='recommendations'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(cnt, 1, "recommendations table missing");

    let has_col: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info('sessions') WHERE name='pending_overseer_recs'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(has_col, 1, "sessions.pending_overseer_recs missing");
}

#[test]
fn migration_0004_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("g.db");
    let _ = Db::open(&path).unwrap();
    // Reopen — should not re-apply.
    let db = Db::open(&path).unwrap();
    let conn = db.pool().get().unwrap();
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM schema_version WHERE version = 4",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 1);
}
