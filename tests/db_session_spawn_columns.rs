use galvolite::db::Db;
use tempfile::TempDir;

#[test]
fn migration_0003_adds_spawn_columns_and_parent_index() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let conn = db.pool().get().unwrap();

    // Columns exist.
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(sessions)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    for needed in ["waiting_on", "pending_rejoin", "terminal_status"] {
        assert!(cols.iter().any(|c| c == needed), "missing column {needed}: {cols:?}");
    }

    // Parent index exists.
    let idx_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_sessions_parent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx_count, 1);

    // schema_version row 3 present.
    let v: i64 = conn
        .query_row("SELECT version FROM schema_version WHERE version = 3", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 3);
}
