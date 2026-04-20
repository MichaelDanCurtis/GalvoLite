# Foundations — Hooks + MCP Coordination Layer (Sub-plan 1/5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the substrate the rest of the hooks+MCP design rests on — append-only `events` log, atomic `state::apply_event()` funnel, schema delta (sessions additions + `events`/`plan_steps`/`path_claims` tables), 7-variant `SessionState` enum from spec §8.1, and route the existing `SignalPump` through the funnel so the legacy signal pipeline generates `events` rows for free.

**Architecture:** A new module `src/state.rs` owns the `SessionState` enum and a single `apply_event(db, session_id, input) → outcome` function. Every state transition — whether triggered by a parsed signal today, or by an MCP tool / hook handler in Sub-plans 2-3 — calls `apply_event` and never writes the `state` column directly. `apply_event` runs one SQLite transaction containing `(events row append, sessions row update)` so the event log is the authoritative "who did what when" record and reconstructable across restarts. Schema migration is gated by a version-aware migrate runner (multiple files in `migrations/`). No new endpoints, no UI in this sub-plan; the legacy stdout `signals.rs` parser stays in place but its outputs are mirrored into the new pipeline. Sub-plan 5 will delete the parser.

**Tech Stack:** Rust 1.75+, `rusqlite` 0.31 with `bundled` feature (SQLite 3.46+), `r2d2` connection pool, `tokio` (existing), `chrono` for timestamps, `serde` for event payload JSON. No new crates required.

---

## File Structure

**New files:**
- `migrations/0002_hooks_mcp_foundations.sql` — schema delta (events, plan_steps, path_claims, session column additions)
- `src/state.rs` — new `SessionState` enum, `EventInput` / `Source` / `EventOutcome` types, `apply_event()` funnel
- `tests/db_v2_schema.rs` — verifies migration 0002 creates the new tables/columns
- `tests/db_v2_pre_migration_reconciliation.rs` — verifies a pre-v2 session row is reconciled to `error` on upgrade
- `tests/state_apply_event.rs` — table-driven transitions, atomicity, dedup
- `tests/state_signal_bridge.rs` — verifies SignalPump-emitted events land in the `events` table

**Modified files:**
- `src/db.rs` — version-aware `migrate()`; `Db::current_schema_version()`; `Db::append_event()`; `SessionState` re-exported from `state` module; remove the old in-file `SessionState` enum body
- `src/lib.rs` — `pub mod state;`
- `src/pty.rs` — `SignalPump` calls `state::apply_event` after `record_signal` so the events log is populated by the legacy pipeline (existing `record_signal` write is retained until Sub-plan 5 deletes the signals table)
- `src/session_manager.rs` — every `db.update_session_state(...)` call replaced with `state::apply_event(...)` so the manager goes through the funnel; existing `db.set_session_role` likewise routed
- `tests/db_init.rs` — assert `schema_version` is now 2; add round-trip test for migration ordering

---

## Spec ↔ task crosswalk

| Spec section | Sub-plan 1 task |
|--------------|-----------------|
| §8.1 (7-state lifecycle)               | Task 3 |
| §8.2 (DB delta — sessions, events, plan_steps, path_claims) | Task 1 |
| §8.3 (atomicity — transaction wraps event+state)            | Task 4 |
| §8.6 (transition reference table)      | Task 4 |
| §8.7 (migration: pre-v2 sessions → error) | Task 1 (SQL) + Task 2 (test) |
| §9.4 first commit ("add new surfaces alongside old")        | Whole sub-plan; no deletions |
| §9.4 second commit ("delete signal pipeline")               | Sub-plan 5 — out of scope here |
| Atomicity invariant (every transition writes one event row first) | Tasks 4 + 5 + 6 |

Sub-plans 2 (hooks), 3 (MCP), 4 (dashboard), 5 (signal deletion) all depend on this sub-plan. Nothing in §5 (MCP catalog), §6 (hook surface), §7 (`.mcp.json` injection), §10 (dashboard), §11.2 (restart recovery) is in scope here.

---

## Branch / worktree

This plan should be executed on a dedicated branch:
```bash
git switch -c plan-foundations-hooks-mcp
```
Or in a worktree:
```bash
git worktree add ../galvolite-foundations -b plan-foundations-hooks-mcp main
```
All commits land on this branch. Merge to `main` after Sub-plan 1 passes its goal-backward review.

---

## Task 1: Add migration 0002 SQL + version-aware migrate runner

**Files:**
- Create: `migrations/0002_hooks_mcp_foundations.sql`
- Modify: `src/db.rs:12` (add a const for the new SQL)
- Modify: `src/db.rs:43-66` (`migrate()` becomes version-aware)
- Test: `tests/db_v2_schema.rs` (new)
- Modify: `tests/db_init.rs:19` (assert version 2, not 1)

- [ ] **Step 1: Write the failing test for migration 0002**

Create `tests/db_v2_schema.rs`:

```rust
use galvolite::db::Db;
use tempfile::TempDir;

#[test]
fn v2_creates_events_table() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let conn = db.pool().get().unwrap();

    // events table
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
        "api_token",
        "turn_count",
        "est_tokens",
        "last_status_note",
        "handoff_path",
        "plan_markdown",
        "seed_markdown",
        "last_event_ts",
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

    // need a session row to satisfy FK
    conn.execute(
        "INSERT INTO projects (id, name, path, created_at) VALUES ('p1','p','/tmp','2026-04-18T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, project_id, role, worktree_path, state, task_summary, created_at)
         VALUES ('s1','p1','implementation','/tmp','starting','t','2026-04-18T00:00:00Z')",
        [],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO events (session_id, source, kind, payload_json, dedup_key)
         VALUES ('s1','system','test','{}','dup-1')",
        [],
    )
    .unwrap();

    let err = conn.execute(
        "INSERT INTO events (session_id, source, kind, payload_json, dedup_key)
         VALUES ('s1','system','test','{}','dup-1')",
        [],
    );
    assert!(err.is_err(), "second insert with same dedup_key should fail");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test db_v2_schema
```

Expected: each test fails — likely panics on `Db::open` (no such file `0002_*.sql`) or asserts (table missing).

- [ ] **Step 3: Write `migrations/0002_hooks_mcp_foundations.sql`**

```sql
-- GalvoLite hooks+MCP coordination layer foundations (schema v0002)
-- See docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md §8.2.
--
-- Adds:
--  * sessions: api_token, turn_count, est_tokens, last_status_note,
--    handoff_path, plan_markdown, seed_markdown, last_event_ts
--  * events     — append-only event log (single source of truth for state changes)
--  * plan_steps — structured rows derived from MCP set_plan markdown
--  * path_claims — reserved; populated by Sub-plan 4 spawn/rejoin
--
-- Pre-v2 sessions are reconciled to 'error' per spec §8.7 because the new
-- 7-state SessionState enum is incompatible with the v1 lifecycle.
--
-- All columns are non-NULL with safe defaults so the ALTER cannot fail on
-- pre-existing rows. SQLite's ALTER TABLE only supports column add (not
-- modify), which is fine: the existing `state TEXT NOT NULL` column accepts
-- the new enum strings without schema change.

ALTER TABLE sessions ADD COLUMN api_token        TEXT;
ALTER TABLE sessions ADD COLUMN turn_count       INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN est_tokens       INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN last_status_note TEXT;
ALTER TABLE sessions ADD COLUMN handoff_path     TEXT;
ALTER TABLE sessions ADD COLUMN plan_markdown    TEXT;
ALTER TABLE sessions ADD COLUMN seed_markdown    TEXT;
ALTER TABLE sessions ADD COLUMN last_event_ts    TEXT;

CREATE TABLE IF NOT EXISTS events (
    id            INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    ts            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source        TEXT NOT NULL CHECK(source IN ('mcp','hook','operator','system')),
    kind          TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    result_json   TEXT,
    prev_state    TEXT,
    next_state    TEXT,
    dedup_key     TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS events_session_ts ON events(session_id, ts);
CREATE INDEX IF NOT EXISTS events_kind_ts    ON events(kind, ts);

CREATE TABLE IF NOT EXISTS plan_steps (
    id           INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    step_hash    TEXT NOT NULL,
    ordinal      INTEGER NOT NULL,
    title        TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending'
                 CHECK(status IN ('pending','in_progress','done','blocked')),
    note         TEXT,
    owner_id     TEXT REFERENCES sessions(id),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(session_id, step_hash)
);
CREATE INDEX IF NOT EXISTS plan_steps_session_ord ON plan_steps(session_id, ordinal);

CREATE TABLE IF NOT EXISTS path_claims (
    id         INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    glob       TEXT NOT NULL,
    claimed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(glob)
);

-- One-time reconciliation: every pre-existing session row carried a v1 state
-- ('spawning','working','attention_needed','handing_off','restoring','complete',
-- 'killed','error'). The new 7-state model is not a superset, so any row that
-- had not yet been ended is moved to 'error' with a marker note.
UPDATE sessions
SET    state            = 'error',
       last_status_note = 'pre-migration; state unknown'
WHERE  ended_at IS NULL;
```

- [ ] **Step 4: Modify `src/db.rs` to load migration 0002 by version**

Open `src/db.rs`. Replace the `SCHEMA_V1` constant (line 12) and the `migrate()` body (lines 43-66) with the version-aware runner.

Replace:

```rust
const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");
```

with:

```rust
const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");
const SCHEMA_V2: &str = include_str!("../migrations/0002_hooks_mcp_foundations.sql");

/// (version, sql_body). Order matters: applied lowest-to-highest. Each entry
/// is applied iff the version is not present in `schema_version`.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, SCHEMA_V2),
];
```

Replace the existing `fn migrate(&self) -> Result<()> { ... }` body with:

```rust
fn migrate(&self) -> Result<()> {
    let mut conn = self.pool.get()?;

    // Bootstrap: schema_version itself is created by 0001_initial.sql, so the
    // very first call needs to run that file unconditionally before we can
    // query for applied versions.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
             version    INTEGER NOT NULL PRIMARY KEY,
             applied_at TEXT NOT NULL
         );",
    )?;

    for (version, body) in MIGRATIONS {
        let already: Option<i64> = conn
            .query_row(
                "SELECT version FROM schema_version WHERE version = ?1",
                params![*version],
                |r| r.get(0),
            )
            .ok();
        if already.is_some() {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(body)?;
        tx.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
            params![*version, Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
    }
    Ok(())
}
```

- [ ] **Step 5: Run the migration tests**

```bash
cargo test --test db_v2_schema
```

Expected: all 4 tests pass.

- [ ] **Step 6: Update `tests/db_init.rs` to expect schema_version 2**

In `tests/db_init.rs:19`, change:

```rust
    assert_eq!(version, 1);
```

to:

```rust
    assert_eq!(version, 2);
```

Run:

```bash
cargo test --test db_init
```

Expected: both tests pass.

- [ ] **Step 7: Commit**

```bash
git add migrations/0002_hooks_mcp_foundations.sql src/db.rs tests/db_v2_schema.rs tests/db_init.rs
git commit -m "feat(db): schema v2 — events log, plan_steps, path_claims, session columns

Adds migration 0002 from hooks+MCP coordination layer spec §8.2. New tables
events / plan_steps / path_claims and 8 new sessions columns land alongside
v1; pre-v2 non-ended sessions reconciled to 'error' per spec §8.7. Migration
runner now version-aware so future deltas drop in as new files."
```

---

## Task 2: Pre-migration session reconciliation regression test

**Files:**
- Test: `tests/db_v2_pre_migration_reconciliation.rs` (new)

This task verifies the §8.7 reconciliation rule survives reopening a v1 database. Critical because losing this rule would silently keep stale `working`/`spawning` rows alive after upgrade.

- [ ] **Step 1: Write the failing test**

Create `tests/db_v2_pre_migration_reconciliation.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it passes (Task 1 already wrote the SQL that makes it pass)**

```bash
cargo test --test db_v2_pre_migration_reconciliation
```

Expected: PASS — the SQL `UPDATE sessions ... WHERE ended_at IS NULL` from Task 1 already implements it. This test pins the behavior so Sub-plan 5 cannot accidentally regress it during signal-pipeline cleanup.

- [ ] **Step 3: Commit**

```bash
git add tests/db_v2_pre_migration_reconciliation.rs
git commit -m "test(db): pin §8.7 pre-migration session reconciliation rule"
```

---

## Task 3: New `state::SessionState` enum (7 variants) + module skeleton

**Files:**
- Create: `src/state.rs`
- Modify: `src/lib.rs:8` (add `pub mod state;`)
- Test: `tests/state_session_state.rs` (new)

This task introduces the new enum next to the old one. Task 6 will swap callers over and delete the old `db::SessionState` body.

- [ ] **Step 1: Write the failing test**

Create `tests/state_session_state.rs`:

```rust
use galvolite::state::SessionState;
use std::str::FromStr;

#[test]
fn all_seven_variants_round_trip() {
    let cases = [
        (SessionState::Starting,       "starting"),
        (SessionState::Idle,           "idle"),
        (SessionState::Working,        "working"),
        (SessionState::AwaitingInput,  "awaiting_input"),
        (SessionState::NeedsAttention, "needs_attention"),
        (SessionState::Error,          "error"),
        (SessionState::Ended,          "ended"),
    ];
    for (state, s) in cases {
        assert_eq!(state.to_string(), s);
        assert_eq!(SessionState::from_str(s).unwrap(), state);
    }
}

#[test]
fn unknown_string_is_error() {
    assert!(SessionState::from_str("complete").is_err(),
        "v1 'complete' must not silently parse as a v2 state");
    assert!(SessionState::from_str("").is_err());
    assert!(SessionState::from_str("WORKING").is_err(),
        "casing matters — DB strings are lowercase canonical");
}

#[test]
fn is_terminal_returns_true_for_error_and_ended_only() {
    assert!(SessionState::Error.is_terminal());
    assert!(SessionState::Ended.is_terminal());
    for s in [
        SessionState::Starting,
        SessionState::Idle,
        SessionState::Working,
        SessionState::AwaitingInput,
        SessionState::NeedsAttention,
    ] {
        assert!(!s.is_terminal(), "{s} must not be terminal");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test state_session_state
```

Expected: compile failure — module `galvolite::state` does not exist.

- [ ] **Step 3: Create `src/state.rs` with `SessionState` enum**

```rust
//! State funnel for the hooks+MCP coordination layer.
//!
//! See `docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md`
//! §8 for the design. Every session-state transition in GalvoLite goes through
//! [`apply_event`] (added in Task 5); this module owns the canonical
//! [`SessionState`] enum and the event input/outcome types that funnel
//! describes.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Session lifecycle states from spec §8.1. The DB stores the snake_case
/// rendering returned by [`SessionState::fmt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Row inserted, child PTY not yet observed alive.
    Starting,
    /// PTY alive, no work in flight (default between turns).
    Idle,
    /// Model is mid-turn.
    Working,
    /// Model paused on an operator question (Finding 3 fix).
    AwaitingInput,
    /// Model asked for operator attention (`request_attention` /
    /// `status_report(blocked)`).
    NeedsAttention,
    /// PTY died unexpectedly, or row is left over from a prior schema.
    Error,
    /// Session ended cleanly (`SessionEnd` hook or `request_end` tool).
    Ended,
}

impl SessionState {
    /// `Error` and `Ended` are terminal — no transition out.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Error | Self::Ended)
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Starting       => "starting",
            Self::Idle           => "idle",
            Self::Working        => "working",
            Self::AwaitingInput  => "awaiting_input",
            Self::NeedsAttention => "needs_attention",
            Self::Error          => "error",
            Self::Ended          => "ended",
        })
    }
}

impl FromStr for SessionState {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "starting"        => Self::Starting,
            "idle"            => Self::Idle,
            "working"         => Self::Working,
            "awaiting_input"  => Self::AwaitingInput,
            "needs_attention" => Self::NeedsAttention,
            "error"           => Self::Error,
            "ended"           => Self::Ended,
            other => return Err(Error::Config(format!("unknown session state: {other}"))),
        })
    }
}
```

- [ ] **Step 4: Register the module in `src/lib.rs`**

Insert `pub mod state;` between the existing `pub mod signals;` and the `pub use error::...` re-export.

```rust
//! GalvoLite ritual engine — library crate.
//!
//! See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec
//! and `docs/superpowers/plans/2026-04-15-plan-1a-ritual-engine.md` for this plan.

pub mod api;
pub mod claude_md;
pub mod cli;
pub mod db;
pub mod error;
pub mod handoff;
pub mod pty;
pub mod ritual;
pub mod session_manager;
pub mod session_registry;
pub mod signals;
pub mod state;

pub use error::{Error, Result};
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test --test state_session_state
```

Expected: all 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/lib.rs tests/state_session_state.rs
git commit -m "feat(state): add 7-variant SessionState enum from spec §8.1"
```

---

## Task 4: `EventInput`, `Source`, `EventOutcome` types

**Files:**
- Modify: `src/state.rs` (add types below the enum from Task 3)
- Test: extend `tests/state_session_state.rs` with serialization round-trip

These are the inputs and outputs of the `apply_event` funnel. Defining them in their own task lets Task 5's implementation focus on transition logic.

- [ ] **Step 1: Write the failing test**

Append to `tests/state_session_state.rs`:

```rust
use galvolite::state::{EventInput, EventOutcome, Source};

#[test]
fn source_round_trip() {
    use std::str::FromStr;
    for (src, s) in [
        (Source::Mcp,      "mcp"),
        (Source::Hook,     "hook"),
        (Source::Operator, "operator"),
        (Source::System,   "system"),
    ] {
        assert_eq!(src.to_string(), s);
        assert_eq!(Source::from_str(s).unwrap(), src);
    }
}

#[test]
fn event_input_payload_is_json_value() {
    let e = EventInput {
        source: Source::System,
        kind: "signal.STATUS:WORKING".into(),
        payload: serde_json::json!({ "raw": "[STATUS:WORKING]" }),
        dedup_key: Some("sig-1".into()),
    };
    assert_eq!(e.kind, "signal.STATUS:WORKING");
    assert_eq!(e.payload["raw"], "[STATUS:WORKING]");
}

#[test]
fn event_outcome_carries_event_id_and_states() {
    let o = EventOutcome {
        event_id: 42,
        prev_state: Some(SessionState::Working),
        next_state: SessionState::Idle,
        deduped: false,
    };
    assert_eq!(o.event_id, 42);
    assert_eq!(o.prev_state, Some(SessionState::Working));
    assert_eq!(o.next_state, SessionState::Idle);
    assert!(!o.deduped);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test state_session_state
```

Expected: compile failure — `EventInput`, `EventOutcome`, `Source` not found.

- [ ] **Step 3: Append types to `src/state.rs`**

Add below the `impl FromStr for SessionState` block:

```rust
/// Origin of a state-changing event. Stored verbatim in `events.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Triggered by a model-chosen MCP tool call.
    Mcp,
    /// Fired by Claude Code via `/hooks/ingest`.
    Hook,
    /// Triggered by a dashboard / CLI action.
    Operator,
    /// Synthesized by GalvoLite itself (orphan reconciler, signal bridge,
    /// startup recovery markers).
    System,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mcp      => "mcp",
            Self::Hook     => "hook",
            Self::Operator => "operator",
            Self::System   => "system",
        })
    }
}

impl FromStr for Source {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "mcp"      => Self::Mcp,
            "hook"     => Self::Hook,
            "operator" => Self::Operator,
            "system"   => Self::System,
            other => return Err(Error::Config(format!("unknown event source: {other}"))),
        })
    }
}

/// Input to [`apply_event`]. The funnel writes one `events` row per call
/// (skipped if `dedup_key` already exists).
#[derive(Debug, Clone)]
pub struct EventInput {
    pub source: Source,
    /// Free-form event kind, e.g. `"hook.SessionStart"`, `"mcp.set_role"`,
    /// `"signal.STATUS:WORKING"`, `"system.recovered"`. The convention is
    /// `"<source>.<name>"` but the funnel does not enforce it — kind is
    /// the dimension `read_events`/dashboard filters on, so use whatever
    /// the consumer expects.
    pub kind: String,
    pub payload: serde_json::Value,
    /// Optional UNIQUE key. If present and a prior row carries the same
    /// value, the funnel returns `deduped = true` without writing.
    pub dedup_key: Option<String>,
}

/// Result of a successful `apply_event` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOutcome {
    /// Rowid of the new `events` row, or the existing row if `deduped`.
    pub event_id: i64,
    /// State the session was in immediately before the event landed
    /// (`None` if this is the first event for a brand-new row).
    pub prev_state: Option<SessionState>,
    /// State the session is in after the event was applied. Equals
    /// `prev_state` for events that don't change state (logging-only).
    pub next_state: SessionState,
    /// `true` if the event was a no-op because `dedup_key` matched a
    /// prior row.
    pub deduped: bool,
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --test state_session_state
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/state.rs tests/state_session_state.rs
git commit -m "feat(state): add EventInput, Source, EventOutcome types for funnel"
```

---

## Task 5: `Db::append_event` low-level helper + `state::apply_event` funnel

**Files:**
- Modify: `src/db.rs` (add `append_event` method; new `EventRow` struct)
- Modify: `src/state.rs` (add `apply_event` function + transition table)
- Test: `tests/state_apply_event.rs` (new)

`apply_event` is the heart of the spec. It runs one SQLite transaction containing event-append + session-update; nothing in the codebase mutates `sessions.state` outside this function.

- [ ] **Step 1: Write the failing test for `apply_event` happy paths**

Create `tests/state_apply_event.rs`:

```rust
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::state::{apply_event, EventInput, SessionState, Source};
use serde_json::json;
use tempfile::TempDir;

fn open_db_with_session() -> (Db, TempDir, uuid::Uuid) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let s = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: tmp.path().to_string_lossy().into_owned(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();
    (db, tmp, s.id)
}

fn ev(kind: &str) -> EventInput {
    EventInput {
        source: Source::System,
        kind: kind.into(),
        payload: json!({}),
        dedup_key: None,
    }
}

#[test]
fn signal_status_working_transitions_to_working() {
    let (db, _t, sid) = open_db_with_session();
    let out = apply_event(&db, sid, ev("signal.STATUS:WORKING")).unwrap();
    assert_eq!(out.next_state, SessionState::Working);
    assert!(!out.deduped);

    let conn = db.pool().get().unwrap();
    let state: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "working", "sessions.state row should be updated");
}

#[test]
fn signal_status_complete_transitions_working_to_idle() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("signal.STATUS:WORKING")).unwrap();
    let out = apply_event(&db, sid, ev("signal.STATUS:COMPLETE")).unwrap();
    assert_eq!(out.prev_state, Some(SessionState::Working));
    assert_eq!(out.next_state, SessionState::Idle);
}

#[test]
fn signal_attention_needed_transitions_to_needs_attention() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("signal.STATUS:WORKING")).unwrap();
    let out = apply_event(&db, sid, ev("signal.ATTENTION:NEEDED")).unwrap();
    assert_eq!(out.next_state, SessionState::NeedsAttention);
}

#[test]
fn logging_only_kinds_do_not_change_state() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("signal.STATUS:WORKING")).unwrap();
    let out = apply_event(&db, sid, ev("signal.HANDOFF:READY")).unwrap();
    assert_eq!(out.prev_state, Some(SessionState::Working));
    assert_eq!(out.next_state, SessionState::Working,
        "HANDOFF:READY is logged but doesn't change state");
}

#[test]
fn dedup_key_collision_returns_deduped_outcome_no_second_row() {
    let (db, _t, sid) = open_db_with_session();
    let mut e = ev("signal.STATUS:WORKING");
    e.dedup_key = Some("sig-1".into());

    let first = apply_event(&db, sid, e.clone()).unwrap();
    assert!(!first.deduped);

    let second = apply_event(&db, sid, e).unwrap();
    assert!(second.deduped);
    assert_eq!(second.event_id, first.event_id, "deduped call returns prior event_id");

    let conn = db.pool().get().unwrap();
    let cnt: i64 = conn
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1",
            [sid.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cnt, 1, "deduped event must not write a second row");
}

#[test]
fn event_row_records_prev_and_next_state() {
    let (db, _t, sid) = open_db_with_session();
    apply_event(&db, sid, ev("signal.STATUS:WORKING")).unwrap();
    apply_event(&db, sid, ev("signal.STATUS:COMPLETE")).unwrap();

    let conn = db.pool().get().unwrap();
    let mut stmt = conn
        .prepare("SELECT prev_state, next_state FROM events WHERE session_id = ?1 ORDER BY id ASC")
        .unwrap();
    let rows: Vec<(Option<String>, Option<String>)> = stmt
        .query_map([sid.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(rows.len(), 2);
    // first event: starting → working
    assert_eq!(rows[0].0.as_deref(), Some("starting"));
    assert_eq!(rows[0].1.as_deref(), Some("working"));
    // second event: working → idle
    assert_eq!(rows[1].0.as_deref(), Some("working"));
    assert_eq!(rows[1].1.as_deref(), Some("idle"));
}

#[test]
fn terminal_state_rejects_further_transitions() {
    let (db, _t, sid) = open_db_with_session();
    // Force ended via the explicit lifecycle event.
    apply_event(&db, sid, ev("system.session_ended")).unwrap();
    let err = apply_event(&db, sid, ev("signal.STATUS:WORKING")).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("terminal"), "expected terminal-state rejection, got: {msg}");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test state_apply_event
```

Expected: compile failure — `apply_event` not exported.

- [ ] **Step 3: Add `Db::append_event` to `src/db.rs`**

Add this method inside the `impl Db` block (place it after `record_signal`):

```rust
    /// Append one row to the `events` log inside a caller-supplied transaction.
    /// Returns `(rowid, deduped)` where `deduped = true` if `dedup_key` was
    /// already present (no row was written).
    ///
    /// This is the low-level helper. Callers must use [`crate::state::apply_event`]
    /// for state-changing events so the events row and the sessions update land
    /// inside the same transaction.
    pub fn append_event_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
        source: &str,
        kind: &str,
        payload_json: &str,
        prev_state: Option<&str>,
        next_state: Option<&str>,
        dedup_key: Option<&str>,
    ) -> Result<(i64, bool)> {
        if let Some(key) = dedup_key {
            use rusqlite::OptionalExtension;
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM events WHERE dedup_key = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok((id, true));
            }
        }
        tx.execute(
            "INSERT INTO events
                (session_id, source, kind, payload_json, prev_state, next_state, dedup_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id.to_string(),
                source,
                kind,
                payload_json,
                prev_state,
                next_state,
                dedup_key,
            ],
        )?;
        Ok((tx.last_insert_rowid(), false))
    }
```

- [ ] **Step 4: Implement `apply_event` and the transition table in `src/state.rs`**

Append to `src/state.rs`:

```rust
use crate::db::Db;
use uuid::Uuid;

/// Single funnel for every state-changing event. Runs in one SQLite
/// transaction containing `(events row append, sessions row update,
/// last_event_ts bump)` so the events log is the authoritative pre-image
/// of every state change.
///
/// Returns [`EventOutcome`] describing prev/next state and whether the
/// call was deduped. Callers must not write `sessions.state` directly;
/// per spec §8.3 this is the only legal mutation path.
pub fn apply_event(db: &Db, session_id: Uuid, input: EventInput) -> Result<EventOutcome> {
    let mut conn = db.pool().get()?;
    let tx = conn.transaction()?;

    // 1. Read current state (for transition computation).
    let current_str: String = tx
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Error::SessionNotFound(session_id),
            other => Error::Db(other),
        })?;
    let current = SessionState::from_str(&current_str)?;

    // 2. Reject transitions out of terminal states.
    if current.is_terminal() {
        return Err(Error::Config(format!(
            "session {session_id} is in terminal state {current}; refusing event {kind:?}",
            kind = input.kind,
        )));
    }

    // 3. Compute next state from (current, kind).
    let next = transition(current, &input.kind);

    // 4. Append events row (or short-circuit on dedup hit).
    let payload_str = serde_json::to_string(&input.payload).map_err(|e| {
        Error::Config(format!("event payload serialize failed: {e}"))
    })?;
    let (event_id, deduped) = Db::append_event_in_tx(
        &tx,
        session_id,
        &input.source.to_string(),
        &input.kind,
        &payload_str,
        Some(&current.to_string()),
        Some(&next.to_string()),
        input.dedup_key.as_deref(),
    )?;
    if deduped {
        // Don't re-apply the state mutation; the original call already did.
        tx.commit()?;
        return Ok(EventOutcome {
            event_id,
            prev_state: Some(current),
            next_state: current,
            deduped: true,
        });
    }

    // 5. Update sessions row state + last_event_ts.
    tx.execute(
        "UPDATE sessions
         SET    state         = ?1,
                last_event_ts = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE  id = ?2",
        crate::params_for_state_update(&next.to_string(), &session_id.to_string()),
    )?;

    tx.commit()?;
    Ok(EventOutcome {
        event_id,
        prev_state: Some(current),
        next_state: next,
        deduped: false,
    })
}

/// Transition table from spec §8.6 plus signal-bridge rows. Anything not
/// matched here is treated as logging-only (state unchanged).
fn transition(current: SessionState, kind: &str) -> SessionState {
    use SessionState::*;
    match (current, kind) {
        // Lifecycle (used by SessionManager via Task 6 + Sub-plan 2 hooks).
        (Starting, "system.session_started") | (Starting, "hook.SessionStart") => Idle,
        (_,        "system.session_ended")   | (_,        "hook.SessionEnd")   => Ended,
        (_,        "system.session_orphaned")                                  => Error,

        // Signal bridge (System events emitted by SignalPump in Task 7).
        (Idle,           "signal.STATUS:WORKING")    => Working,
        (Working,        "signal.STATUS:WORKING")    => Working,
        (NeedsAttention, "signal.STATUS:WORKING")    => Working,
        (AwaitingInput,  "signal.STATUS:WORKING")    => Working,
        (Starting,       "signal.STATUS:WORKING")    => Working,

        (Working,        "signal.STATUS:COMPLETE")   => Idle,
        (NeedsAttention, "signal.STATUS:COMPLETE")   => Idle,
        (AwaitingInput,  "signal.STATUS:COMPLETE")   => Idle,

        (Working,        "signal.ATTENTION:NEEDED")  => NeedsAttention,
        (Idle,           "signal.ATTENTION:NEEDED")  => NeedsAttention,
        (AwaitingInput,  "signal.ATTENTION:NEEDED")  => NeedsAttention,

        // Everything else is logged but does not change state. Includes:
        //  - signal.HANDOFF:READY / HANDOFF:BLOCKED / PLAN:READY / SEED:NEW /
        //    ROLE:SET / ATTENTION:CONTEXT — model-side annotations, no
        //    lifecycle effect.
        //  - mcp.* tool calls — handled in Sub-plan 3.
        //  - hook.PreCompact / hook.UserPromptSubmit / hook.Stop /
        //    hook.PostToolUse / hook.Notification — Sub-plan 2 will add their
        //    own arms; until then they no-op.
        //  - operator.* — Sub-plan 4 will add.
        _ => current,
    }
}
```

- [ ] **Step 5: Add the `params_for_state_update` shim**

The transition update binds `state` and `id` as named parameters; rusqlite's `params!` macro is the idiomatic helper but lives in `db.rs` already. Add a tiny re-export at the bottom of `src/db.rs`:

```rust
/// Internal helper exposed to `state::apply_event` so it can build
/// `params!` with the right rusqlite type without depending on the macro.
pub(crate) fn params_for_state_update<'a>(
    state: &'a str,
    id: &'a str,
) -> [&'a dyn rusqlite::ToSql; 2] {
    [state, id]
}
```

And in `src/state.rs`, replace the `crate::params_for_state_update(...)` call site with:

```rust
        rusqlite::params![next.to_string(), session_id.to_string()],
```

(Delete the shim — using `rusqlite::params!` directly is cleaner. The placeholder above was scaffolding.)

- [ ] **Step 6: Run tests**

```bash
cargo test --test state_apply_event
```

Expected: all 7 tests pass.

- [ ] **Step 7: Run the full suite to make sure nothing else broke**

```bash
cargo test
```

Expected: all existing tests still pass (we have not touched `db::SessionState` or any caller yet).

- [ ] **Step 8: Commit**

```bash
git add src/db.rs src/state.rs tests/state_apply_event.rs
git commit -m "feat(state): apply_event funnel — atomic event-append + state update

Implements spec §8.3 atomicity invariant and §8.6 transition table. Every
state-changing call (signal bridge today, hooks/MCP in Sub-plans 2-3) goes
through state::apply_event; the events row and sessions.state update land
in one SQLite transaction. Dedup_key short-circuits replays."
```

---

## Task 6: Replace `db::SessionState` body with re-export from `state`

**Files:**
- Modify: `src/db.rs` (delete the in-file `SessionState` enum + Display + FromStr; re-export from `state`; map removed `Spawning`/`AttentionNeeded`/`HandingOff`/`Restoring`/`Complete`/`Killed` callsites)
- Modify: `src/session_manager.rs` (every `SessionState::Spawning|Killed|Complete|...` reference → new variant)
- Modify: `src/pty.rs` (no SessionState references — verify)
- Modify: `src/api.rs` (verify no broken matches on removed variants)
- Modify: any tests that assert on removed states

This is the largest single mechanical edit in the sub-plan. The new enum is a strict subset of the spec §8.1 lifecycle; v1 names map as follows:

| v1 SessionState | v2 SessionState |
|-----------------|-----------------|
| Spawning        | Starting        |
| Working         | Working (unchanged) |
| AttentionNeeded | NeedsAttention  |
| HandingOff      | Working (no separate state in v2; handoff is an event, not a state) |
| Restoring       | Starting (re-attach is just a fresh start in v2) |
| Complete        | Ended           |
| Killed          | Ended           |
| Error           | Error (unchanged) |

- [ ] **Step 1: Replace `SessionState` block in `src/db.rs`**

Delete `src/db.rs:111-155` (the entire `pub enum SessionState`, `impl Display`, `impl FromStr` block). Replace with one line just after the `use` cluster:

```rust
pub use crate::state::SessionState;
```

- [ ] **Step 2: Update `SessionManager::new` startup reconciler**

In `src/session_manager.rs:42-46` the orphan reconciler logs but uses `SessionState::Error`. Verify the import still resolves through the re-export.

In `src/db.rs:438-445` (`reconcile_orphaned_sessions`), the body calls `SessionState::Error.to_string()` — still valid, no change needed.

- [ ] **Step 3: Update `SessionManager::spawn_session` literals**

In `src/session_manager.rs`, replace every removed-variant reference:

- Line 87, 94, 101: `SessionState::Error` — unchanged (still exists).
- Line 120, 132, 145, 151, 244: `SessionState::Working` / `SessionState::Error` — unchanged.
- Line 223: `SessionState::Complete` → **`SessionState::Ended`**
- Line 337: `SessionState::Killed` → **`SessionState::Ended`**

For Step 3 specifically apply these two replacements with `Edit`:

```rust
// session_manager.rs:223
            let _ = mgr.db.end_session_if_unended(session_id, SessionState::Complete);
```
becomes
```rust
            let _ = mgr.db.end_session_if_unended(session_id, SessionState::Ended);
```

```rust
// session_manager.rs:337
        let killed = self.db.end_session_if_unended(id, SessionState::Killed)?;
```
becomes
```rust
        let killed = self.db.end_session_if_unended(id, SessionState::Ended)?;
```

- [ ] **Step 4: Update `Db::insert_session` initial state**

In `src/db.rs:303`, `let state = SessionState::Spawning;` → `let state = SessionState::Starting;`.

- [ ] **Step 5: Sweep tests for removed variants**

```bash
grep -rn "SessionState::\(Spawning\|AttentionNeeded\|HandingOff\|Restoring\|Complete\|Killed\)" tests src
```

Apply the v1→v2 mapping table above to each hit.

For each test file that breaks, the substitution is mechanical:
- `Spawning` → `Starting`
- `AttentionNeeded` → `NeedsAttention`
- `HandingOff` → `Working`
- `Restoring` → `Starting`
- `Complete` → `Ended`
- `Killed` → `Ended`

For tests that specifically asserted on the difference between Complete and Killed (e.g. checking that user-kill writes `Killed` not `Complete`), update them to assert both paths land in `Ended` and rely on the events log to distinguish reasons. The events log is the new source of "why did this end" information — that is the spec's explicit design.

- [ ] **Step 6: Run the full suite**

```bash
cargo test
```

Expected: every test passes. If a test compiles but fails, re-read its assertion against the v1→v2 mapping and update.

- [ ] **Step 7: Commit**

```bash
git add src/db.rs src/session_manager.rs tests/
git commit -m "refactor: db::SessionState now re-exports state::SessionState (7 variants)

Maps v1 lifecycle (Spawning/AttentionNeeded/HandingOff/Restoring/Complete/
Killed/Error) onto v2 (Starting/Idle/Working/AwaitingInput/NeedsAttention/
Error/Ended) per spec §8.1. Distinctions previously encoded as separate states
(Killed vs Complete, HandingOff as state) are now distinguishable via the
events log instead. No state-mutation callsites changed yet — Task 7 routes
SessionManager through apply_event."
```

---

## Task 7: Route `SessionManager` state writes through `apply_event`

**Files:**
- Modify: `src/session_manager.rs` (each `db.update_session_state(...)` and `db.end_session_if_unended(...)` call swapped for `state::apply_event` with a synthetic system event)
- Test: `tests/session_manager_apply_event.rs` (new)

Goal: every state mutation in SessionManager now produces an events row first, then updates `sessions.state`, in one transaction. After this task, the only callers of `db.update_session_state` are inside `state::apply_event` itself (and `db.end_session_if_unended`, which we leave alone for the kill-path conditional-write race protection it already provides — but we wrap it).

- [ ] **Step 1: Write the failing test**

Create `tests/session_manager_apply_event.rs`:

```rust
use galvolite::db::{Db, NewProject};
use galvolite::session_manager::SessionManager;
use galvolite::state::SessionState;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn spawn_then_kill_writes_lifecycle_events() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let mgr = Arc::new(SessionManager::new(db.clone(), tmp.path().to_path_buf()));
    let proj = mgr
        .add_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn against the project's mock-claude binary fixture.
    let mock = std::env::current_dir()
        .unwrap()
        .join("target/debug/mock-claude");
    if !mock.exists() {
        eprintln!("skipping: mock-claude binary not built; run `cargo build -p mock-claude` first");
        return;
    }
    let session = mgr
        .spawn_session(
            proj.id,
            "task".into(),
            "implementation".into(),
            mock.to_string_lossy().into_owned(),
            vec![],
        )
        .await
        .unwrap();

    mgr.kill_session(session.id).await.unwrap();

    // Assert events log contains a session_started and session_ended row.
    let conn = db.pool().get().unwrap();
    let mut stmt = conn
        .prepare("SELECT kind, next_state FROM events WHERE session_id = ?1 ORDER BY id ASC")
        .unwrap();
    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([session.id.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(
        rows.iter().any(|(k, _)| k == "system.session_started"),
        "expected system.session_started in events; have {rows:?}"
    );
    let last_ended = rows.iter().rfind(|(k, _)| k == "system.session_ended");
    assert!(last_ended.is_some(), "expected system.session_ended; have {rows:?}");
    assert_eq!(
        last_ended.unwrap().1.as_deref(),
        Some("ended"),
        "ended event should set next_state to 'ended'"
    );

    // Final sessions.state row matches.
    let final_state: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(SessionState::from_str(&final_state).unwrap(), SessionState::Ended);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test session_manager_apply_event
```

Expected: assertion failure — no `system.session_started` / `system.session_ended` rows because SessionManager bypasses the funnel.

- [ ] **Step 3: Replace `db.update_session_state(.., Working)` in spawn_session**

In `src/session_manager.rs:143-153`, replace:

```rust
        if let Err(e) = self
            .db
            .update_session_state(session.id, SessionState::Working)
        {
```

with:

```rust
        let started = crate::state::apply_event(
            &self.db,
            session.id,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "system.session_started".into(),
                payload: serde_json::json!({ "role": session.role.clone() }),
                dedup_key: Some(format!("session-started:{}", session.id)),
            },
        );
        if let Err(e) = started {
```

(The rest of the rollback `if let Err(e) = ...` block stays unchanged.)

Note: spec §8.6 maps `Starting → Idle` on `system.session_started`, but our SessionManager today expects the session to be `Working` immediately after spawn because the inject-prompt step runs next. Add a transition arm in `state::transition` that recognizes the immediate-write-to-prompt pattern:

In `src/state.rs::transition`, change the `(Starting, "system.session_started")` arm to:

```rust
        // Per spec §8.6, SessionStart → Idle. SessionManager spawn_session,
        // however, immediately writes the inject prompt right after spawn —
        // which the future hook surface (Sub-plan 2) will model as
        // operator.user_prompt_submit pushing Idle → Working. Until that
        // surface lands, recognize the synthetic kind so spawn lands the
        // session in Working in one event.
        (Starting, "system.session_started") | (Starting, "hook.SessionStart") => Working,
```

(This is a deliberate temporary deviation from the spec's clean §8.6 row; called out so Sub-plan 2 can normalize it once the real `UserPromptSubmit` hook is on the wire.)

- [ ] **Step 4: Replace the bg-task `end_session_if_unended(.., Complete)` call**

In `src/session_manager.rs:223`, replace:

```rust
            let _ = mgr.db.end_session_if_unended(session_id, SessionState::Ended);
```

with:

```rust
            let _ = crate::state::apply_event(
                &mgr.db,
                session_id,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind: "system.session_ended".into(),
                    payload: serde_json::json!({ "reason": "pty_exit" }),
                    dedup_key: Some(format!("session-ended:{session_id}")),
                },
            );
```

The `db.end_session_if_unended` helper still gets called from kill_session; we keep its conditional-write semantics. We need it to also stamp `ended_at` — extend `apply_event` to do that for the lifecycle kinds. Add to `src/state.rs::apply_event`, immediately before `tx.commit()`:

```rust
    if matches!(next, SessionState::Ended | SessionState::Error) {
        tx.execute(
            "UPDATE sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id = ?1 AND ended_at IS NULL",
            [session_id.to_string()],
        )?;
    }
```

- [ ] **Step 5: Replace the kill_session `end_session_if_unended(.., Ended)` call**

In `src/session_manager.rs:337`, replace:

```rust
        let killed = self.db.end_session_if_unended(id, SessionState::Ended)?;
        if !was_running && !killed {
```

with:

```rust
        let killed = crate::state::apply_event(
            &self.db,
            id,
            crate::state::EventInput {
                source: crate::state::Source::Operator,
                kind: "system.session_ended".into(),
                payload: serde_json::json!({ "reason": "operator_kill" }),
                dedup_key: Some(format!("session-ended:{id}")),
            },
        );
        let killed_ok = killed.is_ok();
        if !was_running && !killed_ok {
```

(Adjust the `if` condition; `killed_ok` reads more naturally than reusing `killed: bool`.)

- [ ] **Step 6: Run the full test suite**

```bash
cargo test
```

Expected: all tests pass, including the new `tests/session_manager_apply_event.rs`. If `tests/db_v2_pre_migration_reconciliation.rs` or others break because of the new `ended_at` write inside `apply_event`, double-check they weren't relying on a state being settable without `ended_at` — they should not be, but if so adjust the test fixtures.

- [ ] **Step 7: Commit**

```bash
git add src/state.rs src/session_manager.rs tests/session_manager_apply_event.rs
git commit -m "refactor(session_manager): route lifecycle through state::apply_event

Spawn writes system.session_started; kill / pty-exit write system.session_ended.
Both go through the funnel so events log + sessions.state move atomically.
ended_at is stamped inside apply_event for terminal next_states."
```

---

## Task 8: Bridge `SignalPump` parsed signals into `apply_event`

**Files:**
- Modify: `src/pty.rs:339-379` (`SignalPump::start` body)
- Test: `tests/state_signal_bridge.rs` (new)

`SignalPump` already calls `db.record_signal` (legacy `signals` table) and `db.set_session_role` for `RoleSet`. Now it additionally calls `state::apply_event` per parsed signal so the new `events` table is populated by the legacy pipeline. Both paths run side-by-side until Sub-plan 5 deletes the legacy one.

`db.set_session_role` is a role write, not a state write — it stays as-is in this sub-plan. Sub-plan 3's `mcp.set_role` tool will become the new write path.

- [ ] **Step 1: Write the failing test**

Create `tests/state_signal_bridge.rs`:

```rust
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use std::sync::{Arc, Mutex as StdMutex};
use tempfile::TempDir;

/// Spawn a PTY that prints `[STATUS:WORKING]` and `[STATUS:COMPLETE]`,
/// run them through SignalPump, then assert both events land in the new
/// events table with the correct kind prefix.
#[tokio::test]
async fn signal_pump_emits_events_through_apply_event() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: tmp.path().to_string_lossy().into_owned(),
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();

    // Spawn /bin/sh that prints two signal lines and exits.
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: tmp.path().to_path_buf(),
        command: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "echo '[STATUS:WORKING]'; sleep 0.05; echo '[STATUS:COMPLETE]'".into(),
        ],
        env: vec![],
    })
    .unwrap();

    let cycle_slot: galvolite::pty::CycleSlot = Arc::new(StdMutex::new(None));
    let mut pump = SignalPump::start(pty, db.clone(), session.id, cycle_slot);

    // Drain signals so the pump background task processes both lines.
    while let Some(_sig) =
        tokio::time::timeout(std::time::Duration::from_millis(500), pump.next_signal())
            .await
            .ok()
            .flatten()
    {}

    let conn = db.pool().get().unwrap();
    let mut stmt = conn
        .prepare("SELECT kind FROM events WHERE session_id = ?1 ORDER BY id ASC")
        .unwrap();
    let kinds: Vec<String> = stmt
        .query_map([session.id.to_string()], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert!(
        kinds.iter().any(|k| k == "signal.STATUS:WORKING"),
        "expected signal.STATUS:WORKING in events; got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| k == "signal.STATUS:COMPLETE"),
        "expected signal.STATUS:COMPLETE in events; got {kinds:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test state_signal_bridge
```

Expected: assertion failure — no `signal.*` rows because SignalPump doesn't call `apply_event` yet.

- [ ] **Step 3: Modify `src/pty.rs::SignalPump::start`**

Locate the inner block in `src/pty.rs:351-371` that processes each parsed signal. Add the `apply_event` call right after `db.record_signal`:

Replace:

```rust
        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    // Read the current cycle id on each signal so rotations
                    // (STATUS:COMPLETE → new cycle) are reflected immediately.
                    let cur_cycle = *cycle_id.lock().unwrap();
                    let _ = db.record_signal(
                        session_id,
                        cur_cycle,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );
                    if let Signal::RoleSet { role } = &sig {
                        let _ = db.set_session_role(session_id, role);
                    }
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                }
            }
        });
```

with:

```rust
        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    // Legacy: write to the `signals` table for the old API path.
                    // Sub-plan 5 removes this. The new pipeline below funnels the
                    // same data through state::apply_event so the events log is
                    // populated even though hooks/MCP haven't shipped yet.
                    let cur_cycle = *cycle_id.lock().unwrap();
                    let _ = db.record_signal(
                        session_id,
                        cur_cycle,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );

                    // New funnel path. Source::System because the legacy parser
                    // is an internal bridge, not a model-chosen MCP tool.
                    let _ = crate::state::apply_event(
                        &db,
                        session_id,
                        crate::state::EventInput {
                            source: crate::state::Source::System,
                            kind: format!("signal.{}", sig.kind()),
                            payload: serde_json::json!({
                                "raw_kind": sig.kind(),
                                "payload": sig.payload(),
                            }),
                            // Allow replays (no dedup) — the signals table
                            // already has its own append history.
                            dedup_key: None,
                        },
                    );

                    if let Signal::RoleSet { role } = &sig {
                        // Role write stays direct in this sub-plan; mcp.set_role
                        // (Sub-plan 3) will replace it via the funnel.
                        let _ = db.set_session_role(session_id, role);
                    }
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                }
            }
        });
```

- [ ] **Step 4: Run the bridge test**

```bash
cargo test --test state_signal_bridge
```

Expected: PASS.

- [ ] **Step 5: Run the full suite**

```bash
cargo test
```

Expected: every test green. The legacy `tests/pty_signal_pump.rs` and `tests/role_signal_persistence.rs` still pass because the legacy path is untouched.

- [ ] **Step 6: Commit**

```bash
git add src/pty.rs tests/state_signal_bridge.rs
git commit -m "feat(pty): SignalPump dual-writes signals into events log

Per spec §9.4 first commit: legacy stdout-signal pipeline now also funnels
each parsed signal through state::apply_event with kind 'signal.<KIND>' so
the new events table is populated by the existing pipeline. Sub-plan 5 will
delete the legacy signals table write."
```

---

## Self-review checklist

After completing the tasks above, run the following yourself:

**1. Spec coverage (§-by-§ scan):**

- §8.1 (7-state lifecycle): Task 3 ✓
- §8.2 (DB delta): Task 1 ✓
- §8.3 (atomicity invariant): Tasks 5 + 7 + 8 (every mutation path goes through `apply_event`) ✓
- §8.5 (event-log usage — recovery / forensics / timeline): out of scope for Sub-plan 1; data exists, consumers come in Sub-plans 2-4 ✓
- §8.6 (transition reference): Task 5's `transition()` covers the rows applicable today (signal bridge + lifecycle); hook-driven rows (`Stop`, `UserPromptSubmit`) are flagged with `_ => current` no-op + comment so Sub-plan 2 fills them in ✓
- §8.7 (migration): Tasks 1 + 2 ✓
- §9.4 first commit ("add new surfaces alongside old"): nothing deleted; bridge added ✓
- §15 dependencies (CC SSE / hook semantics / etc.): out of scope; Sub-plan 1 has no external dependencies ✓

**2. Placeholder scan:**

```bash
grep -nE 'TBD|TODO|implement later|fill in details|appropriate (error|validation)' \
    docs/superpowers/plans/2026-04-18-foundations-hooks-mcp.md
```

Should return zero hits.

**3. Type consistency:**

- `EventInput`, `EventOutcome`, `Source`, `SessionState` — used consistently across Tasks 4-8 ✓
- Funnel function name `state::apply_event` (not `apply_state_event`, not `event::apply`) — used consistently ✓
- Migration constant naming `SCHEMA_V1` / `SCHEMA_V2` — matches existing convention in `src/db.rs` ✓
- DB helper name `Db::append_event_in_tx` (free function pattern, takes a `&Transaction`) — used consistently in Task 5 ✓

**4. Test coverage:**

- `tests/db_v2_schema.rs` — schema delta
- `tests/db_v2_pre_migration_reconciliation.rs` — §8.7 rule
- `tests/state_session_state.rs` — enum + EventInput/Outcome/Source types
- `tests/state_apply_event.rs` — transition table + atomicity + dedup
- `tests/session_manager_apply_event.rs` — manager flows through funnel
- `tests/state_signal_bridge.rs` — legacy pump dual-writes

Existing tests (`db_init`, `signals`, `pty_signal_pump`, `role_signal_persistence`, `session_manager*`, `db_*`, `api_*`, `ritual_*`) all stay green.

---

## Done definition for Sub-plan 1

- `cargo test` passes (all existing + 6 new test files).
- `cargo clippy --all-targets -- -D warnings` clean on the new + modified files.
- A fresh `cargo run --bin galvolite -- serve` starts cleanly against an empty DB and creates a v2 schema.
- A fresh `cargo run --bin galvolite -- serve` against an existing v1 DB applies migration 0002 and reconciles non-ended sessions to `error`.
- Spawning a real session (mock-claude or live `claude`) and observing it via `sqlite3 ~/.galvolite/galvolite.db "SELECT kind, prev_state, next_state FROM events ORDER BY id"` shows lifecycle + signal-bridge rows landing in order.
- No new endpoints. No UI changes. Sub-plans 2 (hooks), 3 (MCP), 4 (dashboard), 5 (legacy delete) can now begin in parallel from this base.
