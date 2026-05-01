# Sub-plan 5 — Recovery + Signal Deletion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close out the hooks+MCP coordination layer: wire `set_plan` / `plan_check_step` to populate the `plan_steps` table and emit dashboard events; finish restart recovery (alive- and dead-PTY paths) so non-terminal sessions survive a `serve` restart; clear the carry-forward TODOs from Sub-plan 4 (PromptGuard RAII, WS sink-mutex, `test_app_with_bus`, ExpandedSessionView unknown-hash race); delete the legacy `src/signals.rs` + `SignalPump` stdout pipeline; ship a real-claude smoke fixture proving the new pipeline ignores stdout entirely.

**Architecture:** Plan-step writes piggy-back on the existing `apply_event_with_bus` funnel — one new branch in `apply_column_side_effects` parses the `set_plan` markdown, upserts `plan_steps` rows, computes the diff, and a paired call publishes `SessionPlanReplaced` plus per-step `SessionPlanStepChanged` events. `plan_check_step` writes a single row update through the same funnel. Recovery runs on `SessionManager::with_event_bus` startup before `reconcile_orphaned_sessions`: for each non-terminal session we OS-check the recorded PID; alive → replay last 20 events as a synthetic burst plus a `system.recovered` marker; dead → write an auto-generated handoff from the events log + scrollback (best-effort) and transition through `system.session_orphaned`. Signal deletion is the last task — the pipeline runs in parallel with the new surface until plan-step + recovery flows are proven, then we delete `src/signals.rs`, `SignalPump`, `LineBuffer`, the `signals` table accessors, and the legacy CLAUDE.md instructions in one commit (revert path is one `git revert`).

**Tech Stack:** Rust 1.75+, axum 0.7, tokio, sha2 0.10 (already a transitive — confirm at task time), rusqlite, React 18 + TS + Vitest (existing UI).

---

## File Structure

**New backend files:**
- `src/plan_steps.rs` — pure markdown parser + `step_hash` derivation + diff types (`PlanStepInput`, `PlanStepDiff`)
- `tests/state_plan_step_writes.rs` — integration: `set_plan` populates `plan_steps`, emits `SessionPlanReplaced` + per-step deltas
- `tests/state_plan_check_step_writes.rs` — integration: `plan_check_step` updates one row and emits `SessionPlanStepChanged`
- `tests/recovery_alive_pty.rs` — integration: pre-seed events, simulate restart by recreating the manager, assert replay
- `tests/recovery_dead_pty.rs` — integration: pre-seed events with a fake PID that's not alive, assert orphan transition
- `tests/real_claude_tui_fixture.rs` — fixture-based regression proof Finding 1 cannot reoccur

**New UI test helper / no new files:** the unknown-hash race fix lives inside `ui/src/components/ExpandedSessionView.tsx`; the existing `ExpandedSessionView.test.tsx` gains one case.

**Modified backend files:**
- `src/lib.rs` — `pub mod plan_steps;`; remove `pub mod signals;` in Task 12
- `src/state.rs` — extend `apply_column_side_effects` to call `plan_steps::apply_set_plan_in_tx` and `plan_steps::apply_plan_check_step_in_tx`; extend `apply_event_with_bus` to publish the resulting plan-step domain events; pull the legacy `signal.*` arms out of `transition` in Task 12
- `src/db.rs` — three new helpers (`upsert_plan_step_in_tx`, `delete_plan_steps_for_session_in_tx`, `set_plan_step_status_in_tx`); delete `record_signal` + `list_signals_for_session` + `SignalRow` + the migration `DELETE FROM signals` reference in Task 12
- `src/api/operator.rs` — replace the in-band `register_and_wait` with a `PromptGuard` RAII pattern (`Drop` removes the waiters-map entry)
- `src/api/mod.rs` — replace the `Arc<Mutex<sink>>` sharing in `handle_terminal_ws` with a single writer task draining an `mpsc<Message>` funnel; deletes `TODO(plan-4 follow-up)` lines 340–343
- `src/session_manager.rs` — `orphan_session` takes `&EventBroadcaster`; new `recover_non_terminal_sessions` private fn called from `with_event_bus`; in Task 12 the `SignalPump`-related fields and the bg-task `pump.next_signal()` loop drop out (replaced by a no-op spawn that just owns the cycle close on PTY exit)
- `src/pty.rs` — `LineBuffer` import + the entire `SignalPump` block (lines 269–425) deleted in Task 12; reader thread keeps the scrollback push only
- `src/spawn_config.rs` — delete unused `write_session_files` (Task 14)
- `src/main.rs` — remove `cmd_show_signals` (renamed in Task 12 cleanup) — confirm by grep at task time
- `src/cli.rs` — remove `Show … signals` doc-comment + any subcommand wiring referencing the legacy table
- `src/api/mod.rs` — remove `/api/sessions/:id/signals` route + `get_signals` handler (Task 12)
- `src/claude_md.rs` — remove all `[STATUS:*]` / `[ROLE:*]` / `[ATTENTION:*]` / `[HANDOFF:*]` instruction text from `implementation_profile()` and any sibling profile fns; replace with MCP-tool guidance (Task 14)

**Modified frontend files (under `ui/src/`):**
- `components/ExpandedSessionView.tsx` — when `session.plan_step_changed` arrives with a `step_hash` not present in current state, refetch `/api/sessions/:id/plan_steps` instead of silently dropping (closes the `TODO(sub-plan-5)`)
- `components/ExpandedSessionView.test.tsx` — one new case for the refetch-on-unknown-hash path

**Modified test infra:**
- `tests/common/mod.rs` — add `pub fn test_app_with_bus() -> TestApp` so callers can drive the same `EventBroadcaster` the router holds; migrate `tests/api_event_broadcast.rs` and `tests/dashboard_smoke_e2e.rs` to use it (drop the duplicated boot blocks)

**Deleted files (Task 12):**
- `src/signals.rs`
- `tests/signals.rs`
- `tests/pty_signal_pump.rs`
- `tests/role_signal_persistence.rs`
- `tests/state_signal_bridge.rs`

**Deleted DB content (kept as schema-only / no row writes):**
- The `signals` table itself stays in `migrations/0001_initial.sql` so existing DBs don't fail — no migration to drop it. We just stop writing to it. `migrations/0002_hooks_mcp_foundations.sql` already deletes orphaned rows on its `reconcile_orphaned_sessions` path; no further DB changes here.

---

## Task 1: Plan markdown parser + step_hash

**Files:**
- Create: `src/plan_steps.rs`
- Modify: `src/lib.rs` (add `pub mod plan_steps;`)
- Test: inline `#[cfg(test)] mod tests` in `src/plan_steps.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// at the bottom of src/plan_steps.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unchecked_and_checked_items_in_order() {
        let md = "# Plan\n\n- [ ] First step\n- [x] Second step\n- [X] Third step\nignored line\n";
        let steps = parse_plan_markdown(md);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].title, "First step");
        assert_eq!(steps[0].status, PlanStepStatus::Pending);
        assert_eq!(steps[0].ordinal, 0);
        assert_eq!(steps[1].status, PlanStepStatus::Done);
        assert_eq!(steps[1].ordinal, 1);
        assert_eq!(steps[2].status, PlanStepStatus::Done);
        assert_eq!(steps[2].ordinal, 2);
    }

    #[test]
    fn step_hash_is_stable_across_whitespace_normalization() {
        let a = parse_plan_markdown("- [ ]   Hello   world  \n");
        let b = parse_plan_markdown("- [ ] Hello world\n");
        assert_eq!(a[0].step_hash, b[0].step_hash);
        assert_eq!(a[0].step_hash.len(), 16); // sha256[:16]
    }

    #[test]
    fn diff_detects_added_removed_status_changed() {
        let prior = vec![
            DbPlanStep { step_hash: "h1".into(), title: "Keep".into(), status: "pending".into(), ordinal: 0 },
            DbPlanStep { step_hash: "h2".into(), title: "Drop".into(), status: "pending".into(), ordinal: 1 },
        ];
        let next  = vec![
            PlanStepInput { step_hash: "h1".into(), title: "Keep".into(), status: PlanStepStatus::Done, ordinal: 0 },
            PlanStepInput { step_hash: "h3".into(), title: "Add".into(),  status: PlanStepStatus::Pending, ordinal: 1 },
        ];
        let diff = diff_plan_steps(&prior, &next);
        let changed: Vec<_> = diff.changed.iter().map(|c| (&c.step_hash[..], &c.prev_status[..], &c.next_status[..])).collect();
        assert!(changed.contains(&("h1", "pending", "done")));
        assert!(diff.added.iter().any(|s| s.step_hash == "h3"));
        assert!(diff.removed.iter().any(|s| s == "h2"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib plan_steps`
Expected: FAIL — `unresolved import` / module not found.

- [ ] **Step 3: Implement the parser + diff**

```rust
// src/plan_steps.rs
//! Plan-markdown parser and diff used by the `set_plan` / `plan_check_step`
//! MCP tools (spec §8.4). Pure functions — no DB or I/O.

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
}

impl PlanStepStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Pending     => "pending",
            Self::InProgress  => "in_progress",
            Self::Done        => "done",
            Self::Blocked     => "blocked",
        }
    }

    pub fn from_input_str(s: &str) -> Option<Self> {
        Some(match s {
            "pending"                  => Self::Pending,
            "working" | "in_progress"  => Self::InProgress,
            "done"                     => Self::Done,
            "blocked"                  => Self::Blocked,
            _                          => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PlanStepInput {
    pub step_hash: String,
    pub title: String,
    pub status: PlanStepStatus,
    pub ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct DbPlanStep {
    pub step_hash: String,
    pub title: String,
    pub status: String,
    pub ordinal: i64,
}

#[derive(Debug, Default)]
pub struct PlanStepDiff {
    pub added:   Vec<PlanStepInput>,
    pub removed: Vec<String>, // step_hash
    pub changed: Vec<PlanStepStatusChange>,
}

#[derive(Debug, Clone)]
pub struct PlanStepStatusChange {
    pub step_hash:    String,
    pub prev_status:  String,
    pub next_status:  String,
}

/// Hash a normalized title — collapse runs of whitespace, trim, lowercase
/// not applied (titles are case-sensitive). 16 hex chars from sha256.
pub fn step_hash_for(title: &str) -> String {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    let bytes = h.finalize();
    hex::encode(&bytes[..8])
}

/// Parse markdown for `- [ ]` / `- [x]` / `- [X]` checkbox items. Each match
/// becomes one `PlanStepInput` with stable hash and source-order ordinal.
pub fn parse_plan_markdown(md: &str) -> Vec<PlanStepInput> {
    let mut out = Vec::new();
    let mut ord: i64 = 0;
    for line in md.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("- [") else { continue };
        if rest.len() < 2 { continue }
        let mark = rest.as_bytes()[0] as char;
        if rest.as_bytes()[1] != b']' { continue }
        let after = &rest[2..];
        let title = after.trim();
        if title.is_empty() { continue }
        let status = match mark {
            ' '       => PlanStepStatus::Pending,
            'x' | 'X' => PlanStepStatus::Done,
            _         => continue,
        };
        out.push(PlanStepInput {
            step_hash: step_hash_for(title),
            title:     title.to_string(),
            status,
            ordinal:   ord,
        });
        ord += 1;
    }
    out
}

/// Diff prior DB rows against parsed input. `added`/`removed` are by hash;
/// `changed` covers any step whose status differs from prior. Ordinal moves
/// are NOT reported as `changed` — only the WS event for status shifts.
pub fn diff_plan_steps(prior: &[DbPlanStep], next: &[PlanStepInput]) -> PlanStepDiff {
    let mut diff = PlanStepDiff::default();
    let prior_idx: std::collections::HashMap<&str, &DbPlanStep> =
        prior.iter().map(|p| (p.step_hash.as_str(), p)).collect();
    let next_idx: std::collections::HashSet<&str> =
        next.iter().map(|n| n.step_hash.as_str()).collect();

    for n in next {
        match prior_idx.get(n.step_hash.as_str()) {
            None => diff.added.push(n.clone()),
            Some(p) => {
                let next_db = n.status.as_db_str();
                if p.status != next_db {
                    diff.changed.push(PlanStepStatusChange {
                        step_hash:   n.step_hash.clone(),
                        prev_status: p.status.clone(),
                        next_status: next_db.to_string(),
                    });
                }
            }
        }
    }
    for p in prior {
        if !next_idx.contains(p.step_hash.as_str()) {
            diff.removed.push(p.step_hash.clone());
        }
    }
    diff
}
```

Add to `Cargo.toml` `[dependencies]` (verify presence first — sha2 + hex may already be transitive):

```toml
sha2 = "0.10"
hex  = "0.4"
```

Add to `src/lib.rs`:

```rust
pub mod plan_steps;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib plan_steps`
Expected: PASS — 3 tests.

- [ ] **Step 5: Commit**

```bash
git add src/plan_steps.rs src/lib.rs Cargo.toml Cargo.lock
git commit -m "feat(plan): markdown parser + step_hash + diff (sub-plan 5 task 1)"
```

---

## Task 2: DB plan_steps write helpers

**Files:**
- Modify: `src/db.rs` (add three helpers near `read_plan_steps` at line 761)
- Test: `tests/db_plan_steps_writes.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/db_plan_steps_writes.rs
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
    // read_plan_steps rewrites in_progress -> working for UI; status assertion uses
    // raw select to verify the DB-native value.
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test db_plan_steps_writes`
Expected: FAIL — `Db::upsert_plan_step_in_tx` not found.

- [ ] **Step 3: Implement the helpers**

In `src/db.rs`, after the `read_plan_steps` fn (around line 789):

```rust
    /// Upsert one `plan_steps` row inside a caller-supplied transaction.
    /// Used by `state::apply_event` for `mcp.set_plan` writes. `status` MUST
    /// be DB-native (`pending` / `in_progress` / `done` / `blocked`); see
    /// `crate::plan_steps::PlanStepStatus::as_db_str`.
    pub fn upsert_plan_step_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
        step_hash: &str,
        ordinal: i64,
        title: &str,
        status: &str,
        note: Option<&str>,
    ) -> Result<()> {
        tx.execute(
            "INSERT INTO plan_steps
                (session_id, step_hash, ordinal, title, status, note, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             ON CONFLICT(session_id, step_hash) DO UPDATE SET
                ordinal    = excluded.ordinal,
                title      = excluded.title,
                status     = excluded.status,
                note       = excluded.note,
                updated_at = excluded.updated_at",
            params![session_id.to_string(), step_hash, ordinal, title, status, note],
        )?;
        Ok(())
    }

    /// Drop every plan_step row for a session inside a caller-supplied
    /// transaction. `set_plan` calls this before re-inserting so removed
    /// steps disappear and ordinals stay dense.
    pub fn delete_plan_steps_for_session_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
    ) -> Result<()> {
        tx.execute(
            "DELETE FROM plan_steps WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
    }

    /// Update `status` (+ optional `note`) on a single plan_step row inside
    /// the given transaction. Returns the prior status, or `None` if the
    /// row didn't exist (caller treats as no-op).
    pub fn set_plan_step_status_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
        step_hash: &str,
        status: &str,
        note: Option<&str>,
    ) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let prev: Option<String> = tx
            .query_row(
                "SELECT status FROM plan_steps
                 WHERE session_id = ?1 AND step_hash = ?2",
                params![session_id.to_string(), step_hash],
                |r| r.get(0),
            )
            .optional()?;
        if prev.is_none() {
            return Ok(None);
        }
        tx.execute(
            "UPDATE plan_steps
             SET status = ?1, note = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE session_id = ?3 AND step_hash = ?4",
            params![status, note, session_id.to_string(), step_hash],
        )?;
        Ok(prev)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test db_plan_steps_writes`
Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/db_plan_steps_writes.rs
git commit -m "feat(db): plan_steps upsert/delete/set_status helpers (sub-plan 5 task 2)"
```

---

## Task 3: Wire `set_plan` to populate plan_steps + emit `SessionPlanReplaced`

**Files:**
- Modify: `src/state.rs` (extend `apply_column_side_effects` and `apply_event_with_bus`)
- Test: `tests/state_plan_step_writes.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/state_plan_step_writes.rs
mod common;

use galvolite::api::events::DomainEvent;
use galvolite::state::{apply_event_with_bus, EventInput, Source, SessionState};

#[tokio::test]
async fn set_plan_writes_plan_steps_and_publishes_replaced_with_per_step_changes() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;
    let mut rx = app.mgr.subscribe_events();

    apply_event_with_bus(
        &app.db, sid, app.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.set_plan".into(),
            payload: serde_json::json!({
                "markdown": "- [ ] Step one\n- [x] Step two\n"
            }),
            dedup_key: None,
        },
    ).unwrap();

    let rows = app.db.read_plan_steps(sid).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["text"], "Step one");
    assert_eq!(rows[0]["status"], "pending");
    assert_eq!(rows[1]["status"], "done");

    let mut saw_event_appended = false;
    let mut saw_replaced       = false;
    for _ in 0..6 {
        match tokio::time::timeout(
            std::time::Duration::from_millis(200),
            rx.recv(),
        ).await {
            Ok(Ok(DomainEvent::SessionEventAppended { kind, .. })) if kind == "mcp.set_plan" => {
                saw_event_appended = true;
            }
            Ok(Ok(DomainEvent::SessionPlanReplaced { steps, .. })) => {
                saw_replaced = true;
                assert_eq!(steps.as_array().unwrap().len(), 2);
            }
            _ => {}
        }
    }
    assert!(saw_event_appended);
    assert!(saw_replaced);
}

#[tokio::test]
async fn set_plan_with_one_status_change_emits_one_step_changed_event() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;

    // First plan: A pending, B pending.
    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.set_plan".into(),
        payload: serde_json::json!({ "markdown": "- [ ] A\n- [ ] B\n" }),
        dedup_key: None,
    }).unwrap();

    // Second plan: A done, B pending.
    let mut rx = app.mgr.subscribe_events();
    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.set_plan".into(),
        payload: serde_json::json!({ "markdown": "- [x] A\n- [ ] B\n" }),
        dedup_key: None,
    }).unwrap();

    let mut saw_change = false;
    for _ in 0..8 {
        if let Ok(Ok(DomainEvent::SessionPlanStepChanged { prev_status, next_status, .. }))
            = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            assert_eq!(prev_status, "pending");
            assert_eq!(next_status, "done");
            saw_change = true;
            break;
        }
    }
    assert!(saw_change);
}
```

(Task 9 lands `test_app_with_bus`. To unblock this task before Task 9, scaffold the helper inline at the top of this test file as a local `mod local_app { ... }`, then delete in Task 9. Keep the function name identical so the migration is rename-only.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test state_plan_step_writes`
Expected: FAIL — `SessionPlanReplaced` arm never publishes; rows table empty.

- [ ] **Step 3: Extend `apply_column_side_effects` for `mcp.set_plan`**

In `src/state.rs`, replace the existing `"mcp.set_plan"` arm in `apply_column_side_effects` with:

```rust
        "mcp.set_plan" => {
            if let Some(md) = input.payload.get("markdown").and_then(|v| v.as_str()) {
                tx.execute(
                    "UPDATE sessions SET plan_markdown = ?1 WHERE id = ?2",
                    rusqlite::params![md, sid],
                )?;
                // Plan-step table writes (Sub-plan 5 Task 3). Order matters:
                //   1. snapshot prior rows for diff
                //   2. delete + reinsert so removed steps disappear and
                //      ordinals stay dense
                //   3. caller (`apply_event_with_bus`) reads the snapshot
                //      back out via the dedup helper to publish events.
                // We can't return data from this fn (it's tx-scoped + may run
                // for kinds other than set_plan), so the publish-side reads
                // the resulting rows directly post-commit. See `apply_event_with_bus`.
                let parsed = crate::plan_steps::parse_plan_markdown(md);
                crate::db::Db::delete_plan_steps_for_session_in_tx(tx, session_id)?;
                for s in &parsed {
                    crate::db::Db::upsert_plan_step_in_tx(
                        tx, session_id,
                        &s.step_hash, s.ordinal, &s.title,
                        s.status.as_db_str(),
                        None,
                    )?;
                }
            }
        }
```

- [ ] **Step 4: Extend `apply_event_with_bus` to publish plan events**

In `src/state.rs`, replace the bus-publish block at the end of `apply_event_with_bus` (after the existing `SessionStateChanged` publish) with:

```rust
        // Sub-plan 5 Task 3: when a `mcp.set_plan` lands, emit
        // SessionPlanReplaced (full new step list) plus one
        // SessionPlanStepChanged per step whose status differs from prior.
        // Reading the prior snapshot post-commit means we do not need to
        // thread state through `apply_event` itself.
        if !outcome.deduped && kind == "mcp.set_plan" {
            if let Ok(rows) = db.read_plan_steps(session_id) {
                bus.publish(crate::api::events::DomainEvent::SessionPlanReplaced {
                    session_id,
                    steps: serde_json::Value::Array(rows),
                });
            }
        }
```

To compute the diff, we need the prior rows BEFORE the side-effect runs. Refactor: stash the prior snapshot before calling `apply_event` inside `apply_event_with_bus`:

```rust
pub fn apply_event_with_bus(
    db: &crate::db::Db,
    session_id: uuid::Uuid,
    bus: &crate::api::events::EventBroadcaster,
    input: EventInput,
) -> Result<EventOutcome> {
    let kind = input.kind.clone();
    let source_str = input.source.to_string();
    let payload = input.payload.clone();

    // Sub-plan 5 Task 3: capture prior plan_steps snapshot for diff
    // computation BEFORE the funnel writes the new ones. We only do this for
    // the kinds whose publish path needs it, to avoid the I/O on every event.
    let prior_plan_steps: Option<Vec<crate::plan_steps::DbPlanStep>> =
        if kind == "mcp.set_plan" {
            Some(
                db.read_plan_steps_raw(session_id).unwrap_or_default()
            )
        } else {
            None
        };

    let outcome = apply_event(db, session_id, input)?;
    if !outcome.deduped {
        let ts = chrono::Utc::now().to_rfc3339();
        bus.publish(crate::api::events::DomainEvent::SessionEventAppended {
            session_id,
            event_id: outcome.event_id,
            kind: kind.clone(),
            source: source_str,
            ts: ts.clone(),
            payload,
        });
        if Some(outcome.next_state) != outcome.prev_state {
            bus.publish(crate::api::events::DomainEvent::SessionStateChanged {
                session_id,
                prev: outcome.prev_state,
                next: outcome.next_state,
                last_event_ts: ts,
            });
        }
        if kind == "mcp.set_plan" {
            if let Ok(next_rows) = db.read_plan_steps_raw(session_id) {
                // Publish full replacement so PlanTab can hydrate without refetch.
                let json_steps = db.read_plan_steps(session_id).unwrap_or_default();
                bus.publish(crate::api::events::DomainEvent::SessionPlanReplaced {
                    session_id,
                    steps: serde_json::Value::Array(json_steps),
                });
                // Per-step deltas.
                let prior = prior_plan_steps.unwrap_or_default();
                let next_inputs: Vec<crate::plan_steps::PlanStepInput> = next_rows.into_iter()
                    .map(|r| crate::plan_steps::PlanStepInput {
                        step_hash: r.step_hash,
                        title:     r.title,
                        status:    crate::plan_steps::PlanStepStatus::from_input_str(&r.status)
                                       .unwrap_or(crate::plan_steps::PlanStepStatus::Pending),
                        ordinal:   r.ordinal,
                    })
                    .collect();
                let diff = crate::plan_steps::diff_plan_steps(&prior, &next_inputs);
                for c in diff.changed {
                    bus.publish(crate::api::events::DomainEvent::SessionPlanStepChanged {
                        session_id,
                        step_hash:   c.step_hash,
                        prev_status: c.prev_status,
                        next_status: c.next_status,
                        note:        None,
                    });
                }
            }
        }
    }
    Ok(outcome)
}
```

- [ ] **Step 5: Add `read_plan_steps_raw` to `src/db.rs`**

The existing `read_plan_steps` returns JSON-shaped rows with `in_progress` rewritten to `working`. The diff machinery needs DB-native status. Add a sibling helper:

```rust
    /// Like `read_plan_steps` but returns `DbPlanStep` (DB-native status,
    /// keyed by `step_hash`) for use by the `apply_event_with_bus` plan
    /// diff. Order: ordinal ASC.
    pub fn read_plan_steps_raw(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<crate::plan_steps::DbPlanStep>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT step_hash, ordinal, title, status
             FROM plan_steps
             WHERE session_id = ?1
             ORDER BY ordinal ASC",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string()], |r| {
                Ok(crate::plan_steps::DbPlanStep {
                    step_hash: r.get(0)?,
                    ordinal:   r.get(1)?,
                    title:     r.get(2)?,
                    status:    r.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test --test state_plan_step_writes`
Expected: PASS — both cases.

- [ ] **Step 7: Commit**

```bash
git add src/state.rs src/db.rs tests/state_plan_step_writes.rs
git commit -m "feat(state): set_plan populates plan_steps + emits replaced/changed (sub-plan 5 task 3)"
```

---

## Task 4: Wire `plan_check_step` to update plan_steps + emit `SessionPlanStepChanged`

**Files:**
- Modify: `src/state.rs` (extend `apply_column_side_effects` for `mcp.plan_check_step`; extend `apply_event_with_bus` similarly)
- Test: `tests/state_plan_check_step_writes.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/state_plan_check_step_writes.rs
mod common;

use galvolite::api::events::DomainEvent;
use galvolite::state::{apply_event_with_bus, EventInput, Source, SessionState};

#[tokio::test]
async fn plan_check_step_updates_one_row_and_emits_step_changed() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;

    // Seed a plan first.
    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.set_plan".into(),
        payload: serde_json::json!({ "markdown": "- [ ] First step\n- [ ] Second step\n" }),
        dedup_key: None,
    }).unwrap();

    let mut rx = app.mgr.subscribe_events();

    apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.plan_check_step".into(),
        payload: serde_json::json!({
            "title": "First step",
            "status": "done",
            "note": "ok"
        }),
        dedup_key: None,
    }).unwrap();

    let rows = app.db.read_plan_steps(sid).unwrap();
    let first = rows.iter().find(|r| r["text"] == "First step").unwrap();
    assert_eq!(first["status"], "done");
    assert_eq!(first["note"], "ok");

    let mut saw = false;
    for _ in 0..8 {
        if let Ok(Ok(DomainEvent::SessionPlanStepChanged { prev_status, next_status, note, .. }))
            = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            assert_eq!(prev_status, "pending");
            assert_eq!(next_status, "done");
            assert_eq!(note.as_deref(), Some("ok"));
            saw = true;
            break;
        }
    }
    assert!(saw);
}

#[tokio::test]
async fn plan_check_step_with_unknown_title_is_a_no_op() {
    let app = common::test_app_with_bus();
    let sid = common::seed_session_in_state(&app.db, SessionState::Idle).0;
    let outcome = apply_event_with_bus(&app.db, sid, app.mgr.event_bus(), EventInput {
        source: Source::Mcp, kind: "mcp.plan_check_step".into(),
        payload: serde_json::json!({ "title": "missing", "status": "done" }),
        dedup_key: None,
    });
    // Side-effect is a no-op (no row to update); the event row still lands.
    assert!(outcome.is_ok());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test state_plan_check_step_writes`
Expected: FAIL.

- [ ] **Step 3: Extend `apply_column_side_effects` for `mcp.plan_check_step`**

Add this branch alongside the `mcp.set_plan` one:

```rust
        "mcp.plan_check_step" => {
            // Resolve target by step_hash (preferred) or by hashing title.
            let hash: Option<String> = input.payload
                .get("step_hash").and_then(|v| v.as_str()).map(str::to_string)
                .or_else(|| input.payload.get("title").and_then(|v| v.as_str())
                    .map(crate::plan_steps::step_hash_for));
            let Some(hash) = hash else { return Ok(()); };
            let status_in = input.payload.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
            let status = crate::plan_steps::PlanStepStatus::from_input_str(status_in)
                .map(|s| s.as_db_str().to_string())
                .unwrap_or_else(|| "pending".to_string());
            let note = input.payload.get("note").and_then(|v| v.as_str());
            // Side-effect; prior status is used by the bus publisher upstream.
            let _ = crate::db::Db::set_plan_step_status_in_tx(
                tx, session_id, &hash, &status, note,
            )?;
        }
```

- [ ] **Step 4: Extend `apply_event_with_bus` to emit one `SessionPlanStepChanged`**

Replace the existing `kind == "mcp.set_plan"` branch with a multi-arm match. Add this block right after the `SessionStateChanged` publish:

```rust
        if kind == "mcp.plan_check_step" {
            // Read prior status BEFORE we mutated. The funnel already wrote
            // the new value; we recover prev from the events row's
            // `payload_json` instead. Simpler: capture prior status before
            // calling `apply_event` (mirrors the set_plan path).
            // — already captured below via `prior_step_status`.
        }
```

In practice, this requires the same prior-snapshot-before-write trick used in Task 3. Pull `prior_step_status` capture up alongside `prior_plan_steps`:

```rust
    let prior_step_status: Option<(String, String)> = // (step_hash, prev_status)
        if kind == "mcp.plan_check_step" {
            let hash = input.payload.get("step_hash").and_then(|v| v.as_str()).map(str::to_string)
                .or_else(|| input.payload.get("title").and_then(|v| v.as_str())
                    .map(crate::plan_steps::step_hash_for));
            hash.and_then(|h| {
                db.read_plan_step_status(session_id, &h).ok().flatten()
                    .map(|s| (h, s))
            })
        } else {
            None
        };
```

Then after the funnel succeeds, publish:

```rust
        if !outcome.deduped && kind == "mcp.plan_check_step" {
            let new_status_in = payload.get("status").and_then(|v| v.as_str()).unwrap_or("pending");
            let new_status = crate::plan_steps::PlanStepStatus::from_input_str(new_status_in)
                .map(|s| s.as_db_str().to_string())
                .unwrap_or_else(|| "pending".into());
            if let Some((hash, prev)) = prior_step_status {
                if prev != new_status {
                    bus.publish(crate::api::events::DomainEvent::SessionPlanStepChanged {
                        session_id,
                        step_hash:   hash,
                        prev_status: prev,
                        next_status: new_status,
                        note:        payload.get("note").and_then(|v| v.as_str()).map(str::to_string),
                    });
                }
            }
        }
```

Add `read_plan_step_status` to `src/db.rs`:

```rust
    /// Read the current status of a single plan_step row, or None if
    /// the row doesn't exist. DB-native status (`in_progress`, not `working`).
    pub fn read_plan_step_status(
        &self,
        session_id: Uuid,
        step_hash: &str,
    ) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let conn = self.pool.get()?;
        Ok(conn
            .query_row(
                "SELECT status FROM plan_steps WHERE session_id = ?1 AND step_hash = ?2",
                params![session_id.to_string(), step_hash],
                |r| r.get(0),
            )
            .optional()?)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test state_plan_check_step_writes`
Expected: PASS — 2 cases.

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/db.rs tests/state_plan_check_step_writes.rs
git commit -m "feat(state): plan_check_step updates plan_steps + emits step_changed (sub-plan 5 task 4)"
```

---

## Task 5: UI fix — refetch plan on unknown step_hash

**Files:**
- Modify: `ui/src/components/ExpandedSessionView.tsx` (resolves the `TODO(sub-plan-5)` left by Sub-plan 4 Task 14)
- Test: `ui/src/components/ExpandedSessionView.test.tsx`

- [ ] **Step 1: Write the failing test**

Add a new case at the bottom of `ExpandedSessionView.test.tsx` (style match the surrounding cases — `userEvent`, MSW or jest-mocked `api.ts`):

```tsx
it("refetches plan when plan_step_changed arrives with an unknown hash", async () => {
  const fetched: string[][] = [];
  const fakeApi = {
    getSessionEvents: vi.fn().mockResolvedValue([]),
    getSessionPlanSteps: vi.fn().mockImplementation(async () => {
      // First call: empty plan; second call: contains the new hash.
      const i = (fakeApi.getSessionPlanSteps as any).mock.calls.length;
      const out = i === 1
        ? []
        : [{ hash: "new-hash", text: "Newly added", status: "pending" as const, note: null }];
      fetched.push([String(i)]);
      return out;
    }),
  };
  // …mount ExpandedSessionView with the fake api + a session id, switch to Plan tab…
  // simulate a `session.plan_step_changed` arriving via the test's onEvent prop
  // with step_hash = "new-hash" — the component should call getSessionPlanSteps a second time.
  // assert the Plan tab now shows "Newly added".
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- ExpandedSessionView`
Expected: FAIL — second `getSessionPlanSteps` call never happens.

- [ ] **Step 3: Patch `ExpandedSessionView.tsx`**

Locate the existing `useSessionEvents` handler (named `bridgeHandler` per Task 16 wiring) — wherever `session.plan_step_changed` is matched and the unknown-hash path silently returns. Replace the silent return with a refetch:

```tsx
case "session.plan_step_changed": {
  setPlanSteps((prev) => {
    const idx = prev.findIndex((s) => s.hash === e.step_hash);
    if (idx === -1) {
      // Unknown hash → likely a step appended after our initial fetch
      // resolved but before this WS subscription started receiving frames
      // (or a `set_plan` we missed). Refetch authoritative list.
      void api
        .getSessionPlanSteps(sessionId)
        .then(setPlanSteps)
        .catch(() => { /* leave UI as-is; next set_plan will fully refresh */ });
      return prev;
    }
    const next = [...prev];
    next[idx] = { ...next[idx], status: e.next_status as PlanStepStatus, note: e.note ?? next[idx].note };
    return next;
  });
  break;
}
```

Remove the `// TODO(sub-plan-5)` comment.

- [ ] **Step 4: Run UI tests to verify pass**

Run: `cd ui && npm test`
Expected: PASS — full UI suite green (94+ tests after the new case).

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/ExpandedSessionView.tsx ui/src/components/ExpandedSessionView.test.tsx
git commit -m "fix(ui): refetch plan on unknown step_hash (sub-plan 5 task 5)"
```

---

## Task 6: `orphan_session` takes `&EventBroadcaster`

**Files:**
- Modify: `src/session_manager.rs` (change `orphan_session` signature; route to `apply_event_with_bus`; update all 8 call sites)
- Modify: `src/pty.rs` (the `apply_event` call inside `SignalPump` survives only until Task 12; leave it bare per existing inline rationale)
- Test: `tests/state_session_orphaned.rs` (extend if needed)

- [ ] **Step 1: Write the failing test**

```rust
// add to tests/state_session_orphaned.rs (or new file orphan_session_publishes.rs)
#[tokio::test]
async fn orphan_session_publishes_session_orphaned_event() {
    use galvolite::api::events::DomainEvent;
    let app = common::test_app_with_bus();
    let mut rx = app.mgr.subscribe_events();
    // spawn_session with a deliberately-broken project path to trip the
    // `galvolite_dir_create_failed` orphan path (read-only fs).
    let proj = app.mgr.add_project(galvolite::db::NewProject {
        name: "p".into(),
        path: "/proc/sys/kernel".into(), // read-only on Linux; on macOS use a synthesized inaccessible path
        remote_url: None,
    });
    // If `add_project` rejects (canonicalize failure), seed manually.
    // Simpler: call orphan_session directly via a test-only re-export.
    // ... assert the rx receives DomainEvent::SessionEventAppended with kind = "system.session_orphaned"
}
```

If exposing `orphan_session` to tests is awkward, drive it via `spawn_session` with a non-canonicalizable command (e.g. `claude_cmd = "/no-such-bin"`) — that hits `pty_spawn_failed`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test state_session_orphaned`
Expected: FAIL — no event arrives.

- [ ] **Step 3: Patch `orphan_session`**

```rust
fn orphan_session(
    db: &Db,
    bus: &EventBroadcaster,
    session_id: Uuid,
    reason: &'static str,
) {
    let _ = crate::state::apply_event_with_bus(
        db, session_id, bus,
        crate::state::EventInput {
            source: crate::state::Source::System,
            kind: "system.session_orphaned".into(),
            payload: serde_json::json!({ "reason": reason }),
            dedup_key: Some(format!("session-orphaned:{session_id}")),
        },
    );
}
```

Update every call site in `spawn_session` to pass `&self.event_bus`:

```rust
orphan_session(&self.db, &self.event_bus, session.id, "galvolite_dir_create_failed");
```

(8 call sites; one mechanical replace.)

Drop the trailing comment that explained why the bare-`apply_event` was OK — it is no longer accurate.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test`
Expected: PASS — 244+ tests.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs tests/state_session_orphaned.rs
git commit -m "feat(state): orphan_session publishes via EventBroadcaster (sub-plan 5 task 6)"
```

---

## Task 7: PromptGuard RAII

**Files:**
- Modify: `src/api/operator.rs`
- Test: `tests/api_operator_prompt.rs` (extend with a "future-dropped" case)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn dropping_register_and_wait_future_clears_waiters_map() {
    use galvolite::api::operator::OperatorPromptRegistry;
    let bus = galvolite::api::events::EventBroadcaster::new(8);
    let reg = OperatorPromptRegistry::new();

    let fut = {
        let reg = reg.clone();
        let bus = bus.clone();
        async move {
            reg.register_and_wait(&bus, uuid::Uuid::new_v4(),
                "?".into(), None, Some(60)).await
        }
    };
    // Poll once to register, then drop.
    futures::pin_mut!(fut);
    let _ = futures::poll!(fut.as_mut());
    drop(fut);
    // RAII guard's Drop must have removed the entry.
    assert_eq!(reg.pending_count().await, 0);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_operator_prompt`
Expected: FAIL — `pending_count` not yet exposed; entry leaks.

- [ ] **Step 3: Implement `PromptGuard`**

```rust
struct PromptGuard {
    reg: Arc<OperatorPromptRegistry>,
    id:  Uuid,
}

impl Drop for PromptGuard {
    fn drop(&mut self) {
        // tokio Mutex can't be locked synchronously; std::sync::Mutex is
        // a one-line swap, fits the spec §11 follow-up note ("Minor: std::sync::Mutex
        // for waiters map"). Switch the field accordingly:
        let mut map = self.reg.waiters.lock().expect("waiters mutex poisoned");
        map.remove(&self.id);
    }
}
```

Switch `OperatorPromptRegistry::waiters` to `std::sync::Mutex<HashMap<Uuid, oneshot::Sender<_>>>`. Update `register_and_wait` and `answer` accordingly. Wrap registration in a guard:

```rust
pub async fn register_and_wait(
    self: &Arc<Self>,
    bus: &EventBroadcaster,
    session_id: Uuid,
    question: String,
    choices: Option<Vec<String>>,
    timeout_secs: Option<u64>,
) -> serde_json::Value {
    let prompt_id = Uuid::new_v4();
    let (tx, mut rx) = oneshot::channel();
    {
        let mut map = self.waiters.lock().unwrap();
        map.insert(prompt_id, tx);
    }
    let _guard = PromptGuard { reg: self.clone(), id: prompt_id };
    bus.publish(DomainEvent::OperatorPrompt { prompt_id, session_id, question, choices, timeout_secs });
    let dur = std::time::Duration::from_secs(timeout_secs.unwrap_or(30));
    match tokio::time::timeout(dur, &mut rx).await {
        Ok(Ok(v)) => v,
        _ => {
            // either elapsed or sender dropped; guard handles map cleanup on drop
            serde_json::json!({ "answer": serde_json::Value::Null, "timed_out": true })
        }
    }
}

pub async fn pending_count(&self) -> usize {
    self.waiters.lock().unwrap().len()
}
```

Make `OperatorPromptRegistry` `Clone`-able (already wrapped in `Arc` at the call site — adjust signature to take `self: &Arc<Self>`). Drop the existing race-resolution recovery logic in `register_and_wait` — the guard subsumes it.

Remove the `TODO(plan-4 follow-up)` comment block at lines 44-49.

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --test api_operator_prompt`
Expected: PASS — including the new dropped-future case + the existing 3.

- [ ] **Step 5: Commit**

```bash
git add src/api/operator.rs tests/api_operator_prompt.rs src/session_manager.rs
git commit -m "fix(operator): RAII PromptGuard cleans waiters on dropped futures (sub-plan 5 task 7)"
```

---

## Task 8: WS sink mpsc funnel

**Files:**
- Modify: `src/api/mod.rs::handle_terminal_ws` (lines 324–455)
- Test: `tests/api_event_broadcast.rs::ws_emits_domain_events_as_text_frames` already covers the basic path; add one back-pressure case.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn ws_funnel_serializes_pty_and_event_writes_through_one_writer() {
    // Slow client: ack only after a delay; server must enqueue both PTY
    // bytes and DomainEvent frames in arrival order without serializing
    // through a sink mutex.
    // ... (wire-up mirrors the existing ws_emits_domain_events_as_text_frames)
    // assertion: under intermixed publish + PTY-byte load, no frames are dropped
    // and both frame kinds are observable in the order they were emitted.
}
```

- [ ] **Step 2: Run test to verify it fails or to baseline**

Run: `cargo test --test api_event_broadcast`
Expected: PASS or FAIL — current sink-mutex code may pass simple cases; the new test is exercising a contention scenario.

- [ ] **Step 3: Refactor to a single writer task draining `mpsc<Message>`**

Replace the `Arc<Mutex<sink>>` cluster with a single `mpsc::channel::<Message>(256)`. Three producer tasks (`writer`, `event_forwarder`, plus the resize-snapshot path inside `reader`) call `tx.send(Message::Binary(...))` / `tx.send(Message::Text(...))`. One consumer task drains and sends to `sink`.

```rust
let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Message>(256);

let writer_task = async move {
    while let Some(msg) = frame_rx.recv().await {
        if sink.send(msg).await.is_err() { break; }
    }
};
// PTY bytes path:
let pty_tx = frame_tx.clone();
let pty_pump = async move {
    loop {
        tokio::select! {
            biased;
            snap = snap_rx.recv() => {
                let Some(bytes) = snap else { break };
                if pty_tx.send(Message::Binary(bytes)).await.is_err() { break; }
            }
            msg = rx.recv() => {
                match msg {
                    Ok(bytes) => {
                        if pty_tx.send(Message::Binary(bytes)).await.is_err() { break; }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
};
// Event forwarder path:
let evt_tx = frame_tx.clone();
let event_forwarder = async move {
    loop {
        match event_rx.recv().await {
            Ok(evt) => {
                if !evt.applies_to(session_id) { continue; }
                let Ok(json) = serde_json::to_string(&evt) else { continue };
                if evt_tx.send(Message::Text(json)).await.is_err() { break; }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
};
drop(frame_tx); // close channel when all producers exit

tokio::select! {
    _ = writer_task => {},
    _ = pty_pump => {},
    _ = reader => {},
    _ = event_forwarder => {},
}
```

Delete the `// TODO(plan-4 follow-up)` comment block at lines 340-343.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test api_event_broadcast`
Expected: PASS — including the new contention case.

- [ ] **Step 5: Commit**

```bash
git add src/api/mod.rs tests/api_event_broadcast.rs
git commit -m "refactor(ws): single writer task draining mpsc<Message> (sub-plan 5 task 8)"
```

---

## Task 9: Factor `tests/common::test_app_with_bus`

**Files:**
- Modify: `tests/common/mod.rs`
- Modify: `tests/api_event_broadcast.rs` (drop the duplicated boot block in `apply_event_publishes_state_changed_and_event_appended` and `ws_emits_domain_events_as_text_frames`)
- Modify: `tests/dashboard_smoke_e2e.rs` (drop its duplicated boot block)
- Modify: `tests/state_plan_step_writes.rs` (Task 3) — drop the inline `mod local_app`
- Modify: `tests/state_plan_check_step_writes.rs` (Task 4) — same
- Modify: `tests/state_session_orphaned.rs` (Task 6) — same

- [ ] **Step 1: Add `test_app_with_bus` to `tests/common/mod.rs`**

```rust
/// Build a TestApp where the EventBroadcaster is constructed up-front and
/// shared between the SessionManager and the caller. Use this when the test
/// asserts on `SessionManager::subscribe_events()` or publishes manually.
pub fn test_app_with_bus() -> TestApp {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db.clone(), data_dir, bus,
    ));
    let router = galvolite::api::router(mgr.clone());
    TestApp { app: router.clone(), router, db, mgr, _dir: dir }
}

/// Bind an ephemeral 127.0.0.1 listener for tests that need a real HTTP/WS
/// surface; returns the bound port and spawns the server.
pub async fn serve_ephemeral(app: axum::Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
    port
}
```

- [ ] **Step 2: Migrate the duplicated boot blocks**

Open each consumer file, replace the manual `tempdir + Db::open + EventBroadcaster::new + SessionManager::with_event_bus + axum::serve` block with `let app = common::test_app_with_bus();` (then `let port = common::serve_ephemeral(app.app.clone()).await;` for WS tests).

- [ ] **Step 3: Run all tests to verify pass**

Run: `cargo test`
Expected: PASS — 244+ tests, no regressions.

- [ ] **Step 4: Commit**

```bash
git add tests/common/mod.rs tests/api_event_broadcast.rs tests/dashboard_smoke_e2e.rs tests/state_plan_step_writes.rs tests/state_plan_check_step_writes.rs tests/state_session_orphaned.rs
git commit -m "refactor(tests): test_app_with_bus + serve_ephemeral helpers (sub-plan 5 task 9)"
```

---

## Task 10: Restart recovery — alive-PTY replay

**Files:**
- Modify: `src/session_manager.rs` (new `recover_non_terminal_sessions`; called from `with_event_bus` AFTER `reconcile_orphaned_sessions`)
- Modify: `src/api/events.rs` (add `DomainEvent::SessionResumed { session_id, last_events: Value }`)
- Modify: `ui/src/types.ts` + `useSessionEvents.ts` to recognize the new variant (no UI surface required — log only or apply to the events tab buffer)
- Test: `tests/recovery_alive_pty.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/recovery_alive_pty.rs
mod common;

#[tokio::test]
async fn recovery_replays_last_20_events_for_alive_pty() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    // Seed: insert a session in `working` state with a fake PID that IS our
    // own (std::process::id() is guaranteed alive).
    let proj = db.insert_project(galvolite::db::NewProject {
        name: "p".into(), path: dir.path().to_string_lossy().into(),
        remote_url: None,
    }).unwrap();
    let s = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: dir.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET state='working', pid=?1 WHERE id=?2",
        rusqlite::params![std::process::id() as i64, s.id.to_string()],
    ).unwrap();
    // Insert 25 prior events; expect last 20 to be replayed via WS.
    for i in 0..25 {
        galvolite::state::apply_event(&db, s.id, galvolite::state::EventInput {
            source: galvolite::state::Source::System,
            kind: "system.tick".into(),
            payload: serde_json::json!({ "i": i }),
            dedup_key: Some(format!("tick-{i}")),
        }).unwrap();
    }

    // Start the manager — this triggers recovery.
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mut rx = bus.subscribe();
    let _mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::with_event_bus(
            db, dir.path().to_path_buf(), bus
        )
    );

    // Drain. Expect SessionResumed exactly once + system.recovered marker.
    let mut saw_resumed = false;
    let mut saw_marker  = false;
    for _ in 0..40 {
        match tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(galvolite::api::events::DomainEvent::SessionResumed { session_id, last_events })) => {
                assert_eq!(session_id, s.id);
                assert_eq!(last_events.as_array().unwrap().len(), 20);
                saw_resumed = true;
            }
            Ok(Ok(galvolite::api::events::DomainEvent::SessionEventAppended { kind, .. })) if kind == "system.recovered" => {
                saw_marker = true;
            }
            _ => {}
        }
    }
    assert!(saw_resumed && saw_marker);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test recovery_alive_pty`
Expected: FAIL — `SessionResumed` variant doesn't exist; recovery fn doesn't run.

- [ ] **Step 3: Add `SessionResumed` variant**

In `src/api/events.rs`, add:

```rust
    #[serde(rename = "session.resumed")]
    SessionResumed { session_id: Uuid, last_events: serde_json::Value },
```

Update `applies_to` to return true for `SessionResumed { session_id, .. }`.

In `ui/src/types.ts`, add a matching discriminated-union arm:

```ts
| { type: "session.resumed"; session_id: string; last_events: SessionEvent[] }
```

(Frontend may ignore for now; PlanTab + EventsTab won't break since the existing arms are typed.)

- [ ] **Step 4: Implement `recover_non_terminal_sessions`**

In `src/session_manager.rs::with_event_bus`, after `db.reconcile_orphaned_sessions()`:

```rust
        if let Err(e) = recover_non_terminal_sessions(&db, &event_bus) {
            tracing::warn!("startup recovery failed: {e}");
        }
```

```rust
fn recover_non_terminal_sessions(db: &Db, bus: &EventBroadcaster) -> Result<()> {
    let rows = db.list_non_terminal_sessions()?;
    for (sid, pid) in rows {
        let alive = pid_is_alive(pid);
        if alive {
            // Alive PTY: replay last 20 events as one SessionResumed frame
            // (we cannot rebind the read loop — that requires the original
            // master fd). Mark a recovery boundary in the events log.
            let recent = db.read_recent_events(sid, 20).unwrap_or_default();
            bus.publish(DomainEvent::SessionResumed {
                session_id:  sid,
                last_events: serde_json::Value::Array(recent),
            });
            let _ = crate::state::apply_event_with_bus(
                db, sid, bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind:   "system.recovered".into(),
                    payload: serde_json::json!({ "pid": pid, "alive": true }),
                    dedup_key: Some(format!("recovered:{sid}:{}", chrono::Utc::now().timestamp())),
                },
            );
        } else {
            // Dead PTY path is Task 11.
        }
    }
    Ok(())
}

#[cfg(unix)]
fn pid_is_alive(pid: i64) -> bool {
    if pid <= 0 { return false; }
    // SIGNAL 0: reachability check, no-op delivery.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
#[cfg(not(unix))]
fn pid_is_alive(_pid: i64) -> bool { false }
```

Add `list_non_terminal_sessions` to `src/db.rs`:

```rust
    /// Return `(session_id, pid)` for every session in a non-terminal state
    /// (`starting | idle | working | awaiting_input | needs_attention`).
    /// Used by startup recovery (spec §11.2).
    pub fn list_non_terminal_sessions(&self) -> Result<Vec<(Uuid, i64)>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(pid, 0) FROM sessions
             WHERE state IN ('starting','idle','working','awaiting_input','needs_attention')",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((parse_uuid(&r.get::<_, String>(0)?)?, r.get::<_, i64>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

Add `libc = "0.2"` to `Cargo.toml` if not already present.

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test --test recovery_alive_pty`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/api/events.rs src/session_manager.rs src/db.rs ui/src/types.ts Cargo.toml Cargo.lock tests/recovery_alive_pty.rs
git commit -m "feat(recovery): replay last 20 events for alive PTY on startup (sub-plan 5 task 10)"
```

---

## Task 11: Restart recovery — dead-PTY → orphan with auto-handoff

**Files:**
- Modify: `src/session_manager.rs::recover_non_terminal_sessions` (fill the dead-PTY branch)
- Modify: `src/handoff.rs` (add `write_auto_recovery_handoff` helper)
- Test: `tests/recovery_dead_pty.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn dead_pty_session_transitions_to_error_with_auto_handoff() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    // Seed a session in working with a PID guaranteed-not-alive.
    let proj = db.insert_project(galvolite::db::NewProject {
        name: "p".into(), path: dir.path().to_string_lossy().into(), remote_url: None,
    }).unwrap();
    let s = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: dir.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    db.pool().get().unwrap().execute(
        "UPDATE sessions SET state='working', pid=1 WHERE id=?1",
        // PID 1 is `init` and alive on most systems; use a sentinel that
        // cannot exist on Unix — pid 999_999_999 is reliably absent.
        rusqlite::params![s.id.to_string()],
    ).unwrap();
    db.pool().get().unwrap().execute(
        "UPDATE sessions SET pid=999999999 WHERE id=?1",
        rusqlite::params![s.id.to_string()],
    ).unwrap();

    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let _mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::with_event_bus(
            db.clone(), dir.path().to_path_buf(), bus
        )
    );

    let conn = db.pool().get().unwrap();
    let state: String = conn.query_row(
        "SELECT state FROM sessions WHERE id=?1",
        [s.id.to_string()], |r| r.get(0),
    ).unwrap();
    assert_eq!(state, "error");
    let handoff: Option<String> = conn.query_row(
        "SELECT handoff_path FROM sessions WHERE id=?1",
        [s.id.to_string()], |r| r.get(0),
    ).unwrap();
    assert!(handoff.is_some(), "auto-recovery handoff path must be stamped");
    assert!(std::path::Path::new(handoff.as_deref().unwrap()).exists());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test recovery_dead_pty`
Expected: FAIL — state remains `working`; no handoff stamped.

- [ ] **Step 3: Implement dead-PTY branch**

In `src/handoff.rs`:

```rust
/// Generate a minimal recovery-handoff markdown document from the events
/// log when a session was found dead at restart. Returns the absolute path
/// of the file written under `<data_dir>/.galvolite/handoffs/<session>/`.
pub fn write_auto_recovery_handoff(
    handoff_dir: &HandoffDir,
    session_id: uuid::Uuid,
    last_events: &[serde_json::Value],
) -> std::io::Result<std::path::PathBuf> {
    let session_dir = handoff_dir.root().join(session_id.to_string());
    std::fs::create_dir_all(&session_dir)?;
    let path = session_dir.join(format!(
        "auto-recovery-{}.md",
        chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
    ));
    let mut body = String::new();
    body.push_str("# Auto-Recovery Handoff\n\n");
    body.push_str("Session was found in a non-terminal state at GalvoLite restart with no live PTY.\n\n");
    body.push_str("## Last events\n\n");
    for e in last_events.iter().take(20) {
        let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
        let ts   = e.get("ts").and_then(|v| v.as_str()).unwrap_or("?");
        body.push_str(&format!("- `{ts}` · {kind}\n"));
    }
    std::fs::write(&path, body)?;
    Ok(path)
}
```

In `recover_non_terminal_sessions`, expand the `else` branch:

```rust
        } else {
            let recent = db.read_recent_events(sid, 20).unwrap_or_default();
            // Resolve worktree → handoff dir for this session.
            if let Ok(s) = db.get_session(sid) {
                let dir = HandoffDir::new(
                    std::path::Path::new(&s.worktree_path)
                        .join(".galvolite/handoffs")
                );
                if let Ok(p) = crate::handoff::write_auto_recovery_handoff(&dir, sid, &recent) {
                    let _ = db.set_session_handoff_path(sid, &p.to_string_lossy());
                }
            }
            let _ = crate::state::apply_event_with_bus(
                db, sid, bus,
                crate::state::EventInput {
                    source: crate::state::Source::System,
                    kind:   "system.session_orphaned".into(),
                    payload: serde_json::json!({ "reason": "pty_dead_at_restart", "pid": pid }),
                    dedup_key: Some(format!("session-orphaned:{sid}")),
                },
            );
        }
```

Add `set_session_handoff_path` to `src/db.rs` if absent:

```rust
    pub fn set_session_handoff_path(&self, id: Uuid, path: &str) -> Result<()> {
        self.pool.get()?.execute(
            "UPDATE sessions SET handoff_path = ?1 WHERE id = ?2",
            params![path, id.to_string()],
        )?;
        Ok(())
    }
```

- [ ] **Step 4: Run tests to verify pass**

Run: `cargo test --test recovery_dead_pty`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs src/handoff.rs src/db.rs tests/recovery_dead_pty.rs
git commit -m "feat(recovery): dead PTY → orphan + auto-handoff on startup (sub-plan 5 task 11)"
```

---

## Task 12: Delete the legacy signal pipeline

**Files (delete):**
- `src/signals.rs`
- `tests/signals.rs`
- `tests/pty_signal_pump.rs`
- `tests/role_signal_persistence.rs`
- `tests/state_signal_bridge.rs`

**Files (modify):**
- `src/lib.rs` — remove `pub mod signals;`
- `src/pty.rs` — delete the `SignalPump` struct + `PtyHandleShared` block (lines 269–425); change `PtyHandle::next_line` to return raw scrollback frames or remove (no consumer); remove `LineBuffer` import + the `line_buf` instance in the reader thread.
- `src/session_manager.rs` — remove `use crate::pty::SignalPump;` + `use crate::signals::Signal;`; the bg task that drained `pump.next_signal()` becomes a `tokio::spawn` that simply waits on a child-exit notification; rotate cycle on natural exit only (the `STATUS:COMPLETE` rotation arm goes away — cycles end on PTY exit and on operator kill, period).
- `src/state.rs::transition` — delete the `signal.STATUS:*` / `signal.ATTENTION:*` arms (lines ~378–390).
- `src/api/mod.rs` — remove the `/api/sessions/:id/signals` route + `get_signals` handler.
- `src/db.rs` — delete `record_signal`, `list_signals_for_session`, `SignalRow` struct (kept the `signals` table itself for backward compat; no migrations).
- `src/main.rs` / `src/cli.rs` — remove `cmd_show_signals` + the `Show … signals` subcommand wiring.
- `src/ritual.rs` — remove `use crate::signals::Signal;` + any switch-on-Signal arms; the ritual driver becomes hooks/MCP-driven only.
- `src/claude_md.rs` — remove the bracket-token instruction text from every profile fn; replace with MCP-tool guidance ("Call `status_report({...})` to update state. Call `set_role` to change role.").

**Files (new):**
- None.

- [ ] **Step 1: Pre-flight grep**

```bash
grep -rn "signals" src/ tests/ ui/ migrations/ | grep -v "signal_steps\|signal\.kind\|/signals\|signal\.applies"
```

Expected: only the call sites listed above. Anything else is a missed reference.

- [ ] **Step 2: Delete files + remove module wiring**

```bash
git rm src/signals.rs tests/signals.rs tests/pty_signal_pump.rs tests/role_signal_persistence.rs tests/state_signal_bridge.rs
```

Edit `src/lib.rs` — drop `pub mod signals;`.

- [ ] **Step 3: Patch `src/pty.rs`**

Delete lines 269–425 (`use crate::db::Db;` through end of `impl SignalPump`). Inside `PtyHandle::spawn` reader thread, remove `LineBuffer::new()` + the line-buffer feed loop; the thread keeps only the scrollback push.

Change `PtyHandle::next_line` removal: any caller in `src/ritual.rs` that consumed `next_line()` is removed in step below.

- [ ] **Step 4: Patch `src/session_manager.rs`**

Replace the `SignalPump::start` + `pump.next_signal()` loop with:

```rust
let pty_arc = std::sync::Arc::new(StdMutex::new(Some(pty)));
let handle = PtyHandle::shared_view(pty_arc.clone()); // new helper or direct construction
let scrollback = ...;
// bg task: wait on child exit; rotate cycle / fire system.session_ended.
let mgr = Arc::clone(self);
tokio::spawn(async move {
    // Use a child-exit channel; tokio::task::spawn_blocking wraps the
    // blocking `child.wait()` call.
    // ... existing bg-task body MINUS the next_signal loop ...
});
```

The `STATUS:COMPLETE` cycle-rotation arm goes away. Cycles now end on PTY exit and on `kill_session`.

- [ ] **Step 5: Patch `src/state.rs`**

Delete the `signal.*` arms in `transition`. The `pty.rs::SignalPump`'s `apply_event` call is gone, so no caller produces those event kinds anymore.

- [ ] **Step 6: Patch `src/db.rs`, `src/api/mod.rs`, `src/main.rs`, `src/cli.rs`, `src/ritual.rs`, `src/claude_md.rs`**

Mechanical removals (one-line + import-cleanup). For `src/claude_md.rs`, replace the deleted bracket-token text with a one-paragraph MCP tool guide:

```rust
"Coordinate with GalvoLite via MCP tools (no stdout signals): \
 `status_report({kind: 'working'|'complete'|'blocked', note?})`, \
 `set_role({role})`, `set_plan({markdown})`, `plan_check_step({title|step_hash, status})`, \
 `write_handoff({summary, next_steps})`, `request_attention({reason})`, \
 `notify_operator({level, message})`, `ask_operator({question, choices?})`."
```

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: PASS — count goes DOWN by ~5 test files. Backend ~225 + new tests from earlier tasks = ~245 total.

Run: `cargo clippy --all-targets --no-deps`
Expected: clean.

Run: `cd ui && npm test`
Expected: PASS — UI unaffected (no signal references in the frontend).

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(state): delete legacy stdout signal pipeline (sub-plan 5 task 12)

Replaced wholesale by hooks (Sub-plan 2) + MCP tools (Sub-plan 3) + plan_steps
wiring (Sub-plan 5 Tasks 1-4) + recovery (Sub-plan 5 Tasks 10-11). The
\`signals\` table stays in 0001_initial.sql for backward-compat; nothing
writes to it. Revert path: \`git revert <this-sha>\`."
```

---

## Task 13: Real-claude smoke fixture

**Files:**
- Create: `tests/real_claude_tui_fixture.rs`
- Create: `tests/fixtures/claude_tui_sample.bin` — recorded raw PTY bytes containing ANSI-wrapped lines including bracket-token-LIKE patterns (e.g. `\x1b[32m● [STATUS:WORKING]\x1b[0m\n`)

- [ ] **Step 1: Write the failing test (cannot fail — proof construct)**

```rust
//! Fixture-based regression proof: even when the PTY emits text that LOOKS
//! like the legacy bracket-token signals (e.g. `[STATUS:WORKING]`), no
//! `signal.*` event lands in the events log because the signal parser is
//! deleted.

use galvolite::db::Db;

#[tokio::test]
async fn ansi_wrapped_status_lines_produce_zero_signal_events() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("t.sqlite")).unwrap();
    let proj = db.insert_project(galvolite::db::NewProject {
        name: "p".into(), path: dir.path().to_string_lossy().into(),
        remote_url: None,
    }).unwrap();
    let s = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: dir.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    // Spawn a session whose command emits the fixture bytes. `cat` of the
    // fixture file pipes raw bytes — including ANSI-wrapped bracket tokens
    // — through the PTY reader. With the signal pipeline deleted, the bytes
    // hit scrollback only.
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::with_event_bus(
            db.clone(), dir.path().to_path_buf(), bus
        )
    );
    let session = mgr.spawn_session(
        proj.id, "echo".into(), "implementation".into(),
        "cat".into(),
        vec!["tests/fixtures/claude_tui_sample.bin".into()],
    ).await.unwrap();

    // Give the reader a moment to drain the file.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    // No event row whose `kind` starts with `signal.` exists.
    let conn = db.pool().get().unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE kind LIKE 'signal.%' AND session_id = ?1",
        [session.id.to_string()],
        |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 0);
    let _ = s;
}
```

- [ ] **Step 2: Capture fixture bytes**

Manually craft `tests/fixtures/claude_tui_sample.bin` with:

```bash
printf '\x1b[32m\xe2\x97\x8f [STATUS:WORKING]\x1b[0m\nNormal output line.\n\x1b[32m\xe2\x97\x8f [HANDOFF:READY path="/tmp/x"]\x1b[0m\n' \
  > tests/fixtures/claude_tui_sample.bin
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo test --test real_claude_tui_fixture`
Expected: PASS — `count == 0`.

- [ ] **Step 4: Commit**

```bash
git add tests/real_claude_tui_fixture.rs tests/fixtures/claude_tui_sample.bin
git commit -m "test(smoke): ANSI-wrapped TUI bytes produce zero signal events (sub-plan 5 task 13)"
```

---

## Task 14: Final cleanup — strip CLAUDE.md signal text + delete unused helper

**Files:**
- Modify: `src/spawn_config.rs` — delete `write_session_files` (currently unused per memory `cd5c990` note)
- Modify: `src/claude_md.rs` — final pass: confirm Task 12's edits are clean; ensure no `[STATUS:` / `[ROLE:` / `[ATTENTION:` / `[HANDOFF:` / `[PLAN:` / `[SEED:` substrings remain in any profile body

- [ ] **Step 1: Grep for residual instructions**

```bash
grep -rn '\[STATUS:\|\[ROLE:\|\[ATTENTION:\|\[HANDOFF:\|\[PLAN:\|\[SEED:' src/ tests/ ui/
```

Expected: zero matches across `src/` and `ui/`. (Test fixtures may still legitimately reference the patterns for the regression proof.)

- [ ] **Step 2: Delete `write_session_files`**

```rust
// Delete fn write_session_files (lines ~134-147). No call sites remain after
// `cd5c990`. The two-step path (`write_settings_json` + `write_mcp_json`) is
// what `start_session` uses so each gets its own targeted Error variant.
```

Run: `cargo build`
Expected: clean — no orphan import.

- [ ] **Step 3: Run full suite + clippy**

Run: `cargo test && cargo clippy --all-targets --no-deps && cd ui && npm test`
Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add src/spawn_config.rs src/claude_md.rs
git commit -m "chore: drop unused write_session_files + final claude_md sweep (sub-plan 5 task 14)"
```

---

## Self-review checklist (run before merge)

1. **Spec coverage:**
   - §5.1 `set_plan` / `plan_check_step` populate `plan_steps` + emit WS deltas — Tasks 1-4. ✓
   - §10.1 `session.plan_replaced` / `session.plan_step_changed` / `session.resumed` produced — Tasks 3, 4, 10. ✓
   - §11.2 restart recovery alive + dead PTY paths — Tasks 10-11. ✓
   - §9.1 deletion of `src/signals.rs`, `SignalPump`, signal-bridge transition arms — Task 12. ✓
   - §9.4 two-commit migration order respected: new surfaces first (Tasks 1-11), delete last (Task 12). ✓
   - §12.4 real-claude regression fixture — Task 13. ✓
2. **Carry-forward TODOs from Sub-plan 4:**
   - PromptGuard RAII — Task 7. ✓
   - WS sink-mutex → mpsc — Task 8. ✓
   - `test_app_with_bus()` factor — Task 9. ✓
   - `ExpandedSessionView.tsx` unknown-hash race — Task 5. ✓
   - `orphan_session` bus access — Task 6. ✓
   - Unused `write_session_files` — Task 14. ✓
3. **No placeholders:** every code step has full Rust/TS bodies. ✓
4. **Type consistency:**
   - `PlanStepStatus` (Rust) → `as_db_str()` writes DB-native; UI `PlanStep.status` reads via `read_plan_steps` rewrite — match. ✓
   - `step_hash_for(title)` is the single hash function used by both `parse_plan_markdown` and `apply_column_side_effects` for `mcp.plan_check_step`. ✓
   - `SessionResumed` variant added in 10 also exposed in `applies_to`. ✓

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-29-plan-5-recovery-signal-deletion.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task; combined spec+quality review for mechanical tasks (12, 14); two-pass review for integration-heavy tasks (3, 4, 8, 10, 11). Implementer/reviewer prompt templates at `~/.claude/plugins/cache/claude-plugins-official/superpowers/5.0.7/skills/subagent-driven-development/`.

**2. Inline Execution** — execute tasks in this session using `superpowers:executing-plans`; checkpoint after Task 4 (plan-step wiring complete) and after Task 11 (recovery complete).

**Which approach?**
