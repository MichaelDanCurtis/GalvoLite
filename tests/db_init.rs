use galvolite::db::Db;
use tempfile::TempDir;

#[test]
fn new_db_applies_schema() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");

    let db = Db::open(&db_path).expect("open db");

    let conn = db.pool().get().unwrap();
    let version: i64 = conn
        .query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("query schema_version");
    assert_eq!(version, 1);
}

#[test]
fn reopening_db_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");

    let _ = galvolite::db::Db::open(&db_path).unwrap();
    let _ = galvolite::db::Db::open(&db_path).unwrap();
    // no panic = pass
}
