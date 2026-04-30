# Sub-plan 6: Session Spawn / Fan-out / Rejoin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A running GalvoLite session can spawn child sessions, idle-wait until they reach terminal status, and resume on a single synthesized rejoin prompt — backed by `git worktree`-isolated children and a hook-driven payload-delivery path that survives PTY drops and recovery restarts.

**Architecture:** Three pillars. (1) Two new MCP tools — `spawn_session` and `wait_children` — let the model fan-out work in-turn. (2) New columns on `sessions` (`waiting_on`, `pending_rejoin`, `terminal_status`) plus a reactor in `SessionManager::on_child_terminal` form the harness state machine; the existing `parent_session_id` column from `0001_initial.sql` is reused. (3) When all children are terminal, the harness writes a marker token to the parent's PTY; the existing `galvolite-hook` binary recognises it, calls `GET /api/sessions/:id/pending-rejoin`, and feeds the markdown payload back into Claude Code as `additionalContext` while suppressing the marker via `decision: "block"`.

**Tech Stack:** Rust 1.75+, axum 0.7, rusqlite, tokio, reqwest (blocking, in the hook binary), uuid, serde_json. React 18 + Vitest for the UI delta. `git worktree` shells out via `std::process::Command`.

**Spec:** [`docs/superpowers/specs/2026-04-30-session-spawn-fanout-rejoin-design.md`](../specs/2026-04-30-session-spawn-fanout-rejoin-design.md). Read it first.

**Conventions:**
- The user has authorized direct main-branch commits for this plan, matching Plans 4 & 5.
- TDD: red → green → commit per task. Run `cargo test` before each commit.
- Frontend changes commit separately from backend even when topically related.
- Per memory `feedback_idle_sessions_no_cost.md`: the parent session's PTY is parked across a `wait_children` window. Do NOT add wall-clock polling.

---

## File Structure

**New files:**

| Path | Responsibility |
|---|---|
| `migrations/0003_session_spawn.sql` | Schema delta |
| `src/worktree.rs` | `git worktree` plumbing (add / remove / branch derivation / collision retry) |
| `src/mcp/tools/spawn.rs` | `spawn_session` + `wait_children` MCP tools, soft-cap warn toasts |
| `src/api/spawn.rs` | `GET /api/sessions/:id/pending-rejoin` handler with atomic clear + worktree GC |
| `src/hooks/marker.rs` | Pure marker pattern detection (testable without I/O) |
| `tests/worktree.rs` | Unit tests against a real git tempdir |
| `tests/spawn_session_mcp.rs` | Integration: spawn / wait_children round-trip |
| `tests/pending_rejoin_endpoint.rs` | Integration: atomic clear + worktree GC |
| `tests/spawn_recovery.rs` | Integration: restart mid-wait |
| `tests/spawn_e2e.rs` | E2E with mock-claude binary |
| `ui/src/components/SessionRow.test.tsx` | Vitest: depth prop + waiting badge |
| `ui/src/hooks/useSessions.test.ts` | Vitest: reducer arm for `session.children_resolved` |

**Extended files:**

| Path | Change |
|---|---|
| `src/db.rs` | Append `(3, SCHEMA_V3)` to `MIGRATIONS` |
| `src/session_manager.rs` | Four new methods: `spawn_child`, `set_waiting_on`, `build_rejoin_markdown`, `on_child_terminal` + recovery extension |
| `src/state.rs` | Register three new event kinds + side-effect arms |
| `src/api/events.rs` | Add `SessionChildrenResolved` variant |
| `src/api/mod.rs` | Wire `GET /api/sessions/:id/pending-rejoin` route |
| `src/bin/galvolite-hook.rs` | Intercept marker before `/hooks/ingest` forward, output Claude Code JSON |
| `src/hooks/client.rs` | Optional: a `fetch_pending_rejoin` helper for the binary |
| `src/mcp/tools/mod.rs` | Register `spawn::tools()` in `all()` |
| `src/claude_md.rs` | Extend orchestrator + brainstorm profiles to mention spawn tools |
| `ui/src/types.ts` | Add `waiting_on` on `Session`, add `session.children_resolved` `DomainEvent` |
| `ui/src/hooks/useSessions.ts` | Reducer arm for `session.children_resolved` |
| `ui/src/components/SessionRow.tsx` | `depth?: number` prop + "waiting on N" badge |
| `ui/src/components/Dashboard.tsx` | Render nested tree from `parent_session_id` |

---

## Task 1: Schema Migration `0003_session_spawn.sql`

**Files:**
- Create: `migrations/0003_session_spawn.sql`
- Modify: `src/db.rs:13-22` (append migration entry)
- Test: `tests/db_session_spawn_columns.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/db_session_spawn_columns.rs
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
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test db_session_spawn_columns
```

Expected: FAIL — file `0003_session_spawn.sql` not present in `MIGRATIONS`.

- [ ] **Step 3: Create the migration SQL**

```sql
-- migrations/0003_session_spawn.sql
-- Schema delta for Sub-plan 6 spawn/fan-out/rejoin.
-- See docs/superpowers/specs/2026-04-30-session-spawn-fanout-rejoin-design.md §2.
--
-- parent_session_id and worktree_path are already present from 0001_initial.sql;
-- this migration only adds the new wait/rejoin/terminal-status columns and the
-- parent lookup index.

ALTER TABLE sessions ADD COLUMN waiting_on      TEXT NULL;
ALTER TABLE sessions ADD COLUMN pending_rejoin  TEXT NULL;
ALTER TABLE sessions ADD COLUMN terminal_status TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
```

- [ ] **Step 4: Wire the migration into the loader**

Edit `src/db.rs` — at the top, alongside `SCHEMA_V1` and `SCHEMA_V2`:

```rust
const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");
const SCHEMA_V2: &str = include_str!("../migrations/0002_hooks_mcp_foundations.sql");
const SCHEMA_V3: &str = include_str!("../migrations/0003_session_spawn.sql");

const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, SCHEMA_V2),
    (3, SCHEMA_V3),
];
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test --test db_session_spawn_columns
cargo test  # full suite — confirm no existing test regresses on the schema add
```

Expected: PASS on the new test, no regressions in the suite.

- [ ] **Step 6: Commit**

```bash
git add migrations/0003_session_spawn.sql src/db.rs tests/db_session_spawn_columns.rs
git commit -m "feat(db): migration 0003 — waiting_on, pending_rejoin, terminal_status, parent index"
```

---

## Task 2: `src/worktree.rs` — `add_worktree` + branch derivation

**Files:**
- Create: `src/worktree.rs`
- Modify: `src/lib.rs` (add `pub mod worktree;`)
- Test: `tests/worktree.rs` (new)

- [ ] **Step 1: Write the failing tests**

```rust
// tests/worktree.rs
use galvolite::worktree::{add_worktree, derive_branch_name};
use std::process::Command;
use tempfile::TempDir;
use uuid::Uuid;

fn init_repo(dir: &std::path::Path) {
    Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir).status().unwrap();
    std::fs::write(dir.join("README"), "x").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
    Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"])
        .current_dir(dir).status().unwrap();
}

#[test]
fn derive_branch_name_uses_first_8_uuid_chars() {
    let id = Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap();
    assert_eq!(derive_branch_name(id), "galvolite/01234567");
}

#[test]
fn add_worktree_creates_dir_branch_and_returns_paths() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let id = Uuid::new_v4();
    let (wt, branch) = add_worktree(tmp.path(), id, "HEAD").expect("add_worktree");
    assert!(wt.exists(), "worktree dir created: {wt:?}");
    assert!(wt.starts_with(tmp.path().join(".galvolite/worktrees")));
    assert!(branch.starts_with("galvolite/"));
    // The branch ref exists in git
    let out = Command::new("git")
        .args(["rev-parse", "--verify", &branch])
        .current_dir(tmp.path())
        .output().unwrap();
    assert!(out.status.success(), "branch must resolve: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn add_worktree_retries_on_branch_collision() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let id = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
    // Pre-create the first-attempt branch to force collision.
    Command::new("git")
        .args(["branch", "galvolite/aaaaaaaa"])
        .current_dir(tmp.path())
        .status().unwrap();
    let (_wt, branch) = add_worktree(tmp.path(), id, "HEAD").expect("retry must succeed");
    assert_eq!(branch, "galvolite/aaaaaaaa-2");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test worktree
```

Expected: FAIL — `worktree` module does not exist.

- [ ] **Step 3: Implement `src/worktree.rs`**

```rust
//! `git worktree` plumbing for Sub-plan 6. Each child session gets its own
//! worktree under `<repo>/.galvolite/worktrees/<session-id>/` on a branch
//! derived from the session UUID. Collisions retry with `-2`, `-3` suffixes.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

/// `galvolite/<first-8-of-uuid>`.
pub fn derive_branch_name(session_id: Uuid) -> String {
    let s = session_id.simple().to_string();
    format!("galvolite/{}", &s[..8])
}

/// `git worktree add <repo>/.galvolite/worktrees/<id>/ -b <branch> <base_ref>`.
/// Retries the branch suffix `-2`, `-3` on collision; gives up after 3 tries.
pub fn add_worktree(
    repo_root: &Path,
    session_id: Uuid,
    base_ref: &str,
) -> Result<(PathBuf, String)> {
    let wt_dir = repo_root.join(".galvolite").join("worktrees").join(session_id.to_string());
    if let Some(parent) = wt_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let base_branch = derive_branch_name(session_id);
    let mut last_stderr = String::new();
    for attempt in 1..=3 {
        let branch = if attempt == 1 {
            base_branch.clone()
        } else {
            format!("{base_branch}-{attempt}")
        };
        let out = Command::new("git")
            .args([
                "-C", repo_root.to_str().ok_or_else(|| Error::Config("repo path not utf-8".into()))?,
                "worktree", "add",
                wt_dir.to_str().ok_or_else(|| Error::Config("worktree path not utf-8".into()))?,
                "-b", &branch, base_ref,
            ])
            .output()
            .map_err(|e| Error::io(repo_root, e))?;
        if out.status.success() {
            return Ok((wt_dir, branch));
        }
        last_stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        // Branch-already-exists is the only case we retry; any other error is fatal.
        if !last_stderr.contains("already exists") {
            return Err(Error::Config(format!("git worktree add failed: {last_stderr}")));
        }
    }
    Err(Error::Config(format!(
        "git worktree add: branch collision unresolved after 3 attempts: {last_stderr}"
    )))
}
```

Edit `src/lib.rs` — add `pub mod worktree;` next to other top-level modules.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test worktree
```

Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/worktree.rs src/lib.rs tests/worktree.rs
git commit -m "feat(worktree): add_worktree + derive_branch_name with collision retry"
```

---

## Task 3: `src/worktree.rs` — `remove_worktree`

**Files:**
- Modify: `src/worktree.rs`
- Test: `tests/worktree.rs` (extend)

- [ ] **Step 1: Write the failing test**

Append to `tests/worktree.rs`:

```rust
use galvolite::worktree::remove_worktree;

#[test]
fn remove_worktree_deletes_dir_and_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let id = Uuid::new_v4();
    let (wt, _branch) = add_worktree(tmp.path(), id, "HEAD").unwrap();
    assert!(wt.exists());
    remove_worktree(&wt).expect("first remove ok");
    assert!(!wt.exists(), "dir should be gone");
    // Idempotent: removing again must not error.
    remove_worktree(&wt).expect("second remove ok (idempotent)");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test worktree remove_worktree_deletes_dir_and_is_idempotent
```

Expected: FAIL — `remove_worktree` not defined.

- [ ] **Step 3: Implement `remove_worktree`**

Append to `src/worktree.rs`:

```rust
/// `git -C <repo> worktree remove --force <path>`. Idempotent: a missing
/// worktree dir is treated as success (the GC pass in the rejoin endpoint
/// re-runs this on every read; it must not error after the first call).
pub fn remove_worktree(worktree_path: &Path) -> Result<()> {
    if !worktree_path.exists() {
        return Ok(());
    }
    // The repo root is two levels up: <repo>/.galvolite/worktrees/<id>/
    let repo_root = worktree_path
        .ancestors()
        .nth(3)
        .ok_or_else(|| Error::Config(format!("worktree path too shallow: {worktree_path:?}")))?;
    let out = Command::new("git")
        .args([
            "-C", repo_root.to_str().ok_or_else(|| Error::Config("repo path not utf-8".into()))?,
            "worktree", "remove", "--force",
            worktree_path.to_str().ok_or_else(|| Error::Config("worktree path not utf-8".into()))?,
        ])
        .output()
        .map_err(|e| Error::io(repo_root, e))?;
    if !out.status.success() {
        // Fall back to plain rmdir; git refuses if it's not registered as a worktree.
        let _ = std::fs::remove_dir_all(worktree_path);
    }
    Ok(())
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test --test worktree
```

Expected: 4 PASS (3 prior + new).

- [ ] **Step 5: Commit**

```bash
git add src/worktree.rs tests/worktree.rs
git commit -m "feat(worktree): remove_worktree (idempotent)"
```

---

## Task 4: `SessionManager::spawn_child`

**Files:**
- Modify: `src/session_manager.rs`
- Test: `tests/spawn_session_mcp.rs` (new — only the spawn_child probe; later tasks extend)

- [ ] **Step 1: Write the failing test**

```rust
// tests/spawn_session_mcp.rs
mod common;
use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;

fn init_git(dir: &std::path::Path) {
    Command::new("git").args(["init", "-q", "-b", "main"]).current_dir(dir).status().unwrap();
    std::fs::write(dir.join("README"), "x").unwrap();
    Command::new("git").args(["add", "."]).current_dir(dir).status().unwrap();
    Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"])
        .current_dir(dir).status().unwrap();
}

#[tokio::test]
async fn spawn_child_creates_worktree_and_db_row_with_parent() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());

    let proj = app.mgr.add_project(NewProject {
        name: "p".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(),
        "/bin/cat".into(), vec![],
    ).await.expect("parent spawn");

    let child = app.mgr.spawn_child(parent.id, "implementation", "child task")
        .await.expect("spawn_child");

    assert_eq!(child.parent_session_id, Some(parent.id));
    assert!(std::path::Path::new(&child.worktree_path).exists());
    assert!(child.branch.as_deref().unwrap().starts_with("galvolite/"));
    assert_ne!(child.worktree_path, parent.worktree_path);
}
```

(Note: `Session` struct must already expose `parent_session_id` and `worktree_path`. If not deserialized from DB, extend `db::Session` accordingly in this task.)

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test spawn_session_mcp spawn_child_creates_worktree_and_db_row_with_parent
```

Expected: FAIL — `spawn_child` not defined.

- [ ] **Step 3: Implement `spawn_child` on `SessionManager`**

Add inside `impl SessionManager` in `src/session_manager.rs`:

```rust
/// Spawn a child session whose worktree is a fresh `git worktree` of the
/// parent's repo. Mints session row with `parent_session_id` set, runs the
/// usual settings/MCP/api-token writes, spawns the child PTY, applies
/// `mcp.spawn_session` (parent) + `system.session_started` (child) events.
pub async fn spawn_child(
    self: &Arc<Self>,
    parent_id: Uuid,
    role: &str,
    task: &str,
) -> Result<crate::db::Session> {
    let parent = self.db.get_session(parent_id)?;
    // The repo root is the project path; parent's worktree may itself be a child worktree.
    let project = self.db.get_project(parent.project_id)?;
    let repo_root = std::path::PathBuf::from(&project.path);

    // Resolve parent's HEAD as base ref so the child sees parent WIP commits.
    let head_out = std::process::Command::new("git")
        .args(["-C", parent.worktree_path.as_str(), "rev-parse", "HEAD"])
        .output()
        .map_err(|e| Error::io(&repo_root, e))?;
    let base_ref = if head_out.status.success() {
        String::from_utf8_lossy(&head_out.stdout).trim().to_string()
    } else {
        "HEAD".to_string()
    };

    // Provisional UUID so we can derive the branch before insert.
    let child_id = Uuid::new_v4();
    let (wt_path, branch) = crate::worktree::add_worktree(&repo_root, child_id, &base_ref)?;

    let session = self.db.insert_session(crate::db::NewSession {
        project_id: parent.project_id,
        role: role.to_string(),
        branch: Some(branch.clone()),
        worktree_path: wt_path.to_string_lossy().into_owned(),
        task_summary: task.to_string(),
        parent_session_id: Some(parent_id),
    })?;
    // Force the row id to match the worktree id we already created.
    if session.id != child_id {
        // Worktree namespace must match. Update path on disk: rename dir.
        let new_wt = repo_root.join(".galvolite").join("worktrees").join(session.id.to_string());
        std::fs::rename(&wt_path, &new_wt).map_err(|e| Error::io(&new_wt, e))?;
        self.db.update_worktree_path(session.id, &new_wt.to_string_lossy())?;
    }

    // Re-read to get final paths.
    let session = self.db.get_session(session.id)?;
    let wt_pathbuf = std::path::PathBuf::from(&session.worktree_path);

    // Standard per-session bootstrap: .galvolite dir, CLAUDE-session.md, settings.json, MCP json, api token.
    let galvolite_dir = wt_pathbuf.join(".galvolite");
    std::fs::create_dir_all(&galvolite_dir).map_err(|e| Error::io(&galvolite_dir, e))?;
    let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
    std::fs::write(&claude_md_path, crate::claude_md::profile_for_role(role))
        .map_err(|e| Error::io(&claude_md_path, e))?;
    let _ = crate::handoff::ensure_session_dir(
        &crate::handoff::HandoffDir::new(galvolite_dir.join("handoffs")),
        session.id,
    );

    let hook_bin = crate::spawn_config::resolve_hook_binary()
        .map_err(|e| Error::io(std::path::PathBuf::from("galvolite-hook"), e))?;
    crate::spawn_config::write_settings_json(&wt_pathbuf, hook_bin.to_string_lossy().as_ref())
        .map_err(|e| Error::io(wt_pathbuf.join(".claude/settings.json"), e))?;

    let api_token = format!("tok_{}", Uuid::new_v4().simple());
    self.db.set_api_token(session.id, &api_token)?;
    let api_base = crate::spawn_config::resolve_api_base();
    crate::spawn_config::write_mcp_json(&wt_pathbuf, &api_base, &api_token)?;

    // PTY spawn + standard event/cycle wiring (extracted into a helper if it gets repeated;
    // for v1 we mirror spawn_session's PTY block inline). The model invokes the same
    // claude_cmd as parent — resolved via env or settings.
    let claude_cmd = std::env::var("GALVOLITE_CLAUDE_CMD").unwrap_or_else(|_| "claude".to_string());
    let cycle_id = self.db.start_cycle(session.id)?;
    let pty = crate::pty::PtyHandle::spawn(crate::pty::PtyOpts {
        cwd: wt_pathbuf.clone(),
        command: claude_cmd,
        args: vec![],
        env: vec![],
    })?;
    self.db.update_session_pid(session.id, pty.pid as i64)?;

    // Apply mcp.spawn_session on PARENT (audit trail) and system.session_started on CHILD.
    let _ = crate::state::apply_event_with_bus(
        &self.db, parent_id, &self.event_bus,
        crate::state::EventInput {
            source: crate::state::Source::Mcp,
            kind: "mcp.spawn_session".into(),
            payload: serde_json::json!({ "child_id": session.id, "role": role, "branch": branch }),
            dedup_key: Some(format!("spawn:{}:{}", parent_id, session.id)),
        },
    );
    crate::state::apply_event_with_bus(
        &self.db, session.id, &self.event_bus,
        crate::state::EventInput {
            source: crate::state::Source::System,
            kind: "system.session_started".into(),
            payload: serde_json::json!({ "role": role, "parent": parent_id }),
            dedup_key: Some(format!("session-started:{}", session.id)),
        },
    )?;

    // Register running and start the PTY-exit reactor (mirrors spawn_session).
    let handle = pty.shared();
    let scrollback = pty.scrollback.clone();
    let cycle_slot = std::sync::Arc::new(std::sync::Mutex::new(Some(cycle_id)));
    let fx = std::sync::Arc::new(crate::ritual::PtySideEffects::new(handle.clone()));
    let driver = std::sync::Arc::new(crate::ritual::RitualDriver::new(fx));
    let handoff_dir = crate::handoff::HandoffDir::new(galvolite_dir.join("handoffs"));
    {
        let mut running = self.running.lock().unwrap();
        running.insert(session.id, RunningSession {
            driver: driver.clone(),
            handle: handle.clone(),
            scrollback,
            cycle_id: cycle_slot.clone(),
            handoff_dir: handoff_dir.clone(),
        });
    }
    let mgr = std::sync::Arc::clone(self);
    let session_id = session.id;
    let handoff_dir_bg = handoff_dir.clone();
    let cycle_slot_bg = cycle_slot.clone();
    tokio::spawn(async move {
        let mut owner = pty;
        owner.wait_exit().await;
        let prev_cycle = cycle_slot_bg.lock().unwrap().take();
        if let Some(c) = prev_cycle {
            let h = crate::handoff::latest_handoff_for(&handoff_dir_bg, session_id)
                .ok().flatten().map(|p| p.to_string_lossy().into_owned());
            let _ = mgr.db.end_cycle_if_unended(c, h.as_deref());
        }
        let _ = crate::state::apply_event_with_bus(
            &mgr.db, session_id, &mgr.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "system.session_ended".into(),
                payload: serde_json::json!({ "reason": "pty_exit" }),
                dedup_key: Some(format!("session-ended:{session_id}")),
            },
        );
        // Hand-off to on_child_terminal — it's idempotent.
        let _ = mgr.on_child_terminal(session_id, "ended").await;
        mgr.running.lock().unwrap().remove(&session_id);
    });

    Ok(session)
}
```

Add `db::Db::update_worktree_path(&self, sid: Uuid, path: &str) -> Result<()>` next to other session updates in `src/db.rs`:

```rust
pub fn update_worktree_path(&self, sid: Uuid, path: &str) -> Result<()> {
    let conn = self.pool.get()?;
    conn.execute(
        "UPDATE sessions SET worktree_path = ?1 WHERE id = ?2",
        rusqlite::params![path, sid.to_string()],
    )?;
    Ok(())
}
```

Add `claude_md::profile_for_role(role)` in `src/claude_md.rs`:

```rust
pub fn profile_for_role(role: &str) -> &'static str {
    match role {
        "implementation" | "fix" => implementation_profile(),
        "brainstorm" => brainstorm_profile(),
        "scout" => scout_profile(),
        "orchestrator" => orchestrator_profile(),
        _ => implementation_profile(),
    }
}
```

(If `brainstorm_profile`, `scout_profile`, `orchestrator_profile` don't exist yet, route all non-impl roles to `implementation_profile()` for v1 — the orchestrator/brainstorm extensions land in Task 15.)

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test --test spawn_session_mcp spawn_child_creates_worktree_and_db_row_with_parent
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs src/db.rs src/claude_md.rs tests/spawn_session_mcp.rs
git commit -m "feat(session): SessionManager::spawn_child with git worktree isolation"
```

---

## Task 5: `set_waiting_on` + `build_rejoin_markdown`

**Files:**
- Modify: `src/session_manager.rs`, `src/db.rs`
- Test: `tests/spawn_session_mcp.rs` (extend)

- [ ] **Step 1: Write the failing tests**

Append to `tests/spawn_session_mcp.rs`:

```rust
#[tokio::test]
async fn set_waiting_on_persists_json_array() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();

    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let waiting: String = conn.query_row(
        "SELECT waiting_on FROM sessions WHERE id = ?1",
        [parent.id.to_string()],
        |r| r.get(0)
    ).unwrap();
    let parsed: Vec<String> = serde_json::from_str(&waiting).unwrap();
    assert_eq!(parsed.len(), 2);
    assert!(parsed.contains(&c1.id.to_string()));
    assert!(parsed.contains(&c2.id.to_string()));
}

#[tokio::test]
async fn build_rejoin_markdown_lists_each_child() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    // Force terminal_status so build_rejoin sees a value.
    let conn = app.db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET terminal_status = 'complete', handoff_path = '/tmp/h.md' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    drop(conn);

    let md = app.mgr.build_rejoin_markdown(parent.id, &[c.id]).await.unwrap();
    assert!(md.starts_with("Children resolved. Resume."));
    assert!(md.contains(&c.id.simple().to_string()[..8]));
    assert!(md.contains("status=complete"));
    assert!(md.contains("handoff=/tmp/h.md"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test spawn_session_mcp set_waiting_on build_rejoin_markdown
```

Expected: FAIL — methods not defined.

- [ ] **Step 3: Implement the methods**

In `src/session_manager.rs` (inside `impl SessionManager`):

```rust
pub async fn set_waiting_on(&self, parent_id: Uuid, children: &[Uuid]) -> Result<()> {
    // Validate every child is owned by this parent.
    let conn = self.db.pool().get()?;
    for c in children {
        let pid: Option<String> = conn.query_row(
            "SELECT parent_session_id FROM sessions WHERE id = ?1",
            [c.to_string()], |r| r.get(0),
        ).optional()?;
        match pid {
            Some(s) if s == parent_id.to_string() => {}
            _ => return Err(Error::Config(format!("child {c} not owned by parent {parent_id}"))),
        }
    }
    let json = serde_json::to_string(&children.iter().map(|u| u.to_string()).collect::<Vec<_>>())?;
    conn.execute(
        "UPDATE sessions SET waiting_on = ?1 WHERE id = ?2",
        rusqlite::params![json, parent_id.to_string()],
    )?;
    let _ = crate::state::apply_event_with_bus(
        &self.db, parent_id, &self.event_bus,
        crate::state::EventInput {
            source: crate::state::Source::Mcp,
            kind: "mcp.wait_children".into(),
            payload: serde_json::json!({ "children": children.iter().map(|u| u.to_string()).collect::<Vec<_>>() }),
            dedup_key: None,
        },
    );
    Ok(())
}

pub async fn build_rejoin_markdown(&self, parent_id: Uuid, children: &[Uuid]) -> Result<String> {
    let conn = self.db.pool().get()?;
    let mut lines = Vec::with_capacity(children.len() + 2);
    lines.push("Children resolved. Resume.".to_string());
    lines.push(String::new());
    for cid in children {
        let row: Option<(String, Option<String>, Option<String>)> = conn.query_row(
            "SELECT role, terminal_status, handoff_path FROM sessions \
             WHERE id = ?1 AND parent_session_id = ?2",
            rusqlite::params![cid.to_string(), parent_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional()?;
        let Some((role, status, handoff)) = row else { continue };
        let short = &cid.simple().to_string()[..8];
        let status_s = status.as_deref().unwrap_or("unknown");
        let handoff_s = handoff.as_deref().unwrap_or("none");
        lines.push(format!("- {short} role={role} status={status_s} handoff={handoff_s}"));
    }
    Ok(lines.join("\n"))
}
```

(Requires `use rusqlite::OptionalExtension;` at the top of `session_manager.rs` if not already imported.)

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test spawn_session_mcp set_waiting_on build_rejoin_markdown
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs tests/spawn_session_mcp.rs
git commit -m "feat(session): set_waiting_on + build_rejoin_markdown"
```

---

## Task 6: `on_child_terminal` reactor

**Files:**
- Modify: `src/session_manager.rs`
- Test: `tests/spawn_session_mcp.rs` (extend)

- [ ] **Step 1: Write the failing tests**

Append to `tests/spawn_session_mcp.rs`:

```rust
#[tokio::test]
async fn on_child_terminal_clears_one_of_two_children() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "a").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "b").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    app.mgr.on_child_terminal(c1.id, "complete").await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let (waiting, pending): (Option<String>, Option<String>) = conn.query_row(
        "SELECT waiting_on, pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| Ok((r.get(0)?, r.get(1)?))
    ).unwrap();
    let arr: Vec<String> = serde_json::from_str(waiting.as_deref().unwrap()).unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr.contains(&c2.id.to_string()));
    assert!(pending.is_none(), "no rejoin yet — c2 still pending");
}

#[tokio::test]
async fn on_child_terminal_writes_pending_rejoin_when_last() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();

    app.mgr.on_child_terminal(c.id, "complete").await.unwrap();

    let conn = app.db.pool().get().unwrap();
    let (waiting, pending, term_status): (Option<String>, Option<String>, Option<String>) =
        conn.query_row(
            "SELECT s.waiting_on, s.pending_rejoin, c.terminal_status \
             FROM sessions s JOIN sessions c ON c.id = ?2 WHERE s.id = ?1",
            [parent.id.to_string(), c.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
    assert!(waiting.is_none(), "waiting_on cleared");
    assert!(pending.as_deref().unwrap().contains("Children resolved"));
    assert_eq!(term_status.as_deref(), Some("complete"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test spawn_session_mcp on_child_terminal
```

Expected: FAIL — `on_child_terminal` not defined.

- [ ] **Step 3: Implement `on_child_terminal`**

```rust
/// Called from three sites: mcp.status_report (kind=complete|blocked) side-effect,
/// PTY exit handler (status=ended|error), and recovery startup pass.
/// Idempotent: writing the same terminal_status twice is a no-op.
pub async fn on_child_terminal(&self, child_id: Uuid, status: &str) -> Result<()> {
    let mut conn = self.db.pool().get()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;

    // 1. Stamp child's terminal_status (first writer wins).
    tx.execute(
        "UPDATE sessions SET terminal_status = COALESCE(terminal_status, ?1) WHERE id = ?2",
        rusqlite::params![status, child_id.to_string()],
    )?;

    // 2. Find every parent observing this child.
    let mut stmt = tx.prepare(
        "SELECT id, waiting_on FROM sessions WHERE waiting_on LIKE ?1"
    )?;
    let pattern = format!("%{}%", child_id);
    let parents: Vec<(String, String)> = stmt
        .query_map([pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    let mut newly_resolved: Vec<(Uuid, Vec<Uuid>)> = Vec::new();

    for (parent_id_str, waiting_json) in parents {
        let parent_id = Uuid::parse_str(&parent_id_str)
            .map_err(|e| Error::Config(format!("bad parent uuid: {e}")))?;
        let mut arr: Vec<String> = serde_json::from_str(&waiting_json).unwrap_or_default();
        let before = arr.len();
        arr.retain(|s| s != &child_id.to_string());
        if arr.len() == before { continue; }

        if arr.is_empty() {
            // All children resolved — clear waiting_on, populate pending_rejoin.
            // Capture the child set BEFORE clearing for the markdown query.
            let original: Vec<Uuid> = serde_json::from_str::<Vec<String>>(&waiting_json)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|s| Uuid::parse_str(&s).ok())
                .collect();
            tx.execute(
                "UPDATE sessions SET waiting_on = NULL WHERE id = ?1",
                [parent_id_str],
            )?;
            newly_resolved.push((parent_id, original));
        } else {
            tx.execute(
                "UPDATE sessions SET waiting_on = ?1 WHERE id = ?2",
                rusqlite::params![serde_json::to_string(&arr).unwrap(), parent_id_str],
            )?;
        }
    }
    tx.commit()?;

    // 3. Outside the transaction: build markdown, write pending_rejoin, fire events,
    //    nudge PTY. Each parent independent.
    for (parent_id, child_set) in newly_resolved {
        let md = self.build_rejoin_markdown(parent_id, &child_set).await?;
        let conn = self.db.pool().get()?;
        conn.execute(
            "UPDATE sessions SET pending_rejoin = ?1 WHERE id = ?2",
            rusqlite::params![md, parent_id.to_string()],
        )?;
        let _ = crate::state::apply_event_with_bus(
            &self.db, parent_id, &self.event_bus,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "session.children_resolved".into(),
                payload: serde_json::json!({
                    "children": child_set.iter().map(|u| u.to_string()).collect::<Vec<_>>()
                }),
                dedup_key: Some(format!("children-resolved:{parent_id}")),
            },
        );
        // Wake the parent's PTY if it is alive.
        let marker = format!("[GALVOLITE:REJOIN {}]\n", parent_id);
        let running_handle = {
            let running = self.running.lock().unwrap();
            running.get(&parent_id).map(|rs| rs.handle.clone())
        };
        if let Some(h) = running_handle {
            let _ = h.write_str(&marker).await;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test spawn_session_mcp on_child_terminal
```

Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs tests/spawn_session_mcp.rs
git commit -m "feat(session): on_child_terminal reactor (atomic waiting_on mutation, marker dispatch)"
```

---

## Task 7: New event kinds in `state.rs` and `events.rs`

**Files:**
- Modify: `src/state.rs`, `src/api/events.rs`
- Test: `tests/spawn_event_kinds.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/spawn_event_kinds.rs
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::state::{apply_event, EventInput, Source};
use serde_json::json;
use tempfile::TempDir;

fn fresh_session() -> (Db, TempDir, uuid::Uuid) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let p = db.insert_project(NewProject{
        name:"p".into(), path: tmp.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let s = db.insert_session(NewSession{
        project_id: p.id, role:"implementation".into(), branch: None,
        worktree_path: tmp.path().to_string_lossy().into_owned(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    (db, tmp, s.id)
}

#[test]
fn mcp_spawn_session_event_appends_without_state_change() {
    let (db, _t, sid) = fresh_session();
    let r = apply_event(&db, sid, EventInput{
        source: Source::Mcp,
        kind: "mcp.spawn_session".into(),
        payload: json!({"child_id":"x","role":"implementation"}),
        dedup_key: None,
    }).unwrap();
    assert!(!r.deduped);
    // No state change — kind is logging-only.
}

#[test]
fn session_children_resolved_appends_and_publishes_domain_event() {
    let (db, _t, sid) = fresh_session();
    let _ = apply_event(&db, sid, EventInput{
        source: Source::System,
        kind: "session.children_resolved".into(),
        payload: json!({"children":["a","b"]}),
        dedup_key: None,
    }).unwrap();
    // Row appended in events table.
    let conn = db.pool().get().unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM events WHERE session_id = ?1 AND kind = 'session.children_resolved'",
        [sid.to_string()], |r| r.get(0),
    ).unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test spawn_event_kinds
```

Expected: FAIL — `apply_event` rejects unknown kind, OR no DomainEvent variant defined.

- [ ] **Step 3: Register the kinds in `src/state.rs`**

Find the kind-classification logic (`apply_event`) — most likely an exhaustive match or `transition()` helper. Add the three new kinds as logging-only. If `transition()` rejects unknown kinds, add the new kinds to its allow-list. Open `src/state.rs` and locate the `transition` function:

```rust
fn transition(current: SessionState, kind: &str) -> SessionState {
    // ... existing arms
    // Add new arms:
    if matches!(kind, "mcp.spawn_session" | "mcp.wait_children" | "session.children_resolved") {
        return current; // logging-only, no state change
    }
    // ... existing default
}
```

(If `transition` does not match unknown kinds explicitly today, find the actual classification site and ensure these three kinds pass through without warnings — see how `mcp.notify_operator` is handled for a pattern.)

- [ ] **Step 4: Add `SessionChildrenResolved` variant to `DomainEvent`**

Edit `src/api/events.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainEvent {
    // ... existing variants
    #[serde(rename = "session.children_resolved")]
    SessionChildrenResolved {
        session_id: Uuid,
        children: Vec<ChildResolution>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ChildResolution {
    pub id: Uuid,
    pub role: String,
    pub status: String,
    pub handoff: Option<String>,
}
```

In the EventBroadcaster fan-out where `session.children_resolved` events are translated into DomainEvent (likely a match in `apply_event_with_bus` at the bottom of `state.rs`), add an arm that selects the child rows and emits the variant:

```rust
"session.children_resolved" => {
    if let Some(children) = input.payload.get("children").and_then(|v| v.as_array()) {
        let conn = db.pool().get().ok();
        let mut payload_children = Vec::new();
        if let Some(conn) = conn {
            for c in children {
                if let Some(cid) = c.as_str().and_then(|s| Uuid::parse_str(s).ok()) {
                    if let Ok((role, status, handoff)) = conn.query_row(
                        "SELECT role, terminal_status, handoff_path FROM sessions WHERE id = ?1",
                        [cid.to_string()],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)),
                    ) {
                        payload_children.push(ChildResolution {
                            id: cid, role,
                            status: status.unwrap_or_else(|| "unknown".into()),
                            handoff,
                        });
                    }
                }
            }
        }
        bus.publish(DomainEvent::SessionChildrenResolved {
            session_id, children: payload_children,
        });
    }
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo test --test spawn_event_kinds
cargo test  # verify no regressions
```

Expected: PASS, no regressions.

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/api/events.rs tests/spawn_event_kinds.rs
git commit -m "feat(events): mcp.spawn_session, mcp.wait_children, session.children_resolved"
```

---

## Task 8: Wire `on_child_terminal` into status_report side-effect

**Files:**
- Modify: `src/state.rs` (extend `apply_column_side_effects` for `mcp.status_report`)
- Test: `tests/spawn_session_mcp.rs` (extend)

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn status_report_complete_triggers_on_child_terminal_via_state() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();

    // Simulate the child reporting complete via the same path mcp tools use.
    galvolite::state::apply_event_with_bus(
        &app.db, c.id, app.mgr.event_bus(),
        galvolite::state::EventInput {
            source: galvolite::state::Source::Mcp,
            kind: "mcp.status_report".into(),
            payload: serde_json::json!({"kind":"complete"}),
            dedup_key: None,
        }
    ).unwrap();

    // Allow the spawned task in apply_event_with_bus to run.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let conn = app.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test spawn_session_mcp status_report_complete_triggers
```

Expected: FAIL — `mcp.status_report` does not currently invoke `on_child_terminal`.

- [ ] **Step 3: Wire the reactor**

The cleanest hook point is right after `apply_event_with_bus` commits the `mcp.status_report` row. Since `apply_event_with_bus` is called from MCP tools and lives below `SessionManager`, route via the bus instead: add a subscriber inside `SessionManager::with_event_bus`. Add to `SessionManager::with_event_bus` constructor:

```rust
pub fn with_event_bus(db: Db, data_dir: PathBuf, event_bus: EventBroadcaster) -> Self {
    let mgr = Self { /* existing init */ };
    let me = Arc::new(mgr);
    // The Arc<Self> we want to spawn into the reactor must come from outside;
    // do this in a separate `start_reactor` method called from main.rs after Arc::new.
    me_inner_unwrap_or_clone_pattern_here
}
```

Better approach: add an explicit `start_terminal_reactor` method:

```rust
pub fn start_terminal_reactor(self: &Arc<Self>) {
    let mgr = Arc::clone(self);
    let mut rx = self.event_bus.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if let DomainEvent::SessionEventAppended { session_id, kind, payload, .. } = ev {
                match kind.as_str() {
                    "mcp.status_report" => {
                        if let Some(k) = payload.get("kind").and_then(|v| v.as_str()) {
                            if k == "complete" || k == "blocked" {
                                let _ = mgr.on_child_terminal(session_id, k).await;
                            }
                        }
                    }
                    "system.session_ended" => {
                        let reason = payload.get("reason").and_then(|v| v.as_str()).unwrap_or("ended");
                        let status = if reason == "operator_kill" { "error" } else { "ended" };
                        let _ = mgr.on_child_terminal(session_id, status).await;
                    }
                    _ => {}
                }
            }
        }
    });
}
```

Then in `src/main.rs` (or wherever `SessionManager` is constructed for the live server), call `mgr.start_terminal_reactor()` once after `Arc::new(...)`. Update `tests/common/mod.rs::test_app_with_bus` to also call it so tests exercise the reactor:

```rust
pub fn test_app_with_bus() -> TestApp {
    // existing setup...
    let mgr = Arc::new(SessionManager::with_event_bus(/*...*/));
    mgr.start_terminal_reactor();
    // existing return...
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test --test spawn_session_mcp status_report_complete_triggers
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs src/main.rs tests/common/mod.rs tests/spawn_session_mcp.rs
git commit -m "feat(session): bus-driven terminal reactor wires on_child_terminal"
```

---

## Task 9: Recovery extension — re-fire reactor on startup

**Files:**
- Modify: `src/session_manager.rs::recover_non_terminal_sessions` (or sibling fn)
- Test: `tests/spawn_recovery.rs` (new)

- [ ] **Step 1: Write the failing test**

```rust
// tests/spawn_recovery.rs
mod common;
use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;

fn init_git(d: &std::path::Path) {
    Command::new("git").args(["init","-q","-b","main"]).current_dir(d).status().unwrap();
    std::fs::write(d.join("README"),"x").unwrap();
    Command::new("git").args(["add","."]).current_dir(d).status().unwrap();
    Command::new("git")
        .args(["-c","user.email=t@t","-c","user.name=t","commit","-qm","i"])
        .current_dir(d).status().unwrap();
}

#[tokio::test]
async fn recovery_resolves_already_terminal_children() {
    // First app: spawn parent + child, mark child terminal, but DON'T let reactor run.
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let app1 = common::test_app_with_bus_at(dir.path()); // helper: same DB path
    let proj = app1.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app1.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app1.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    app1.mgr.set_waiting_on(parent.id, &[c.id]).await.unwrap();
    // Stamp terminal_status directly — simulate the reactor missing the event.
    app1.db.pool().get().unwrap().execute(
        "UPDATE sessions SET terminal_status = 'complete' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    drop(app1);

    // Re-open at the same DB path: recovery pass must turn waiting_on into pending_rejoin.
    let app2 = common::reopen_app_with_bus_at(dir.path());
    app2.mgr.recover_pending_rejoins().await.unwrap();

    let conn = app2.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
}
```

(Add `test_app_with_bus_at(path)` and `reopen_app_with_bus_at(path)` helpers in `tests/common/mod.rs` — both open `Db` at the supplied path so the second instance sees the first's state.)

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test --test spawn_recovery
```

Expected: FAIL — `recover_pending_rejoins` not defined.

- [ ] **Step 3: Implement the recovery pass**

In `src/session_manager.rs`:

```rust
/// Restart-time scan: any parent with a non-NULL `waiting_on` whose listed
/// children already have terminal_status set should resolve immediately.
pub async fn recover_pending_rejoins(&self) -> Result<()> {
    let conn = self.db.pool().get()?;
    let mut stmt = conn.prepare(
        "SELECT id, waiting_on FROM sessions WHERE waiting_on IS NOT NULL"
    )?;
    let parents: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);
    for (_pid, waiting_json) in parents {
        let arr: Vec<String> = serde_json::from_str(&waiting_json).unwrap_or_default();
        for child_str in arr {
            let cid = match Uuid::parse_str(&child_str) { Ok(u) => u, Err(_) => continue };
            let term: Option<String> = conn.query_row(
                "SELECT terminal_status FROM sessions WHERE id = ?1",
                [child_str.as_str()], |r| r.get(0),
            ).optional()?.flatten();
            if let Some(status) = term {
                self.on_child_terminal(cid, &status).await?;
            }
        }
    }
    Ok(())
}
```

Call it from the existing recovery boot path — find `recover_non_terminal_sessions` call site in `src/main.rs`, add right after:

```rust
let _ = mgr.recover_pending_rejoins().await;
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test --test spawn_recovery
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs src/main.rs tests/common/mod.rs tests/spawn_recovery.rs
git commit -m "feat(session): recover_pending_rejoins replays terminal children at startup"
```

---

## Task 10: MCP tool `spawn_session`

**Files:**
- Create: `src/mcp/tools/spawn.rs`
- Modify: `src/mcp/tools/mod.rs` (register)
- Test: `tests/spawn_session_mcp.rs` (extend with MCP-level test)

- [ ] **Step 1: Write the failing test**

Append to `tests/spawn_session_mcp.rs`:

```rust
#[tokio::test]
async fn mcp_spawn_session_returns_child_descriptor() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "orchestrator".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token = app.db.read_api_token(parent.id).unwrap();

    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"spawn_session","arguments":{"role":"implementation","task":"refactor auth"}}
    });
    let resp = common::rpc(app.app.clone(), &token, body).await;
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(inner["session_id"].as_str().is_some());
    assert!(inner["worktree_path"].as_str().unwrap().contains(".galvolite/worktrees"));
    assert!(inner["branch"].as_str().unwrap().starts_with("galvolite/"));
}

#[tokio::test]
async fn mcp_spawn_session_rejects_unknown_role() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "implementation".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token = app.db.read_api_token(parent.id).unwrap();

    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"spawn_session","arguments":{"role":"president","task":"x"}}
    });
    let resp = common::rpc(app.app, &token, body).await;
    // Error path: tool returns InvalidParams; transport surfaces it as result.isError=true.
    assert!(resp["result"]["isError"].as_bool().unwrap_or(false));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test spawn_session_mcp mcp_spawn_session
```

Expected: FAIL — tool unregistered.

- [ ] **Step 3: Create `src/mcp/tools/spawn.rs`**

```rust
//! Sub-plan 6 MCP tools: spawn_session, wait_children. See spec §3.1.

use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use serde::Deserialize;
use serde_json::{json, Value};

const ALLOWED_ROLES: &[&str] = &["implementation", "brainstorm", "scout", "fix", "orchestrator"];
const SOFT_CAP_CONCURRENCY: i64 = 4;
const SOFT_CAP_DEPTH: i64 = 3;

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "spawn_session",
            description: "Spawn a child session in its own git worktree. Returns the child's session_id, worktree_path, and branch.",
            input_schema: json!({
                "type":"object",
                "properties":{
                    "role":{"type":"string","enum":ALLOWED_ROLES},
                    "task":{"type":"string"}
                },
                "required":["role","task"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(spawn_session(ctx, args))),
        },
        ToolDef {
            name: "wait_children",
            description: "Park this session until every listed child reaches terminal status. Returns immediately; rejoin is delivered via additionalContext on the next prompt.",
            input_schema: json!({
                "type":"object",
                "properties":{
                    "children":{"type":"array","items":{"type":"string","format":"uuid"}}
                },
                "required":["children"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(wait_children(ctx, args))),
        },
    ]
}

#[derive(Deserialize)]
struct SpawnArgs { role: String, task: String }

async fn spawn_session(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SpawnArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !ALLOWED_ROLES.contains(&parsed.role.as_str()) {
        return ToolOutcome::InvalidParams(format!("unknown role: {}", parsed.role));
    }

    let child = match ctx.mgr.spawn_child(ctx.session_id, &parsed.role, &parsed.task).await {
        Ok(c) => c,
        Err(e) => return ToolOutcome::Error(e.to_string()),
    };

    // Soft caps — emit warn toasts but proceed.
    if let Ok(active) = active_count_for_project(&ctx.db, child.project_id) {
        if active > SOFT_CAP_CONCURRENCY {
            ctx.mgr.event_bus().publish(crate::api::events::DomainEvent::OperatorToast {
                session_id: ctx.session_id,
                level: "warn".into(),
                message: format!("Project active sessions = {active} (soft cap {SOFT_CAP_CONCURRENCY})"),
            });
        }
    }
    if let Ok(depth) = walk_depth(&ctx.db, child.id) {
        if depth > SOFT_CAP_DEPTH {
            ctx.mgr.event_bus().publish(crate::api::events::DomainEvent::OperatorToast {
                session_id: ctx.session_id,
                level: "warn".into(),
                message: format!("Spawn depth = {depth} (soft cap {SOFT_CAP_DEPTH})"),
            });
        }
    }

    ToolOutcome::Ok(json!({
        "session_id": child.id,
        "worktree_path": child.worktree_path,
        "branch": child.branch,
    }))
}

fn active_count_for_project(db: &crate::db::Db, project_id: uuid::Uuid) -> Result<i64, crate::error::Error> {
    let conn = db.pool().get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE project_id = ?1 AND ended_at IS NULL",
        [project_id.to_string()], |r| r.get(0),
    )?;
    Ok(n)
}

fn walk_depth(db: &crate::db::Db, session_id: uuid::Uuid) -> Result<i64, crate::error::Error> {
    let conn = db.pool().get()?;
    let mut depth = 0;
    let mut current = session_id.to_string();
    while let Some(parent) = conn.query_row(
        "SELECT parent_session_id FROM sessions WHERE id = ?1",
        [current.as_str()],
        |r| r.get::<_, Option<String>>(0),
    ).ok().flatten() {
        depth += 1;
        if depth > 10 { break; } // hard guard
        current = parent;
    }
    Ok(depth)
}

// wait_children stub — full impl in Task 11.
async fn wait_children(_ctx: ToolCtx, _args: Value) -> ToolOutcome {
    ToolOutcome::Error("not implemented yet".into())
}
```

Edit `src/mcp/tools/mod.rs::all`:

```rust
pub fn all() -> Vec<ToolDef> {
    let mut v = Vec::new();
    v.extend(tier1_status::tools());
    v.extend(tier1_role_handoff::tools());
    v.extend(tier1_introspection::tools());
    v.extend(tier2_3_cross::tools());
    v.extend(tier4_stubs::tools());
    v.extend(spawn::tools());  // NEW
    v
}
```

Add `mod spawn;` at the top.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test spawn_session_mcp mcp_spawn_session
```

Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/mcp/tools/spawn.rs src/mcp/tools/mod.rs tests/spawn_session_mcp.rs
git commit -m "feat(mcp): spawn_session tool with soft-cap warn toasts"
```

---

## Task 11: MCP tool `wait_children`

**Files:**
- Modify: `src/mcp/tools/spawn.rs`
- Test: `tests/spawn_session_mcp.rs` (extend)

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn mcp_wait_children_records_waiting_on() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "orchestrator".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    let token = app.db.read_api_token(parent.id).unwrap();

    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"wait_children","arguments":{"children":[c.id.to_string()]}}
    });
    let resp = common::rpc(app.app, &token, body).await;
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["ok"], true);
    assert_eq!(inner["immediate_rejoin"], false);

    let conn = app.db.pool().get().unwrap();
    let waiting: String = conn.query_row(
        "SELECT waiting_on FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(waiting.contains(&c.id.to_string()));
}

#[tokio::test]
async fn mcp_wait_children_short_circuits_on_already_terminal() {
    let app = common::test_app_with_bus();
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: proj_dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "orchestrator".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let c = app.mgr.spawn_child(parent.id, "implementation", "x").await.unwrap();
    // Mark child terminal up front.
    app.db.pool().get().unwrap().execute(
        "UPDATE sessions SET terminal_status = 'complete' WHERE id = ?1",
        [c.id.to_string()],
    ).unwrap();
    let token = app.db.read_api_token(parent.id).unwrap();

    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"wait_children","arguments":{"children":[c.id.to_string()]}}
    });
    let resp = common::rpc(app.app, &token, body).await;
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let inner: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["immediate_rejoin"], true);

    let conn = app.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test spawn_session_mcp mcp_wait_children
```

Expected: FAIL — `wait_children` returns "not implemented".

- [ ] **Step 3: Implement `wait_children`**

Replace the stub in `src/mcp/tools/spawn.rs`:

```rust
#[derive(Deserialize)]
struct WaitArgs { children: Vec<String> }

async fn wait_children(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: WaitArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let mut child_ids = Vec::with_capacity(parsed.children.len());
    for s in &parsed.children {
        match uuid::Uuid::parse_str(s) {
            Ok(u) => child_ids.push(u),
            Err(_) => return ToolOutcome::InvalidParams(format!("not a uuid: {s}")),
        }
    }

    // Validate ownership + check whether all are already terminal.
    let conn = match ctx.db.pool().get() {
        Ok(c) => c,
        Err(e) => return ToolOutcome::Error(e.to_string()),
    };
    let mut all_terminal = true;
    for cid in &child_ids {
        let row: Option<(Option<String>, Option<String>)> = conn.query_row(
            "SELECT parent_session_id, terminal_status FROM sessions WHERE id = ?1",
            [cid.to_string()], |r| Ok((r.get(0)?, r.get(1)?)),
        ).ok();
        let Some((parent_str, term)) = row else {
            return ToolOutcome::Error(format!("unknown_child: {cid}"));
        };
        if parent_str.as_deref() != Some(&ctx.session_id.to_string()) {
            return ToolOutcome::Error(format!("not_owned_child: {cid}"));
        }
        if term.is_none() {
            all_terminal = false;
        }
    }
    drop(conn);

    if all_terminal {
        // Short-circuit: build markdown + write pending_rejoin synchronously.
        let md = match ctx.mgr.build_rejoin_markdown(ctx.session_id, &child_ids).await {
            Ok(m) => m,
            Err(e) => return ToolOutcome::Error(e.to_string()),
        };
        let conn = ctx.db.pool().get().unwrap();
        let _ = conn.execute(
            "UPDATE sessions SET pending_rejoin = ?1 WHERE id = ?2",
            rusqlite::params![md, ctx.session_id.to_string()],
        );
        return ToolOutcome::Ok(json!({"ok":true,"immediate_rejoin":true}));
    }

    if let Err(e) = ctx.mgr.set_waiting_on(ctx.session_id, &child_ids).await {
        return ToolOutcome::Error(e.to_string());
    }
    ToolOutcome::Ok(json!({"ok":true,"immediate_rejoin":false}))
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test spawn_session_mcp mcp_wait_children
```

Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/mcp/tools/spawn.rs tests/spawn_session_mcp.rs
git commit -m "feat(mcp): wait_children tool (parks parent; short-circuits when all terminal)"
```

---

## Task 12: API endpoint `GET /api/sessions/:id/pending-rejoin`

**Files:**
- Create: `src/api/spawn.rs`
- Modify: `src/api/mod.rs` (route registration + module)
- Test: `tests/pending_rejoin_endpoint.rs` (new)

- [ ] **Step 1: Write the failing tests**

```rust
// tests/pending_rejoin_endpoint.rs
mod common;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use std::process::Command;
use tempfile::TempDir;
use tower::ServiceExt;
use galvolite::db::NewProject;

fn init_git(d: &std::path::Path) {
    Command::new("git").args(["init","-q","-b","main"]).current_dir(d).status().unwrap();
    std::fs::write(d.join("README"),"x").unwrap();
    Command::new("git").args(["add","."]).current_dir(d).status().unwrap();
    Command::new("git").args(["-c","user.email=t@t","-c","user.name=t","commit","-qm","i"])
        .current_dir(d).status().unwrap();
}

#[tokio::test]
async fn pending_rejoin_returns_markdown_and_clears_atomically() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();
    let parent = app.mgr.spawn_session(
        proj.id, "p".into(), "orchestrator".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();
    let token = app.db.read_api_token(parent.id).unwrap();
    // Plant a pending_rejoin manually.
    app.db.pool().get().unwrap().execute(
        "UPDATE sessions SET pending_rejoin = 'Children resolved. Resume.' WHERE id = ?1",
        [parent.id.to_string()],
    ).unwrap();

    // First GET returns markdown.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", parent.id))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap();
    let resp = app.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 4096).await.unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(parsed["markdown"].as_str().unwrap().contains("Children resolved"));

    // Second GET returns 404 — atomically cleared.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", parent.id))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pending_rejoin_unauthorized_without_token() {
    let app = common::test_app();
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", uuid::Uuid::new_v4()))
        .body(Body::empty()).unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test pending_rejoin_endpoint
```

Expected: FAIL — route does not exist (404).

- [ ] **Step 3: Implement the handler**

Create `src/api/spawn.rs`:

```rust
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

type AppState = Arc<crate::session_manager::SessionManager>;

pub async fn get_pending_rejoin(
    State(mgr): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<Uuid>,
) -> Response {
    // Bearer auth via per-session token.
    let token = match headers.get("Authorization").and_then(|h| h.to_str().ok()) {
        Some(s) if s.starts_with("Bearer ") => s.trim_start_matches("Bearer ").to_string(),
        _ => return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response(),
    };
    let resolved = match crate::api::auth::resolve_token(mgr.db(), &token) {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, "bad token").into_response(),
    };
    if resolved != session_id {
        return (StatusCode::FORBIDDEN, "session_id mismatch").into_response();
    }

    // Atomic clear via RETURNING.
    let conn = match mgr.db().pool().get() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("db: {e}")).into_response(),
    };
    let cleared: Option<String> = conn.query_row(
        "UPDATE sessions SET pending_rejoin = NULL \
         WHERE id = ?1 AND pending_rejoin IS NOT NULL \
         RETURNING (SELECT pending_rejoin FROM sessions WHERE id = ?1)",
        [session_id.to_string()], |r| r.get(0),
    ).ok();

    let Some(markdown) = cleared else {
        return (StatusCode::NOT_FOUND, "no pending rejoin").into_response();
    };

    // Worktree GC: any complete child of this parent gets its worktree removed (idempotent).
    let mut gc_stmt = conn.prepare(
        "SELECT worktree_path FROM sessions \
         WHERE parent_session_id = ?1 AND terminal_status = 'complete'"
    ).ok();
    if let Some(stmt) = gc_stmt.as_mut() {
        let paths: Vec<String> = stmt
            .query_map([session_id.to_string()], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        for p in paths {
            let _ = crate::worktree::remove_worktree(std::path::Path::new(&p));
        }
    }

    Json(json!({"markdown": markdown})).into_response()
}
```

If `src/api/auth.rs` does not exist, factor `resolve_token` from `src/api/hooks.rs` into a shared module and re-export. Otherwise inline a small clone.

Edit `src/api/mod.rs`:

```rust
pub mod spawn;
// ...
.route("/api/sessions/:id/pending-rejoin", get(spawn::get_pending_rejoin))
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test pending_rejoin_endpoint
```

Expected: 2 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/api/spawn.rs src/api/mod.rs tests/pending_rejoin_endpoint.rs
git commit -m "feat(api): GET /api/sessions/:id/pending-rejoin (atomic clear + worktree gc)"
```

---

## Task 13: `src/hooks/marker.rs` — pattern detection

**Files:**
- Create: `src/hooks/marker.rs`
- Modify: `src/hooks/mod.rs` (`pub mod marker;`)
- Test: `tests/marker_pattern.rs` (new)

- [ ] **Step 1: Write the failing tests**

```rust
// tests/marker_pattern.rs
use galvolite::hooks::marker::detect_rejoin_marker;

#[test]
fn detects_well_formed_marker() {
    let id = "0a1b2c3d-4e5f-6789-abcd-ef0123456789";
    let s = format!("[GALVOLITE:REJOIN {}]", id);
    assert_eq!(detect_rejoin_marker(&s).map(|u| u.to_string()), Some(id.into()));
}

#[test]
fn ignores_unrelated_text() {
    assert!(detect_rejoin_marker("hello world").is_none());
    assert!(detect_rejoin_marker("[GALVOLITE:REJOIN not-a-uuid]").is_none());
    assert!(detect_rejoin_marker("[OTHER:REJOIN 0a1b2c3d-4e5f-6789-abcd-ef0123456789]").is_none());
}

#[test]
fn allows_trailing_whitespace() {
    let id = "0a1b2c3d-4e5f-6789-abcd-ef0123456789";
    let s = format!("[GALVOLITE:REJOIN {}]   \n", id);
    assert_eq!(detect_rejoin_marker(&s).map(|u| u.to_string()), Some(id.into()));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test marker_pattern
```

Expected: FAIL — module missing.

- [ ] **Step 3: Implement `src/hooks/marker.rs`**

```rust
//! Rejoin marker detection. The harness emits one of these to wake a parent
//! whose children have all reached terminal status. The galvolite-hook
//! binary recognises the marker and replaces it with the authoritative
//! markdown payload fetched from /api/sessions/:id/pending-rejoin.

use uuid::Uuid;

/// Returns the parent session id if the trimmed prompt is exactly the marker.
pub fn detect_rejoin_marker(prompt: &str) -> Option<Uuid> {
    let trimmed = prompt.trim();
    let prefix = "[GALVOLITE:REJOIN ";
    let suffix = "]";
    let inner = trimmed.strip_prefix(prefix)?.strip_suffix(suffix)?;
    Uuid::parse_str(inner).ok()
}
```

Edit `src/hooks/mod.rs`:

```rust
pub mod marker;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test marker_pattern
```

Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/marker.rs src/hooks/mod.rs tests/marker_pattern.rs
git commit -m "feat(hooks): rejoin marker pattern detection"
```

---

## Task 14: `galvolite-hook` binary intercepts marker

**Files:**
- Modify: `src/bin/galvolite-hook.rs`
- Modify: `src/hooks/client.rs` (add `fetch_pending_rejoin`)
- Test: `tests/hook_binary_marker.rs` (new — uses `mockito`)

- [ ] **Step 1: Write the failing test**

```rust
// tests/hook_binary_marker.rs
use galvolite::hooks::client::{fetch_pending_rejoin, ClientEnv};

#[test]
fn fetch_pending_rejoin_returns_markdown_on_200() {
    let mut server = mockito::Server::new();
    let _m = server.mock("GET", "/api/sessions/00000000-0000-0000-0000-000000000001/pending-rejoin")
        .match_header("authorization", "Bearer tok-x")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"markdown":"Children resolved. Resume."}"#)
        .create();

    let env = ClientEnv {
        api_base: server.url(),
        session_id: "00000000-0000-0000-0000-000000000001".into(),
        token: "tok-x".into(),
        timeout_secs: 5,
    };
    let md = fetch_pending_rejoin(&env).unwrap();
    assert_eq!(md.as_deref(), Some("Children resolved. Resume."));
}

#[test]
fn fetch_pending_rejoin_returns_none_on_404() {
    let mut server = mockito::Server::new();
    let _m = server.mock("GET", mockito::Matcher::Any)
        .with_status(404).create();
    let env = ClientEnv {
        api_base: server.url(),
        session_id: "00000000-0000-0000-0000-000000000001".into(),
        token: "tok-x".into(),
        timeout_secs: 5,
    };
    let md = fetch_pending_rejoin(&env).unwrap();
    assert!(md.is_none());
}
```

(Add `mockito` to `[dev-dependencies]` in `Cargo.toml` if not already present.)

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test hook_binary_marker
```

Expected: FAIL — `fetch_pending_rejoin` undefined.

- [ ] **Step 3: Implement `fetch_pending_rejoin`**

Append to `src/hooks/client.rs`:

```rust
/// GET /api/sessions/<id>/pending-rejoin. Returns Some(markdown) on 200,
/// None on 404 (or any non-2xx — fail-soft per existing convention).
pub fn fetch_pending_rejoin(env: &ClientEnv) -> Result<Option<String>, ClientError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(env.timeout_secs))
        .build()?;
    let resp = client
        .get(format!("{}/api/sessions/{}/pending-rejoin", env.api_base, env.session_id))
        .header("Authorization", format!("Bearer {}", env.token))
        .send()?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    #[derive(serde::Deserialize)]
    struct Body { markdown: String }
    let body: Body = resp.json()?;
    Ok(Some(body.markdown))
}
```

- [ ] **Step 4: Update the binary to intercept markers**

Replace `src/bin/galvolite-hook.rs`:

```rust
//! Per-hook binary executed by Claude Code.

use galvolite::hooks::{client, marker, types::HookKind};
use std::io::{self, Read, Write};

fn main() {
    if let Err(e) = run() {
        eprintln!("[galvolite-hook] error: {e}");
    }
    // Always exit 0: hooks are fail-soft so claude never sees our errors.
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let env = client::ClientEnv {
        api_base: std::env::var("GALVOLITE_API_BASE")?,
        session_id: std::env::var("GALVOLITE_SESSION_ID")?,
        token: std::env::var("GALVOLITE_TOKEN")?,
        timeout_secs: 5,
    };

    let hook_str = std::env::args().nth(1).ok_or("missing hook arg")?;
    let hook: HookKind = serde_json::from_value(serde_json::Value::String(hook_str.clone()))?;

    let mut payload_buf = String::new();
    io::stdin().read_to_string(&mut payload_buf)?;
    let payload: serde_json::Value = if payload_buf.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&payload_buf)?
    };

    // UserPromptSubmit + marker: intercept before forwarding.
    if matches!(hook, HookKind::UserPromptSubmit) {
        let prompt = payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(parent_id) = marker::detect_rejoin_marker(prompt) {
            // Sanity check: marker's parent_id must equal our session id.
            if parent_id.to_string() == env.session_id {
                match client::fetch_pending_rejoin(&env)? {
                    Some(md) => {
                        let response = serde_json::json!({
                            "decision": "block",
                            "additionalContext": md,
                        });
                        io::stdout().write_all(response.to_string().as_bytes())?;
                        return Ok(());
                    }
                    None => {
                        eprintln!("[galvolite-hook] marker matched but /pending-rejoin returned 404 — falling through");
                    }
                }
            }
        }
    }

    // Default path: forward to /hooks/ingest.
    let outcome = client::run(env, hook, payload)?;
    if let Some(text) = outcome.inject_text {
        io::stdout().write_all(text.as_bytes())?;
    }
    Ok(())
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test --test hook_binary_marker
cargo build --bin galvolite-hook  # confirm binary still builds
```

Expected: 2 PASS, build succeeds.

- [ ] **Step 6: Commit**

```bash
git add src/bin/galvolite-hook.rs src/hooks/client.rs Cargo.toml tests/hook_binary_marker.rs
git commit -m "feat(hook): galvolite-hook intercepts rejoin marker → additionalContext"
```

---

## Task 15: `claude_md.rs` — orchestrator + brainstorm spawn-tool guides

**Files:**
- Modify: `src/claude_md.rs`
- Test: `tests/claude_md_profiles.rs` (new)

- [ ] **Step 1: Write the failing tests**

```rust
// tests/claude_md_profiles.rs
use galvolite::claude_md::profile_for_role;

#[test]
fn orchestrator_profile_mentions_spawn_tools() {
    let p = profile_for_role("orchestrator");
    assert!(p.contains("spawn_session"), "orchestrator must know spawn_session");
    assert!(p.contains("wait_children"), "orchestrator must know wait_children");
}

#[test]
fn brainstorm_profile_mentions_spawn_tools() {
    let p = profile_for_role("brainstorm");
    assert!(p.contains("spawn_session"));
    assert!(p.contains("wait_children"));
}

#[test]
fn implementation_profile_omits_spawn_tools() {
    let p = profile_for_role("implementation");
    assert!(!p.contains("spawn_session"), "leaf roles should not advertise spawn");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --test claude_md_profiles
```

Expected: FAIL — profiles don't mention spawn yet (or `orchestrator_profile`/`brainstorm_profile` don't exist).

- [ ] **Step 3: Add the profile constants**

Append to `src/claude_md.rs`:

```rust
pub fn orchestrator_profile() -> &'static str { ORCH_PROFILE }
pub fn brainstorm_profile() -> &'static str { BRAINSTORM_PROFILE }
pub fn scout_profile() -> &'static str { implementation_profile() } // v1: scouts use impl profile

const SPAWN_TOOL_BLOCK: &str = r#"
- `spawn_session({role, task})` — spawn a child session in its own git
  worktree on a branch derived from its UUID. Returns
  `{session_id, worktree_path, branch}`. Soft caps: 4 concurrent / depth 3
  (warn-only).
- `wait_children({children: [uuid, ...]})` — park this session until every
  listed child reaches terminal status. Returns immediately; the harness
  delivers the rejoin payload as `additionalContext` on your next prompt.
"#;

const ORCH_PROFILE: &str = concat!(
    "# CLAUDE.md — GalvoLite Orchestrator Session\n\n\
     You coordinate parallel work across child sessions. Use the spawn tools\n\
     to fan out subtasks; use `wait_children` to park between waves.\n\n\
     ## Coordination\n",
    /* re-include the same MCP-tool block as the impl profile, plus: */
);
// Concrete: build ORCH_PROFILE / BRAINSTORM_PROFILE by string-concatenating
// the existing IMPL_PROFILE body with SPAWN_TOOL_BLOCK appended right after
// the Coordination section. The simplest mechanical fix:

// Replace the const definitions of ORCH_PROFILE and BRAINSTORM_PROFILE with
// raw strings that include both the standard tool list AND SPAWN_TOOL_BLOCK.
```

For clarity, write the concrete strings directly:

```rust
const ORCH_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Orchestrator Session

You coordinate parallel work across child sessions.

## Coordination

Coordinate with GalvoLite via MCP tools (no stdout signals):

- `status_report({kind: 'working'|'complete'|'blocked', note?})` — heartbeat + state.
- `set_role({role})` — change your declared role mid-session.
- `set_plan({markdown})` / `plan_check_step({...})` — track structured plans.
- `write_handoff({summary, next_steps})` — continuation handoff.
- `request_attention({reason})` — request human intervention.
- `notify_operator({level, message})` — push a notification.
- `ask_operator({question, choices?})` — synchronous operator question.
- `spawn_session({role, task})` — spawn a child session in its own git
  worktree on a branch derived from its UUID. Returns
  `{session_id, worktree_path, branch}`. Soft caps: 4 concurrent / depth 3
  (warn-only).
- `wait_children({children: [uuid, ...]})` — park until every listed child
  reaches terminal status. The harness delivers the rejoin payload as
  `additionalContext` on your next prompt.

## Pattern

Spawn → set_plan → wait_children → resume. For sequenced waves, call
`wait_children` repeatedly with smaller cohorts.
"#;

const BRAINSTORM_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Brainstorm Session

Same MCP tool surface as orchestrator, including spawn_session and
wait_children. Use spawn_session to delegate scouting subtasks while you
keep the design conversation going.
"#;
```

Update `profile_for_role`:

```rust
pub fn profile_for_role(role: &str) -> &'static str {
    match role {
        "orchestrator" => orchestrator_profile(),
        "brainstorm" => brainstorm_profile(),
        "scout" => scout_profile(),
        _ => implementation_profile(),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --test claude_md_profiles
```

Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add src/claude_md.rs tests/claude_md_profiles.rs
git commit -m "feat(profiles): orchestrator + brainstorm advertise spawn_session / wait_children"
```

---

## Task 16: Frontend types — `waiting_on` + `session.children_resolved`

**Files:**
- Modify: `ui/src/types.ts`
- Test: covered indirectly by Task 17/18 vitest

- [ ] **Step 1: Edit `ui/src/types.ts`**

```typescript
export interface Session {
  // ... existing fields
  waiting_on?: string[] | null;
}

export interface ChildResolution {
  id: string;
  role: string;
  status: "complete" | "blocked" | "error" | "ended" | "unknown";
  handoff: string | null;
}

export type DomainEvent =
  // ... existing variants
  | {
      type: "session.children_resolved";
      session_id: string;
      children: ChildResolution[];
    };
```

- [ ] **Step 2: Verify TypeScript build**

```bash
cd ui && npm run typecheck
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add ui/src/types.ts
git commit -m "feat(ui): types — waiting_on + session.children_resolved"
```

---

## Task 17: `useSessions` reducer arm + vitest

**Files:**
- Modify: `ui/src/hooks/useSessions.ts`
- Create: `ui/src/hooks/useSessions.test.ts`

- [ ] **Step 1: Write the failing vitest**

```typescript
// ui/src/hooks/useSessions.test.ts
import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useSessions } from "./useSessions";

describe("useSessions session.children_resolved", () => {
  it("clears waiting_on on the parent", async () => {
    const { result } = renderHook(() => useSessions());
    // Seed: insert a session with waiting_on populated.
    act(() => {
      // The hook exposes setSessions for tests via __setForTest, OR drive via applyEvent flow.
      // Easiest: applyEvent with session.event_appended that the reducer handles is overkill.
      // Instead, expose an internal seeding test util on the hook.
      result.current._seed?.([
        { id: "p", waiting_on: ["c1", "c2"] } as any,
      ]);
    });
    act(() => {
      result.current.applyEvent({
        type: "session.children_resolved",
        session_id: "p",
        children: [
          { id: "c1", role: "implementation", status: "complete", handoff: "/h.md" },
          { id: "c2", role: "implementation", status: "complete", handoff: null },
        ],
      });
    });
    const parent = result.current.sessions.find((s) => s.id === "p");
    expect(parent?.waiting_on).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd ui && npx vitest run src/hooks/useSessions.test.ts
```

Expected: FAIL — reducer does not handle the event yet (or `_seed` test util missing).

- [ ] **Step 3: Add the reducer arm + test seam**

Edit `ui/src/hooks/useSessions.ts`. Add inside the switch in `applyEvent`:

```typescript
case "session.children_resolved": {
  const updated = { ...session, waiting_on: null };
  const next = [...prev];
  next[i] = updated;
  return next;
}
```

If the hook doesn't already expose `_seed`, add a minimal test seam:

```typescript
return { sessions, refresh, applyEvent, /* ...existing... */, _seed: setSessions };
```

(Document this in a comment as test-only.)

- [ ] **Step 4: Run test to verify it passes**

```bash
cd ui && npx vitest run src/hooks/useSessions.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useSessions.ts ui/src/hooks/useSessions.test.ts
git commit -m "feat(ui): useSessions handles session.children_resolved"
```

---

## Task 18: `SessionRow` depth + waiting badge

**Files:**
- Modify: `ui/src/components/SessionRow.tsx`
- Create: `ui/src/components/SessionRow.test.tsx`

- [ ] **Step 1: Write the failing vitest**

```typescript
// ui/src/components/SessionRow.test.tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { SessionRow } from "./SessionRow";
import type { Session } from "../types";

const baseSession: Session = {
  id: "s1", project_id: "p1", role: "implementation",
  branch: null, worktree_path: "/tmp/wt", pty_pid: null,
  state: "idle", autopilot: false,
  parent_session_id: null, task_summary: "task",
  created_at: new Date().toISOString(), ended_at: null,
};

describe("SessionRow", () => {
  it("renders depth padding", () => {
    const { container } = render(
      <SessionRow session={baseSession} depth={2} onRefresh={() => {}} />
    );
    expect(container.firstChild).toHaveClass("pl-8");
  });
  it("renders waiting badge when waiting_on populated", () => {
    const s: Session = { ...baseSession, waiting_on: ["c1", "c2", "c3"] };
    render(<SessionRow session={s} onRefresh={() => {}} />);
    expect(screen.getByTestId("waiting-badge")).toHaveTextContent("waiting on 3");
  });
  it("omits waiting badge when waiting_on is null", () => {
    render(<SessionRow session={baseSession} onRefresh={() => {}} />);
    expect(screen.queryByTestId("waiting-badge")).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd ui && npx vitest run src/components/SessionRow.test.tsx
```

Expected: FAIL — props/badge missing.

- [ ] **Step 3: Edit `SessionRow.tsx`**

```typescript
interface Props {
  session: Session;
  onExpand?: (s: Session) => void;
  onRefresh: () => void;
  depth?: number;
}

export function SessionRow({ session, onExpand, onRefresh, depth = 0 }: Props) {
  // ... existing computations
  const padClass = depth > 0 ? `pl-${depth * 4}` : "";
  const waitingCount = session.waiting_on?.length ?? 0;

  return (
    <div
      role="button"
      tabIndex={0}
      className={`flex items-center gap-3 px-4 py-3 w-full text-left cursor-pointer border-b border-gray-800 hover:bg-gray-800/50 focus:outline-none focus:bg-gray-800/70 ${padClass}`}
      // ...existing handlers
    >
      {/* ... existing dot + content + role/branch/age */}
      {waitingCount > 0 && (
        <span
          data-testid="waiting-badge"
          className="text-xs px-2 py-0.5 rounded bg-amber-900 text-amber-300"
        >
          waiting on {waitingCount}
        </span>
      )}
      {/* ... existing state badge, autopilot badge, kill/handoff buttons */}
    </div>
  );
}
```

(Note: Tailwind's JIT may not pick up dynamic `pl-{n*4}` classes — if so, switch to inline style `style={{ paddingLeft: depth * 16 }}`.)

- [ ] **Step 4: Run test to verify it passes**

```bash
cd ui && npx vitest run src/components/SessionRow.test.tsx
```

Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/SessionRow.tsx ui/src/components/SessionRow.test.tsx
git commit -m "feat(ui): SessionRow depth prop + waiting-on badge"
```

---

## Task 19: `Dashboard` nested tree

**Files:**
- Modify: `ui/src/components/Dashboard.tsx`
- Create: `ui/src/components/Dashboard.test.tsx`

- [ ] **Step 1: Write the failing vitest**

```typescript
// ui/src/components/Dashboard.test.tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Dashboard } from "./Dashboard";

// Mock useSessions to return a parent + child.
vi.mock("../hooks/useSessions", () => ({
  useSessions: () => ({
    sessions: [
      { id: "P", parent_session_id: null, task_summary: "parent", state: "idle", role: "orchestrator", autopilot: false, created_at: new Date().toISOString(), project_id: "x", branch: null, worktree_path: "/", pty_pid: null, ended_at: null },
      { id: "C", parent_session_id: "P", task_summary: "child", state: "idle", role: "implementation", autopilot: false, created_at: new Date().toISOString(), project_id: "x", branch: null, worktree_path: "/", pty_pid: null, ended_at: null },
    ],
    refresh: () => {}, applyEvent: () => {},
    createSession: async () => {}, sendText: async () => {},
    triggerHandoff: async () => {}, killSession: async () => {},
    toggleAutopilot: async () => {},
  }),
}));
vi.mock("../hooks/useProjects", () => ({
  useProjects: () => ({ projects: [{ id: "x", name: "x", path: "/" }], createProject: async () => {}, removeProject: async () => {} }),
}));

describe("Dashboard nesting", () => {
  it("renders child indented under parent", () => {
    render(<Dashboard />);
    const rows = screen.getAllByText(/parent|child/);
    // Order check: parent before child in DOM.
    expect(rows[0].textContent).toContain("parent");
    expect(rows[1].textContent).toContain("child");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd ui && npx vitest run src/components/Dashboard.test.tsx
```

Expected: FAIL — flat rendering today.

- [ ] **Step 3: Edit `Dashboard.tsx` to nest by `parent_session_id`**

Inside `Dashboard.tsx`, replace the flat `filtered.map(s => <SessionRow ... />)` with a tree-aware render:

```typescript
const tree = useMemo(() => {
  // Group by parent_session_id.
  const byParent = new Map<string | null, Session[]>();
  for (const s of filtered) {
    const k = s.parent_session_id ?? null;
    const list = byParent.get(k) ?? [];
    list.push(s);
    byParent.set(k, list);
  }
  // DFS from roots.
  const out: Array<{ s: Session; depth: number }> = [];
  const walk = (parentKey: string | null, depth: number) => {
    const kids = byParent.get(parentKey) ?? [];
    for (const k of kids) {
      out.push({ s: k, depth });
      walk(k.id, depth + 1);
    }
  };
  walk(null, 0);
  return out;
}, [filtered]);

// In the JSX where the flat list rendered:
{tree.map(({ s, depth }) => (
  <SessionRow
    key={s.id}
    session={s}
    depth={depth}
    onExpand={expand}
    onRefresh={refresh}
  />
))}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd ui && npx vitest run src/components/Dashboard.test.tsx
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/Dashboard.tsx ui/src/components/Dashboard.test.tsx
git commit -m "feat(ui): Dashboard renders sessions as nested tree by parent_session_id"
```

---

## Task 20: E2E spawn round-trip

**Files:**
- Create: `tests/spawn_e2e.rs`

- [ ] **Step 1: Write the E2E test**

```rust
// tests/spawn_e2e.rs
//! End-to-end: a parent (mock-claude) calls spawn_session twice + wait_children;
//! children mock-claude exits cleanly; harness writes pending_rejoin; parent's PTY
//! sees the marker; the hook binary fetches and returns additionalContext.
mod common;

use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;
use galvolite::db::NewProject;

fn init_git(d: &std::path::Path) {
    Command::new("git").args(["init","-q","-b","main"]).current_dir(d).status().unwrap();
    std::fs::write(d.join("README"),"x").unwrap();
    Command::new("git").args(["add","."]).current_dir(d).status().unwrap();
    Command::new("git").args(["-c","user.email=t@t","-c","user.name=t","commit","-qm","i"])
        .current_dir(d).status().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spawn_e2e_two_children_parent_sees_rejoin() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let proj = app.mgr.add_project(NewProject{
        name:"p".into(), path: dir.path().to_string_lossy().into_owned(), remote_url: None
    }).unwrap();

    // Parent uses /bin/cat as a PTY-friendly stand-in (long-running, accepts stdin).
    let parent = app.mgr.spawn_session(
        proj.id, "parent".into(), "orchestrator".into(), "/bin/cat".into(), vec![]
    ).await.unwrap();

    // Spawn two children manually (E2E: spawn via SessionManager directly).
    let c1 = app.mgr.spawn_child(parent.id, "implementation", "task1").await.unwrap();
    let c2 = app.mgr.spawn_child(parent.id, "implementation", "task2").await.unwrap();
    app.mgr.set_waiting_on(parent.id, &[c1.id, c2.id]).await.unwrap();

    // Drive both to terminal via the same path mcp tools take.
    for cid in [c1.id, c2.id] {
        galvolite::state::apply_event_with_bus(
            &app.db, cid, app.mgr.event_bus(),
            galvolite::state::EventInput {
                source: galvolite::state::Source::Mcp,
                kind: "mcp.status_report".into(),
                payload: serde_json::json!({"kind":"complete"}),
                dedup_key: None,
            }
        ).unwrap();
    }
    // Allow the bus reactor to fire on_child_terminal.
    tokio::time::sleep(Duration::from_millis(150)).await;

    // Parent now has pending_rejoin set.
    let conn = app.db.pool().get().unwrap();
    let pending: Option<String> = conn.query_row(
        "SELECT pending_rejoin FROM sessions WHERE id = ?1",
        [parent.id.to_string()], |r| r.get(0)
    ).unwrap();
    assert!(pending.as_deref().unwrap_or("").contains("Children resolved"));
    assert!(pending.as_deref().unwrap().contains(&c1.id.simple().to_string()[..8]));
    assert!(pending.as_deref().unwrap().contains(&c2.id.simple().to_string()[..8]));

    // GET /pending-rejoin clears it.
    let token = app.db.read_api_token(parent.id).unwrap();
    let req = axum::http::Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{}/pending-rejoin", parent.id))
        .header("Authorization", format!("Bearer {token}"))
        .body(axum::body::Body::empty()).unwrap();
    use tower::ServiceExt;
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);

    // Worktrees of complete children are GC'd.
    assert!(!std::path::Path::new(&c1.worktree_path).exists());
    assert!(!std::path::Path::new(&c2.worktree_path).exists());
}
```

- [ ] **Step 2: Run test to verify it passes**

```bash
cargo test --test spawn_e2e
```

Expected: PASS (all prior tasks must be in place).

- [ ] **Step 3: Commit**

```bash
git add tests/spawn_e2e.rs
git commit -m "test(e2e): spawn → wait_children → status_report → pending_rejoin"
```

---

## Task 21: Final sweep

**Files:** none (verification only)

- [ ] **Step 1: Run the full backend test suite**

```bash
cargo test
```

Expected: all tests pass. Note any newly skipped tests.

- [ ] **Step 2: Run clippy**

```bash
cargo clippy --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 3: Run UI test suite**

```bash
cd ui && npm run typecheck && npx vitest run
```

Expected: typecheck clean, all vitest pass.

- [ ] **Step 4: Smoke-run the dev server**

```bash
cd ui && npm run tauri dev   # or whatever the dev command is for this repo
```

Verify in the dashboard that:
- Spawning a child via the orchestrator profile shows up indented
- "waiting on 1" badge appears on the parent during `wait_children`
- After the child completes, the badge clears and the parent's row pulses

- [ ] **Step 5: Update the project memory**

Edit `~/.claude/projects/.../memory/project_hooks_mcp_spec.md`:
- Mark Sub-plan 6 DONE on main with the date and commit range
- Update test count (target: ~250+ backend tests)
- Note key architectural fact: hook binary intercepts marker → additionalContext

```bash
# (memory updates happen via Edit tool, no commit)
```

- [ ] **Step 6: Final commit (if any cleanup needed)**

```bash
git status
# If clippy/typecheck produced trivial fixes, commit them:
git add -A
git commit -m "chore: sub-plan 6 final sweep — clippy + typecheck"
```

---

## Self-Review Notes

**Spec coverage check:**

| Spec section | Task |
|---|---|
| §2 Migration | 1 |
| §3.1 worktree.rs | 2, 3 |
| §3.1 src/mcp/tools/spawn.rs | 10, 11 |
| §3.1 src/api/spawn.rs | 12 |
| §3.2 SessionManager.spawn_child | 4 |
| §3.2 set_waiting_on / build_rejoin_markdown | 5 |
| §3.2 on_child_terminal | 6, 8 |
| §3.2 hooks/marker.rs (replaces handler branch) | 13, 14 |
| §3.2 claude_md.rs | 15 |
| §3.2 state.rs new event kinds | 7 |
| §3.3 frontend types/hooks/components | 16, 17, 18, 19 |
| §4.3 Recovery | 9 |
| §5 Soft caps | 10 |
| §6 Error handling rows | 5 (ownership), 11 (unknown_child), 6 (concurrent), 12 (auth) |
| §7 Worktree lifecycle | 2, 3, 12 (GC) |
| §8 UI | 18, 19 |
| §9 Testing | every task |
| E2E | 20 |

No gaps.

**No-placeholder check:** every step has actual code or actual command. No "TBD". No "similar to Task N".

**Type consistency check:**
- `spawn_child(&Arc<Self>, parent_id: Uuid, role: &str, task: &str) -> Result<Session>` — used in 4, 5, 6, 8, 11, 20. ✓
- `set_waiting_on(&self, parent_id: Uuid, &[Uuid]) -> Result<()>` — used in 5, 6, 8, 11, 20. ✓
- `build_rejoin_markdown(&self, parent_id: Uuid, &[Uuid]) -> Result<String>` — used in 5, 6, 11. ✓
- `on_child_terminal(&self, child_id: Uuid, status: &str) -> Result<()>` — used in 6, 8, 9, 20. ✓
- `detect_rejoin_marker(&str) -> Option<Uuid>` — Task 13, used in 14. ✓
- `fetch_pending_rejoin(&ClientEnv) -> Result<Option<String>, ClientError>` — Task 14. ✓
- `worktree::add_worktree / remove_worktree / derive_branch_name` — Tasks 2, 3, used in 4, 12, 20. ✓

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-04-30-plan-6-spawn-fanout-rejoin.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — fresh subagent per task, two-stage review (spec compliance + code quality), fast iteration in this session.

**2. Inline Execution** — execute tasks here using `superpowers:executing-plans`, batch execution with checkpoints for review.

**Which approach?**
