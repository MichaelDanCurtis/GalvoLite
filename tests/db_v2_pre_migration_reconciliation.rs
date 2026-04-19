use galvolite::db::Db;
use rusqlite::params;
use tempfile::TempDir;

/// Manually fabricate a v1-only DB, then trigger v2 migration and assert that
/// every non-ended session row was moved to 'error' with the marker note.
#[test]
fn pre_v2_sessions_are_reconciled_to_error() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("v1.db");

    // 1. Build a v1-only schema by hand (skip our migrate() runner).
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_initial.sql")).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (1, '2026-04-15T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, path, created_at)
             VALUES ('p1', 'p', '/tmp', '2026-04-15T00:00:00Z')",
            [],
        )
        .unwrap();
        // One non-ended session in v1 'working' state, one already-ended.
        conn.execute(
            "INSERT INTO sessions
                (id, project_id, role, worktree_path, state, task_summary, created_at)
             VALUES ('s_open','p1','implementation','/tmp','working','t','2026-04-15T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
                (id, project_id, role, worktree_path, state, task_summary, created_at, ended_at)
             VALUES ('s_done','p1','implementation','/tmp','complete','t',
                     '2026-04-15T00:00:00Z','2026-04-16T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    // 2. Open via Db::open — this should run migration 0002.
    let _db = Db::open(&db_path).unwrap();

    // 3. Assert reconciliation outcome.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (state_open, note_open): (String, Option<String>) = conn
        .query_row(
            "SELECT state, last_status_note FROM sessions WHERE id = ?1",
            params!["s_open"],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state_open, "error", "open v1 session should be reconciled");
    assert_eq!(note_open.as_deref(), Some("pre-migration; state unknown"));

    // The already-ended row is left alone (ended_at IS NOT NULL).
    let state_done: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            params!["s_done"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state_done, "complete", "ended row must not be touched");
}
