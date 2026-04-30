//! Fixture-based regression proof: even when the PTY emits text that LOOKS
//! like the legacy bracket-token signals (e.g. `[STATUS:WORKING]`), no
//! `signal.*` event lands in the events log because the signal parser is
//! deleted.
//!
//! The fixture (`tests/fixtures/claude_tui_sample.bin`) contains genuine
//! `[STATUS:WORKING]` and `[HANDOFF:READY path="/tmp/x"]` bracket tokens
//! wrapped in ANSI color escapes — exactly the shape the legacy parser used
//! to match. With Sub-plan 5 deleting that pipeline, the bytes flow into
//! the PTY scrollback ring only; no `signal.*` row should ever land in the
//! `events` table.

use galvolite::db::Db;

#[tokio::test]
async fn ansi_wrapped_status_lines_produce_zero_signal_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("t.sqlite")).unwrap();
    let proj = db
        .insert_project(galvolite::db::NewProject {
            name: "p".into(),
            // SessionManager::spawn_session uses project.path as the PTY's
            // cwd; the dir must exist and be writable for the
            // `.galvolite/`, `.claude/settings.json`, and `.mcp.json`
            // bootstrap files spawn_session writes there.
            path: dir.path().to_string_lossy().into(),
            remote_url: None,
        })
        .unwrap();
    let _s = db
        .insert_session(galvolite::db::NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: dir.path().to_string_lossy().into(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();

    // Spawn a session whose command emits the fixture bytes. `cat` of the
    // fixture file pipes raw bytes — including ANSI-wrapped bracket tokens
    // — through the PTY reader. With the signal pipeline deleted, the bytes
    // hit scrollback only.
    //
    // Use an absolute path because the spawned PTY's cwd is the project
    // directory (a tempdir), not the workspace root. CARGO_MANIFEST_DIR
    // resolves to the crate root at compile time and is CI-safe.
    let fixture_abs = format!(
        "{}/tests/fixtures/claude_tui_sample.bin",
        env!("CARGO_MANIFEST_DIR")
    );

    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::with_event_bus(
            db.clone(),
            dir.path().to_path_buf(),
            bus,
        ),
    );
    let session = mgr
        .spawn_session(
            proj.id,
            "echo".into(),
            "implementation".into(),
            "cat".into(),
            vec![fixture_abs],
        )
        .await
        .unwrap();

    // Give the reader a moment to drain the file.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // No event row whose `kind` starts with `signal.` exists. We scope the
    // query to this session_id so any unrelated rows from other tests in
    // the same DB cannot pollute the assertion (the DB here is a fresh
    // tempdir, but defense in depth is cheap).
    let conn = db.pool().get().unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind LIKE 'signal.%' AND session_id = ?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "expected zero signal.* events for session {}, got {count}",
        session.id
    );
}
