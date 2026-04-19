use galvolite::db::Db;
use tempfile::TempDir;

#[test]
fn v2_creates_events_table() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let conn = db.pool().get().unwrap();

    let cnt: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='events'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cnt, 1, "events table missing after migration 0002");
}

#[test]
fn v2_creates_plan_steps_and_path_claims_tables() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let conn = db.pool().get().unwrap();

    for table in ["plan_steps", "path_claims"] {
        let cnt: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 1, "{table} missing after migration 0002");
    }
}

#[test]
fn v2_adds_new_session_columns() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let conn = db.pool().get().unwrap();

    let mut stmt = conn.prepare("PRAGMA table_info(sessions)").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    for required in [
        "api_token", "turn_count", "est_tokens", "last_status_note",
        "handoff_path", "plan_markdown", "seed_markdown", "last_event_ts",
    ] {
        assert!(
            cols.iter().any(|c| c == required),
            "sessions missing column {required}; have {cols:?}"
        );
    }
}

#[test]
fn v2_events_table_enforces_dedup_key_unique() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let conn = db.pool().get().unwrap();

    conn.execute(
        "INSERT INTO projects (id, name, path, created_at) VALUES ('p1','p','/tmp','2026-04-18T00:00:00Z')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO sessions (id, project_id, role, worktree_path, state, task_summary, created_at)
         VALUES ('s1','p1','implementation','/tmp','starting','t','2026-04-18T00:00:00Z')",
        [],
    ).unwrap();

    conn.execute(
        "INSERT INTO events (session_id, source, kind, payload_json, dedup_key)
         VALUES ('s1','system','test','{}','dup-1')",
        [],
    ).unwrap();

    let err = conn.execute(
        "INSERT INTO events (session_id, source, kind, payload_json, dedup_key)
         VALUES ('s1','system','test','{}','dup-1')",
        [],
    );
    assert!(err.is_err(), "second insert with same dedup_key should fail");
}
