# Plan 2 — Dashboard CRUD Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the CLI-vs-GUI gap left by Plan 1b. Every API action gets a GUI surface: Add/Remove Project, Start Session, Send text, Autopilot toggle. Establish the modal + collapsible rail + expand-to-focus patterns for all future plans.

**Architecture:** Two new backend endpoints (`DELETE /api/projects/:id` cascade; `PUT /api/sessions/:id/autopilot` state-only) plus a frontend layer of seven modal components, a SessionRail icon strip, and an ExpandedSessionView that scales a session preview to near-fullscreen via Framer Motion. Autopilot is state-only in Plan 2 — Plan 3 wires behavior.

**Tech Stack:** Rust (axum 0.7, SessionManager, rusqlite), React 18 + TypeScript, Framer Motion, Lucide React icons, Tailwind, Vitest + React Testing Library + jsdom (new test harness).

**Run this plan in a dedicated worktree:** `git worktree add .worktrees/plan-2 -b plan-2-dashboard-crud-parity main` before starting Task 1.

---

## File Structure

### Backend — Rust
- **Modify:** `src/error.rs` — add `ProjectNotFound(Uuid)` variant.
- **Modify:** `src/api.rs` — map `ProjectNotFound → 404`, add two new route handlers.
- **Modify:** `src/db.rs` — add `delete_project_cascade(id)`, `set_session_autopilot(id, bool)`.
- **Modify:** `src/session_manager.rs` — add `delete_project(id)` (kill running → db cascade), `set_autopilot(id, bool)`.
- **Create:** `tests/api_projects_delete.rs` — integration test for DELETE /api/projects/:id.
- **Create:** `tests/api_autopilot.rs` — integration test for autopilot toggle.
- **Create:** `tests/db_delete_cascade.rs` — unit-level test for DB cascade.

### Frontend — scaffolding & shared
- **Modify:** `ui/package.json` — add `framer-motion`, `lucide-react`, `vitest`, `@testing-library/react`, `@testing-library/jest-dom`, `@testing-library/user-event`, `jsdom`.
- **Create:** `ui/vitest.config.ts` — vitest config with jsdom env.
- **Create:** `ui/src/test/setup.ts` — RTL setup with jest-dom matchers + framer-motion mock.
- **Modify:** `ui/src/api.ts` — add `deleteProject`, `setAutopilot`.
- **Modify:** `ui/src/types.ts` — already has `autopilot: boolean`; no change expected.
- **Modify:** `ui/src/hooks/useProjects.ts` — add `createProject`, `removeProject` mutations.
- **Modify:** `ui/src/hooks/useSessions.ts` — add `createSession`, `sendText`, `triggerHandoff`, `killSession`, `toggleAutopilot` mutations.

### Frontend — modal layer
- **Create:** `ui/src/components/modals/Modal.tsx` — base modal with motion, backdrop, ESC/×/backdrop close.
- **Create:** `ui/src/components/modals/AddProjectModal.tsx`
- **Create:** `ui/src/components/modals/StartSessionModal.tsx`
- **Create:** `ui/src/components/modals/RemoveProjectModal.tsx`
- **Create:** `ui/src/components/modals/SendModal.tsx`
- **Create:** `ui/src/components/modals/HandoffModal.tsx`
- **Create:** `ui/src/components/modals/EndSessionModal.tsx`

### Frontend — session view
- **Create:** `ui/src/components/SessionRail.tsx` — collapsible icon strip.
- **Create:** `ui/src/components/ExpandedSessionView.tsx` — overlay frame + backdrop.
- **Create:** `ui/src/hooks/useExpandedSession.ts` — expand state + ESC handler.
- **Modify:** `ui/src/components/Dashboard.tsx` — mount Add/Start Project trigger buttons, manage modal state, wire expand.
- **Modify:** `ui/src/components/SessionRow.tsx` — make card clickable to expand.

### Frontend — tests
- **Create:** `ui/src/components/modals/Modal.test.tsx`
- **Create:** `ui/src/components/modals/AddProjectModal.test.tsx`
- **Create:** `ui/src/components/modals/StartSessionModal.test.tsx`
- **Create:** `ui/src/components/modals/RemoveProjectModal.test.tsx`
- **Create:** `ui/src/components/modals/SendModal.test.tsx`
- **Create:** `ui/src/components/modals/HandoffModal.test.tsx`
- **Create:** `ui/src/components/modals/EndSessionModal.test.tsx`
- **Create:** `ui/src/components/SessionRail.test.tsx`
- **Create:** `ui/src/components/ExpandedSessionView.test.tsx`

### Docs
- **Modify:** `README.md` — update quickstart to note the GUI is now functionally complete (no CLI fallback needed).

---

## Phase 1 — Backend

### Task 1: Add `ProjectNotFound` Error variant and 404 mapping

**Files:**
- Modify: `src/error.rs`
- Modify: `src/api.rs:59-74` (err_to_response)

- [ ] **Step 1: Write failing test for err_to_response**

Create `tests/error_project_not_found.rs`:

```rust
use galvolite::error::Error;
use uuid::Uuid;

#[test]
fn project_not_found_has_clear_display() {
    let id = Uuid::new_v4();
    let e = Error::ProjectNotFound(id);
    let s = e.to_string();
    assert!(s.contains("project"), "display should mention project: {s}");
    assert!(s.contains(&id.to_string()), "display should include id: {s}");
}
```

Run: `cargo test --test error_project_not_found -- --nocapture`
Expected: FAIL — `ProjectNotFound` variant does not exist.

- [ ] **Step 2: Add the variant**

In `src/error.rs`, add within the existing `#[derive(...)] pub enum Error { ... }`:

```rust
    #[error("project not found: {0}")]
    ProjectNotFound(uuid::Uuid),
```

Place it alphabetically near `SessionNotFound` to match style.

- [ ] **Step 3: Verify test passes**

Run: `cargo test --test error_project_not_found -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Map to 404 in the API**

In `src/api.rs`, extend the `match &e` in `err_to_response`:

```rust
        Error::SessionNotFound(_) | Error::ProjectNotFound(_) => {
            (StatusCode::NOT_FOUND, e.to_string()).into_response()
        }
```

Remove the old `Error::SessionNotFound(_)` arm (it's now combined).

- [ ] **Step 5: Update the doc comment above `err_to_response`**

Replace the lines mentioning `ProjectNotFound: missing variant` with:
```
/// - `ProjectNotFound` → 404 (project id does not exist; added in Plan 2)
```

- [ ] **Step 6: Run workspace tests to confirm nothing regressed**

Run: `cargo test --workspace`
Expected: all existing tests still pass.

- [ ] **Step 7: Commit**

```bash
git add src/error.rs src/api.rs tests/error_project_not_found.rs
git commit -m "feat(error): add ProjectNotFound variant mapped to 404"
```

---

### Task 2: Add `Db::delete_project_cascade`

**Files:**
- Create: `tests/db_delete_cascade.rs`
- Modify: `src/db.rs` (add method near `list_projects` around line 239)

- [ ] **Step 1: Write failing test**

Create `tests/db_delete_cascade.rs`:

```rust
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::ritual::SessionState;
use tempfile::tempdir;
use uuid::Uuid;

fn setup_db() -> (Db, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("test.db")).unwrap();
    (db, dir)
}

#[test]
fn delete_project_cascade_removes_project_sessions_and_signals() {
    let (db, _dir) = setup_db();

    let project = db
        .insert_project(NewProject {
            name: "proj".into(),
            path: "/tmp/proj".into(),
            remote_url: None,
        })
        .unwrap();

    let session = db
        .insert_session(NewSession {
            id: Uuid::new_v4(),
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: "/tmp/proj".into(),
            parent_session_id: None,
            task_summary: "test".into(),
        })
        .unwrap();

    db.record_signal(session.id, None, "TEST", Some("payload")).unwrap();

    db.delete_project_cascade(project.id).unwrap();

    assert!(db.get_project(project.id).is_err(), "project should be gone");
    assert_eq!(db.list_sessions().unwrap().len(), 0, "sessions purged");
    assert_eq!(
        db.list_signals_for_session(session.id).unwrap().len(),
        0,
        "signals purged"
    );
}

#[test]
fn delete_project_cascade_returns_not_found_for_unknown_id() {
    let (db, _dir) = setup_db();
    let res = db.delete_project_cascade(Uuid::new_v4());
    assert!(matches!(
        res,
        Err(galvolite::error::Error::ProjectNotFound(_))
    ));
}
```

Run: `cargo test --test db_delete_cascade -- --nocapture`
Expected: FAIL — method does not exist.

- [ ] **Step 2: Implement the method**

In `src/db.rs`, add after `list_projects()`:

```rust
    /// Cascade-delete a project: every signal, cycle, session row and finally
    /// the project row, inside a single transaction.
    ///
    /// Returns `Error::ProjectNotFound` if the id isn't present.
    pub fn delete_project_cascade(&self, id: uuid::Uuid) -> Result<()> {
        use rusqlite::OptionalExtension;
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;

        // Verify the project exists first so we can return a clean 404.
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM projects WHERE id = ?1",
                [id.to_string()],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !exists {
            return Err(Error::ProjectNotFound(id));
        }

        tx.execute(
            "DELETE FROM signals WHERE session_id IN
                (SELECT id FROM sessions WHERE project_id = ?1)",
            [id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM cycles WHERE session_id IN
                (SELECT id FROM sessions WHERE project_id = ?1)",
            [id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM sessions WHERE project_id = ?1",
            [id.to_string()],
        )?;
        tx.execute("DELETE FROM projects WHERE id = ?1", [id.to_string()])?;

        tx.commit()?;
        Ok(())
    }
```

(The `use rusqlite::OptionalExtension;` is added at function scope to keep the change local.)

- [ ] **Step 3: Run cascade tests**

Run: `cargo test --test db_delete_cascade -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 4: Run workspace tests to confirm no regression**

Run: `cargo test --workspace`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/db_delete_cascade.rs
git commit -m "feat(db): add delete_project_cascade with transactional purge"
```

---

### Task 3: Add `Db::set_session_autopilot`

**Files:**
- Modify: `src/db.rs`
- Create: test in `tests/db_delete_cascade.rs` OR new `tests/db_autopilot.rs` — use the latter for isolation.

- [ ] **Step 1: Write failing test**

Create `tests/db_autopilot.rs`:

```rust
use galvolite::db::{Db, NewProject, NewSession};
use tempfile::tempdir;
use uuid::Uuid;

#[test]
fn set_autopilot_flips_flag_and_persists() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();

    let project = db
        .insert_project(NewProject {
            name: "p".into(),
            path: "/tmp/p".into(),
            remote_url: None,
        })
        .unwrap();

    let session = db
        .insert_session(NewSession {
            id: Uuid::new_v4(),
            project_id: project.id,
            role: "impl".into(),
            branch: None,
            worktree_path: "/tmp/p".into(),
            parent_session_id: None,
            task_summary: "t".into(),
        })
        .unwrap();

    assert!(!session.autopilot, "default should be false");

    db.set_session_autopilot(session.id, true).unwrap();
    assert!(db.get_session(session.id).unwrap().autopilot);

    db.set_session_autopilot(session.id, false).unwrap();
    assert!(!db.get_session(session.id).unwrap().autopilot);
}

#[test]
fn set_autopilot_unknown_session_errors() {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();
    let res = db.set_session_autopilot(Uuid::new_v4(), true);
    assert!(res.is_err());
}
```

Run: `cargo test --test db_autopilot -- --nocapture`
Expected: FAIL — method missing.

- [ ] **Step 2: Implement**

In `src/db.rs`, add after `update_session_pid`:

```rust
    /// Set the `autopilot` flag on a session row.
    /// Returns `Error::SessionNotFound` if no row was updated.
    pub fn set_session_autopilot(&self, id: uuid::Uuid, enabled: bool) -> Result<()> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET autopilot = ?1 WHERE id = ?2",
            rusqlite::params![enabled as i64, id.to_string()],
        )?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test db_autopilot -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Workspace regression check**

Run: `cargo test --workspace`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/db_autopilot.rs
git commit -m "feat(db): add set_session_autopilot setter"
```

---

### Task 4: Add `SessionManager::delete_project`

**Files:**
- Modify: `src/session_manager.rs`

- [ ] **Step 1: Write failing test**

Append to `tests/session_manager.rs` (existing file). First inspect it to match its style:

Run: `head -30 tests/session_manager.rs`

Then append this test (use the same setup helpers; if the file has a `setup()` helper, reuse it):

```rust
#[tokio::test]
async fn delete_project_purges_rows_and_kills_running_sessions() {
    let (mgr, _dir) = setup_mgr().await;

    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // spawn an echo-as-agent session so there's something live
    let session = mgr
        .spawn_session(
            project.id,
            "task".into(),
            "impl".into(),
            "echo".into(),
            vec!["hello".into()],
        )
        .await
        .unwrap();

    assert_eq!(mgr.list_sessions().unwrap().len(), 1);

    mgr.delete_project(project.id).await.unwrap();

    assert!(mgr.get_project(project.id).is_err(), "project removed");
    assert_eq!(mgr.list_sessions().unwrap().len(), 0, "sessions purged");
    assert!(!mgr.running_ids().contains(&session.id));
}

#[tokio::test]
async fn delete_project_unknown_returns_not_found() {
    let (mgr, _dir) = setup_mgr().await;
    let res = mgr.delete_project(uuid::Uuid::new_v4()).await;
    assert!(matches!(
        res,
        Err(galvolite::error::Error::ProjectNotFound(_))
    ));
}
```

If `tests/session_manager.rs` doesn't already have a `setup_mgr()` helper, add one near the top:

```rust
async fn setup_mgr() -> (std::sync::Arc<galvolite::session_manager::SessionManager>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.db")).unwrap();
    let mgr = std::sync::Arc::new(
        galvolite::session_manager::SessionManager::new(db, dir.path().to_path_buf()),
    );
    (mgr, dir)
}
```

Run: `cargo test --test session_manager delete_project -- --nocapture`
Expected: FAIL — `delete_project` doesn't exist.

- [ ] **Step 2: Implement `delete_project`**

In `src/session_manager.rs`, add near `kill_session`:

```rust
    /// Cascade-delete a project: kill any running sessions under this project,
    /// then purge rows via `Db::delete_project_cascade`.
    ///
    /// Returns `Error::ProjectNotFound` if the id is unknown.
    /// Kill errors on a per-session basis are best-effort: if a PTY is already
    /// dead the session row is still purged. If every kill succeeds but the
    /// cascade fails (e.g. DB closed), the kills are not rolled back.
    pub async fn delete_project(&self, project_id: Uuid) -> Result<()> {
        // Early 404 so we don't half-kill sessions of a nonexistent project.
        let _ = self.db.get_project(project_id).map_err(|e| match e {
            Error::Db(rusqlite::Error::QueryReturnedNoRows) => Error::ProjectNotFound(project_id),
            other => other,
        })?;

        // Snapshot sessions that belong to this project and are currently running.
        let running: Vec<Uuid> = self
            .db
            .list_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.project_id == project_id)
            .map(|s| s.id)
            .filter(|id| self.running_ids().contains(id))
            .collect();

        for id in running {
            // Best-effort: log and continue on per-session kill failure.
            if let Err(e) = self.kill_session(id).await {
                tracing::warn!(session_id = %id, error = ?e, "kill during project delete failed; continuing");
            }
        }

        self.db.delete_project_cascade(project_id)?;
        Ok(())
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test --test session_manager delete_project -- --nocapture`
Expected: both tests PASS.

- [ ] **Step 4: Workspace regression check**

Run: `cargo test --workspace`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs tests/session_manager.rs
git commit -m "feat(session-manager): add delete_project with kill+cascade"
```

---

### Task 5: Add `SessionManager::set_autopilot`

**Files:**
- Modify: `src/session_manager.rs`
- Modify: `tests/session_manager.rs`

- [ ] **Step 1: Write failing test**

Append to `tests/session_manager.rs`:

```rust
#[tokio::test]
async fn set_autopilot_toggles_session_flag() {
    let (mgr, _dir) = setup_mgr().await;
    let project = mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(project.id, "t".into(), "impl".into(), "echo".into(), vec![])
        .await
        .unwrap();

    assert!(!session.autopilot);
    mgr.set_autopilot(session.id, true).await.unwrap();
    assert!(mgr.get_session(session.id).unwrap().autopilot);
    mgr.set_autopilot(session.id, false).await.unwrap();
    assert!(!mgr.get_session(session.id).unwrap().autopilot);
}

#[tokio::test]
async fn set_autopilot_unknown_session_returns_not_found() {
    let (mgr, _dir) = setup_mgr().await;
    let res = mgr.set_autopilot(uuid::Uuid::new_v4(), true).await;
    assert!(matches!(
        res,
        Err(galvolite::error::Error::SessionNotFound(_))
    ));
}
```

Run: `cargo test --test session_manager set_autopilot -- --nocapture`
Expected: FAIL.

- [ ] **Step 2: Implement**

In `src/session_manager.rs`, add near `kill_session`:

```rust
    /// Toggle the session's `autopilot` flag. State-only in Plan 2; Plan 3
    /// wires behavior off this flag.
    pub async fn set_autopilot(&self, id: Uuid, enabled: bool) -> Result<()> {
        self.db.set_session_autopilot(id, enabled)
    }
```

(Keeping it `async fn` even though it's sync-only today lets Plan 3 add futures without breaking callers.)

- [ ] **Step 3: Run tests**

Run: `cargo test --test session_manager set_autopilot -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Workspace regression check**

Run: `cargo test --workspace`
Expected: green.

- [ ] **Step 5: Commit**

```bash
git add src/session_manager.rs tests/session_manager.rs
git commit -m "feat(session-manager): add set_autopilot passthrough"
```

---

### Task 6: Add `DELETE /api/projects/:id` endpoint

**Files:**
- Modify: `src/api.rs`
- Create: `tests/api_projects_delete.rs`

- [ ] **Step 1: Write failing integration test**

Create `tests/api_projects_delete.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::api::router;
use galvolite::db::{Db, NewProject};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup() -> (axum::Router, Arc<SessionManager>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();
    let mgr = Arc::new(SessionManager::new(db, dir.path().to_path_buf()));
    let app = router(mgr.clone());
    (app, mgr, dir)
}

#[tokio::test]
async fn delete_project_returns_204_and_purges() {
    let (app, mgr, _dir) = setup().await;
    let project = mgr
        .add_project(NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects/{}", project.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(mgr.get_project(project.id).is_err());
}

#[tokio::test]
async fn delete_project_unknown_returns_404() {
    let (app, _mgr, _dir) = setup().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects/{}", uuid::Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

Run: `cargo test --test api_projects_delete -- --nocapture`
Expected: FAIL — route not wired.

- [ ] **Step 2: Wire the route**

In `src/api.rs`, modify the router:

```rust
        .route(
            "/api/projects/:id",
            get(get_project).delete(delete_project),
        )
```

- [ ] **Step 3: Add the handler**

Below `get_project` in `src/api.rs`:

```rust
async fn delete_project(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.delete_project(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_to_response(e),
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_projects_delete -- --nocapture`
Expected: both PASS.

- [ ] **Step 5: Workspace regression check**

Run: `cargo test --workspace`
Expected: green.

- [ ] **Step 6: Commit**

```bash
git add src/api.rs tests/api_projects_delete.rs
git commit -m "feat(api): DELETE /api/projects/:id cascade"
```

---

### Task 7: Add `PUT /api/sessions/:id/autopilot` endpoint

**Files:**
- Modify: `src/api.rs`
- Create: `tests/api_autopilot.rs`

- [ ] **Step 1: Write failing integration test**

Create `tests/api_autopilot.rs`:

```rust
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use galvolite::api::router;
use galvolite::db::{Db, NewProject};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup() -> (axum::Router, Arc<SessionManager>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();
    let mgr = Arc::new(SessionManager::new(db, dir.path().to_path_buf()));
    (router(mgr.clone()), mgr, dir)
}

#[tokio::test]
async fn put_autopilot_true_flips_flag() {
    let (app, mgr, _dir) = setup().await;
    let project = mgr
        .add_project(NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(project.id, "t".into(), "impl".into(), "echo".into(), vec![])
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/sessions/{}/autopilot", session.id))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["autopilot"], true);
    assert!(mgr.get_session(session.id).unwrap().autopilot);
}

#[tokio::test]
async fn put_autopilot_unknown_session_404() {
    let (app, _mgr, _dir) = setup().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/sessions/{}/autopilot", uuid::Uuid::new_v4()))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

Run: `cargo test --test api_autopilot -- --nocapture`
Expected: FAIL — route missing.

- [ ] **Step 2: Wire the route**

In `src/api.rs`, add to the router chain:

```rust
        .route(
            "/api/sessions/:id/autopilot",
            axum::routing::put(set_autopilot),
        )
```

- [ ] **Step 3: Add the handler**

Below `send_to_session` in `src/api.rs`:

```rust
#[derive(Deserialize)]
struct AutopilotReq {
    enabled: bool,
}

async fn set_autopilot(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<AutopilotReq>,
) -> Response {
    match mgr.set_autopilot(id, req.enabled).await {
        Ok(()) => Json(serde_json::json!({"autopilot": req.enabled})).into_response(),
        Err(e) => err_to_response(e),
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_autopilot -- --nocapture`
Expected: both PASS.

- [ ] **Step 5: Workspace regression check + clippy**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/api.rs tests/api_autopilot.rs
git commit -m "feat(api): PUT /api/sessions/:id/autopilot toggle"
```

---

## Phase 2 — Frontend scaffolding

### Task 8: Install new frontend dependencies

**Files:**
- Modify: `ui/package.json` + `ui/package-lock.json`

- [ ] **Step 1: Install runtime deps**

Run: `cd ui && npm install framer-motion@^11.15.0 lucide-react@^0.468.0`
Expected: deps added, no peer warnings that block.

- [ ] **Step 2: Install test harness deps**

Run: `cd ui && npm install -D vitest@^2.1.8 @testing-library/react@^16.1.0 @testing-library/jest-dom@^6.6.3 @testing-library/user-event@^14.5.2 jsdom@^25.0.1 @vitest/ui@^2.1.8`
Expected: dev deps added.

- [ ] **Step 3: Verify build still clean**

Run: `cd ui && npm run build`
Expected: builds without errors.

- [ ] **Step 4: Commit**

```bash
git add ui/package.json ui/package-lock.json
git commit -m "chore(ui): add framer-motion, lucide-react, vitest+RTL"
```

---

### Task 9: Configure vitest + test setup

**Files:**
- Create: `ui/vitest.config.ts`
- Create: `ui/src/test/setup.ts`
- Modify: `ui/package.json` (add `test` script)
- Modify: `ui/tsconfig.json` (include vitest types if needed)

- [ ] **Step 1: Create vitest config**

Create `ui/vitest.config.ts`:

```ts
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    globals: true,
    css: false,
  },
});
```

- [ ] **Step 2: Create test setup file**

Create `ui/src/test/setup.ts`:

```ts
import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// Framer Motion animations are not meaningful in jsdom.
// Mock `motion.*` to regular DOM elements so markup queries still work.
vi.mock("framer-motion", async () => {
  const actual = await vi.importActual<typeof import("framer-motion")>(
    "framer-motion",
  );
  return {
    ...actual,
    AnimatePresence: ({ children }: { children: React.ReactNode }) => children,
    motion: new Proxy(
      {},
      {
        get: (_t, tag: string) => {
          const Component = (props: Record<string, unknown>) => {
            const React = require("react");
            // Strip motion-only props that React complains about.
            const {
              initial,
              animate,
              exit,
              transition,
              variants,
              whileHover,
              whileTap,
              layoutId,
              ...rest
            } = props;
            return React.createElement(tag, rest, (props as any).children);
          };
          return Component;
        },
      },
    ),
  };
});
```

- [ ] **Step 3: Add test script to package.json**

In `ui/package.json`, add inside `"scripts"`:

```json
    "test": "vitest run",
    "test:watch": "vitest"
```

- [ ] **Step 4: Write a smoke test**

Create `ui/src/test/smoke.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";

describe("test harness", () => {
  it("renders and queries DOM", () => {
    render(<div>hello</div>);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });
});
```

- [ ] **Step 5: Run the smoke test**

Run: `cd ui && npm test`
Expected: 1 test PASS.

- [ ] **Step 6: Verify `npm run build` still passes**

Run: `cd ui && npm run build`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add ui/vitest.config.ts ui/src/test/setup.ts ui/src/test/smoke.test.tsx ui/package.json
git commit -m "chore(ui): vitest + RTL harness with framer-motion mock"
```

---

### Task 10: Add API client methods for delete + autopilot

**Files:**
- Modify: `ui/src/api.ts`

- [ ] **Step 1: Extend `api` object**

In `ui/src/api.ts`, inside the `export const api = { ... }` block, add:

```ts
  deleteProject: (id: string) =>
    request<void>(`/api/projects/${id}`, { method: "DELETE" }).catch((e) => {
      // 204 No Content returns empty body; our `request` helper assumes JSON.
      // Treat parse errors on an OK response as success.
      if (e instanceof SyntaxError) return;
      throw e;
    }),

  setAutopilot: (id: string, enabled: boolean) =>
    request<{ autopilot: boolean }>(`/api/sessions/${id}/autopilot`, {
      method: "PUT",
      body: JSON.stringify({ enabled }),
    }),
```

- [ ] **Step 2: Fix the 204 issue cleanly — update `request` helper**

The catch-hack above is brittle. Replace the `request` function with:

```ts
async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...init?.headers,
    },
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`${resp.status}: ${text}`);
  }
  if (resp.status === 204) {
    return undefined as T;
  }
  const len = resp.headers.get("content-length");
  if (len === "0") {
    return undefined as T;
  }
  return resp.json() as Promise<T>;
}
```

And simplify `deleteProject`:

```ts
  deleteProject: (id: string) =>
    request<void>(`/api/projects/${id}`, { method: "DELETE" }),
```

- [ ] **Step 3: TypeCheck**

Run: `cd ui && npx tsc --noEmit`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add ui/src/api.ts
git commit -m "feat(ui): api.deleteProject + api.setAutopilot"
```

---

### Task 11: Add mutation helpers to `useProjects` and `useSessions`

**Files:**
- Modify: `ui/src/hooks/useProjects.ts`
- Modify: `ui/src/hooks/useSessions.ts`

- [ ] **Step 1: Read current hooks**

Run: `cat ui/src/hooks/useProjects.ts ui/src/hooks/useSessions.ts`

Note the existing shape (list state + refresh). The new mutations should return promises and call the existing `refresh` / `reload` on success.

- [ ] **Step 2: Extend `useProjects`**

Assuming the existing hook exposes `{ projects, loading, error, refresh }`, modify the return type and add:

```ts
import { api } from "../api";
import type { CreateProjectReq, Project } from "../types";

// ... existing state ...

const createProject = async (req: CreateProjectReq): Promise<Project> => {
  const p = await api.createProject(req);
  await refresh();
  return p;
};

const removeProject = async (id: string): Promise<void> => {
  await api.deleteProject(id);
  await refresh();
};

return { projects, loading, error, refresh, createProject, removeProject };
```

Adjust to match the actual existing return shape. If the hook already returns an object with spread members, add `createProject` and `removeProject` to the returned object.

- [ ] **Step 3: Extend `useSessions`**

Add to the existing returned object:

```ts
const createSession = async (req: CreateSessionReq) => {
  const s = await api.createSession(req);
  await refresh();
  return s;
};

const sendText = (id: string, text: string) =>
  api.sendToSession(id, { text, newline: true });

const triggerHandoff = (id: string) => api.triggerHandoff(id);

const killSession = async (id: string) => {
  await api.killSession(id);
  await refresh();
};

const toggleAutopilot = async (id: string, enabled: boolean) => {
  await api.setAutopilot(id, enabled);
  await refresh();
};
```

Import the types at the top:

```ts
import type { CreateSessionReq } from "../types";
```

Add all five names to the returned object.

- [ ] **Step 4: TypeCheck**

Run: `cd ui && npx tsc --noEmit`
Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useProjects.ts ui/src/hooks/useSessions.ts
git commit -m "feat(ui): mutation helpers on useProjects/useSessions"
```

---

## Phase 3 — Base modal component

### Task 12: Build `Modal.tsx` base component

**Files:**
- Create: `ui/src/components/modals/Modal.tsx`
- Create: `ui/src/components/modals/Modal.test.tsx`

- [ ] **Step 1: Write failing test**

Create `ui/src/components/modals/Modal.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { Modal } from "./Modal";

describe("Modal", () => {
  it("renders children when open", () => {
    render(
      <Modal open onClose={() => {}} title="Test">
        <p>body</p>
      </Modal>,
    );
    expect(screen.getByText("body")).toBeInTheDocument();
    expect(screen.getByText("Test")).toBeInTheDocument();
  });

  it("does not render when closed", () => {
    render(
      <Modal open={false} onClose={() => {}} title="Test">
        <p>body</p>
      </Modal>,
    );
    expect(screen.queryByText("body")).not.toBeInTheDocument();
  });

  it("calls onClose when × is clicked", async () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="T">
        <p>x</p>
      </Modal>,
    );
    await userEvent.click(screen.getByRole("button", { name: /close/i }));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose on ESC keypress", async () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="T">
        <p>x</p>
      </Modal>,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("calls onClose when backdrop clicked", async () => {
    const onClose = vi.fn();
    render(
      <Modal open onClose={onClose} title="T">
        <p>x</p>
      </Modal>,
    );
    await userEvent.click(screen.getByTestId("modal-backdrop"));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
```

Run: `cd ui && npm test Modal`
Expected: FAIL — `./Modal` doesn't exist.

- [ ] **Step 2: Implement `Modal.tsx`**

Create `ui/src/components/modals/Modal.tsx`:

```tsx
import { useEffect } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { X } from "lucide-react";

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title: string;
  children: React.ReactNode;
  originRect?: DOMRect | null;
}

export function Modal({ open, onClose, title, children, originRect }: ModalProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  const spring = originRect
    ? {
        initial: {
          opacity: 0,
          scale: 0.2,
          x: originRect.left - window.innerWidth / 2,
          y: originRect.top - window.innerHeight / 2,
        },
        animate: { opacity: 1, scale: 1, x: 0, y: 0 },
        exit: { opacity: 0, scale: 0.5 },
      }
    : {
        initial: { opacity: 0, scale: 0.9 },
        animate: { opacity: 1, scale: 1 },
        exit: { opacity: 0, scale: 0.9 },
      };

  return (
    <AnimatePresence>
      {open ? (
        <motion.div
          key="backdrop"
          data-testid="modal-backdrop"
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          onClick={onClose}
        >
          <motion.div
            key="panel"
            className="relative bg-neutral-900 border border-neutral-700 rounded-lg min-w-[360px] max-w-[560px] w-full mx-4 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
            {...spring}
            transition={{ type: "spring", stiffness: 260, damping: 22 }}
          >
            <header className="flex items-center justify-between px-4 py-3 border-b border-neutral-800">
              <h2 className="text-sm font-semibold text-neutral-100">{title}</h2>
              <button
                aria-label="Close"
                className="text-neutral-400 hover:text-neutral-100"
                onClick={onClose}
              >
                <X size={16} />
              </button>
            </header>
            <div className="p-4">{children}</div>
          </motion.div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
```

- [ ] **Step 3: Run tests**

Run: `cd ui && npm test Modal`
Expected: all 5 tests PASS.

- [ ] **Step 4: TypeCheck**

Run: `cd ui && npx tsc --noEmit`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/modals/Modal.tsx ui/src/components/modals/Modal.test.tsx
git commit -m "feat(ui): Modal base component with spring motion + close handlers"
```

---

## Phase 4 — Project modals

### Task 13: AddProjectModal

**Files:**
- Create: `ui/src/components/modals/AddProjectModal.tsx`
- Create: `ui/src/components/modals/AddProjectModal.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { AddProjectModal } from "./AddProjectModal";

describe("AddProjectModal", () => {
  it("submits name + path to onSubmit", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    render(<AddProjectModal open onClose={onClose} onSubmit={onSubmit} />);

    await userEvent.type(screen.getByLabelText(/name/i), "galvolite");
    await userEvent.type(screen.getByLabelText(/path/i), "/tmp/galvolite");
    await userEvent.click(screen.getByRole("button", { name: /add/i }));

    expect(onSubmit).toHaveBeenCalledWith({
      name: "galvolite",
      path: "/tmp/galvolite",
    });
  });

  it("shows server error inline", async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error("400: bad path"));
    render(<AddProjectModal open onClose={() => {}} onSubmit={onSubmit} />);

    await userEvent.type(screen.getByLabelText(/name/i), "p");
    await userEvent.type(screen.getByLabelText(/path/i), "/nope");
    await userEvent.click(screen.getByRole("button", { name: /add/i }));

    expect(await screen.findByText(/bad path/)).toBeInTheDocument();
  });

  it("disables submit when fields empty", () => {
    render(<AddProjectModal open onClose={() => {}} onSubmit={vi.fn()} />);
    expect(screen.getByRole("button", { name: /add/i })).toBeDisabled();
  });
});
```

Run: `cd ui && npm test AddProjectModal`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/modals/AddProjectModal.tsx`:

```tsx
import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (req: { name: string; path: string }) => Promise<unknown>;
}

export function AddProjectModal({ open, onClose, onSubmit }: Props) {
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const canSubmit = name.trim().length > 0 && path.trim().length > 0 && !busy;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit({ name: name.trim(), path: path.trim() });
      setName("");
      setPath("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Add Project">
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Name</span>
          <input
            className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Path</span>
          <input
            className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100 font-mono"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="/absolute/path/to/workspace"
          />
        </label>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <button
            type="button"
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            disabled={!canSubmit}
          >
            {busy ? "Adding…" : "Add"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
```

- [ ] **Step 3: Run tests**

Run: `cd ui && npm test AddProjectModal`
Expected: all 3 PASS.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/modals/AddProjectModal.tsx ui/src/components/modals/AddProjectModal.test.tsx
git commit -m "feat(ui): AddProjectModal"
```

---

### Task 14: RemoveProjectModal (confirm)

**Files:**
- Create: `ui/src/components/modals/RemoveProjectModal.tsx`
- Create: `ui/src/components/modals/RemoveProjectModal.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { RemoveProjectModal } from "./RemoveProjectModal";

describe("RemoveProjectModal", () => {
  it("shows project name + session count", () => {
    render(
      <RemoveProjectModal
        open
        onClose={() => {}}
        onConfirm={vi.fn()}
        projectName="galvolite"
        sessionCount={3}
      />,
    );
    expect(screen.getByText(/galvolite/)).toBeInTheDocument();
    expect(screen.getByText(/3 session/i)).toBeInTheDocument();
  });

  it("calls onConfirm when Remove clicked", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(
      <RemoveProjectModal
        open
        onClose={() => {}}
        onConfirm={onConfirm}
        projectName="p"
        sessionCount={0}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /remove/i }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("shows error inline", async () => {
    const onConfirm = vi.fn().mockRejectedValue(new Error("500: db locked"));
    render(
      <RemoveProjectModal
        open
        onClose={() => {}}
        onConfirm={onConfirm}
        projectName="p"
        sessionCount={1}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /remove/i }));
    expect(await screen.findByText(/db locked/)).toBeInTheDocument();
  });
});
```

Run: `cd ui && npm test RemoveProjectModal`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/modals/RemoveProjectModal.tsx`:

```tsx
import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onConfirm: () => Promise<unknown>;
  projectName: string;
  sessionCount: number;
}

export function RemoveProjectModal({
  open,
  onClose,
  onConfirm,
  projectName,
  sessionCount,
}: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleConfirm() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Remove Project">
      <div className="flex flex-col gap-3 text-sm text-neutral-300">
        <p>
          Remove <strong className="text-neutral-100">{projectName}</strong> from
          GalvoLite?
        </p>
        <p className="text-xs text-neutral-400">
          This kills {sessionCount} session{sessionCount === 1 ? "" : "s"} and
          deletes all history (signals + cycles). The workspace directory on
          disk is not touched.
        </p>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <button
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="px-3 py-1 text-sm bg-red-700 hover:bg-red-600 disabled:opacity-50 rounded text-white"
            onClick={handleConfirm}
            disabled={busy}
          >
            {busy ? "Removing…" : "Remove"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
```

- [ ] **Step 3: Run tests**

Run: `cd ui && npm test RemoveProjectModal`
Expected: 3 PASS.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/modals/RemoveProjectModal.tsx ui/src/components/modals/RemoveProjectModal.test.tsx
git commit -m "feat(ui): RemoveProjectModal confirm"
```

---

### Task 15: Wire Add / Remove Project into Dashboard

**Files:**
- Modify: `ui/src/components/Dashboard.tsx`
- Modify: `ui/src/components/ProjectSelector.tsx` (add remove affordance)

- [ ] **Step 1: Inspect current Dashboard + ProjectSelector**

Run: `cat ui/src/components/Dashboard.tsx ui/src/components/ProjectSelector.tsx`

Note where the project list renders and how `createProject` / `removeProject` from the updated hook should flow.

- [ ] **Step 2: Add Add Project trigger + modal**

In `Dashboard.tsx` add state and button:

```tsx
import { useState } from "react";
import { Plus } from "lucide-react";
import { AddProjectModal } from "./modals/AddProjectModal";
import { RemoveProjectModal } from "./modals/RemoveProjectModal";
// ...
const [addOpen, setAddOpen] = useState(false);
const [removeTarget, setRemoveTarget] = useState<Project | null>(null);
const { projects, createProject, removeProject } = useProjects();
const { sessions } = useSessions();

// within the header area, near the project selector
<button
  className="flex items-center gap-1 text-xs text-neutral-300 hover:text-neutral-100 border border-neutral-700 rounded px-2 py-1"
  onClick={() => setAddOpen(true)}
>
  <Plus size={12} /> Add Project
</button>

// mount the modals at end of the component's render tree
<AddProjectModal
  open={addOpen}
  onClose={() => setAddOpen(false)}
  onSubmit={(req) => createProject(req)}
/>
<RemoveProjectModal
  open={removeTarget !== null}
  projectName={removeTarget?.name ?? ""}
  sessionCount={
    removeTarget
      ? sessions.filter((s) => s.project_id === removeTarget.id).length
      : 0
  }
  onClose={() => setRemoveTarget(null)}
  onConfirm={() =>
    removeTarget ? removeProject(removeTarget.id) : Promise.resolve()
  }
/>
```

- [ ] **Step 3: Add remove affordance to ProjectSelector**

Where each project renders in `ProjectSelector.tsx`, add a small trash icon button that calls an `onRemove(project)` prop. Pipe the prop through from Dashboard so clicking it sets `removeTarget`.

In `ProjectSelector.tsx`:

```tsx
import { Trash2 } from "lucide-react";

interface Props {
  projects: Project[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onRemove?: (p: Project) => void;
}

// inside the map over projects:
<div className="flex items-center justify-between group">
  <button onClick={() => onSelect(p.id)}>{p.name}</button>
  {onRemove && (
    <button
      className="opacity-0 group-hover:opacity-100 text-neutral-500 hover:text-red-400"
      aria-label={`Remove ${p.name}`}
      onClick={(e) => {
        e.stopPropagation();
        onRemove(p);
      }}
    >
      <Trash2 size={12} />
    </button>
  )}
</div>
```

And pass `onRemove={setRemoveTarget}` from Dashboard.

- [ ] **Step 4: TypeCheck + build**

Run: `cd ui && npx tsc --noEmit && npm run build`
Expected: clean.

- [ ] **Step 5: Run all frontend tests**

Run: `cd ui && npm test`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add ui/src/components/Dashboard.tsx ui/src/components/ProjectSelector.tsx
git commit -m "feat(ui): wire AddProject + RemoveProject modals to Dashboard"
```

---

## Phase 5 — Session modals + Start Session

### Task 16: SendModal

**Files:**
- Create: `ui/src/components/modals/SendModal.tsx`
- Create: `ui/src/components/modals/SendModal.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { SendModal } from "./SendModal";

describe("SendModal", () => {
  it("sends typed text", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<SendModal open onClose={() => {}} onSubmit={onSubmit} />);
    await userEvent.type(
      screen.getByRole("textbox"),
      "please summarise the plan",
    );
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(onSubmit).toHaveBeenCalledWith("please summarise the plan");
  });

  it("disables send when empty", () => {
    render(<SendModal open onClose={() => {}} onSubmit={vi.fn()} />);
    expect(screen.getByRole("button", { name: /send/i })).toBeDisabled();
  });
});
```

Run: `cd ui && npm test SendModal`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/modals/SendModal.tsx`:

```tsx
import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (text: string) => Promise<unknown>;
}

export function SendModal({ open, onClose, onSubmit }: Props) {
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit(text);
      setText("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Send to Session">
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <textarea
          className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100 font-mono min-h-[120px]"
          value={text}
          onChange={(e) => setText(e.target.value)}
          autoFocus
          placeholder="Type message to send…"
        />
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button
            type="button"
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            disabled={!text.trim() || busy}
          >
            {busy ? "Sending…" : "Send"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
```

- [ ] **Step 3: Run tests**

Run: `cd ui && npm test SendModal`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/modals/SendModal.tsx ui/src/components/modals/SendModal.test.tsx
git commit -m "feat(ui): SendModal"
```

---

### Task 17: HandoffModal

**Files:**
- Create: `ui/src/components/modals/HandoffModal.tsx`
- Create: `ui/src/components/modals/HandoffModal.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { HandoffModal } from "./HandoffModal";

describe("HandoffModal", () => {
  it("calls onConfirm when Trigger clicked", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(<HandoffModal open onClose={() => {}} onConfirm={onConfirm} />);
    await userEvent.click(screen.getByRole("button", { name: /trigger/i }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });

  it("shows error inline", async () => {
    const onConfirm = vi.fn().mockRejectedValue(new Error("409: not running"));
    render(<HandoffModal open onClose={() => {}} onConfirm={onConfirm} />);
    await userEvent.click(screen.getByRole("button", { name: /trigger/i }));
    expect(await screen.findByText(/not running/)).toBeInTheDocument();
  });
});
```

Run: `cd ui && npm test HandoffModal`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/modals/HandoffModal.tsx`:

```tsx
import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onConfirm: () => Promise<unknown>;
}

export function HandoffModal({ open, onClose, onConfirm }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handle() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Trigger Handoff">
      <div className="flex flex-col gap-3 text-sm text-neutral-300">
        <p>
          Start the handoff ritual. The session will be asked to write its
          <code className="text-neutral-400"> HANDOFF.md</code> and exit.
        </p>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            onClick={handle}
            disabled={busy}
          >
            {busy ? "Triggering…" : "Trigger"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
```

- [ ] **Step 3: Run tests + commit**

```
cd ui && npm test HandoffModal
git add ui/src/components/modals/HandoffModal.tsx ui/src/components/modals/HandoffModal.test.tsx
git commit -m "feat(ui): HandoffModal"
```

---

### Task 18: EndSessionModal

**Files:**
- Create: `ui/src/components/modals/EndSessionModal.tsx`
- Create: `ui/src/components/modals/EndSessionModal.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { EndSessionModal } from "./EndSessionModal";

describe("EndSessionModal", () => {
  it("calls onConfirm when End clicked", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    render(
      <EndSessionModal
        open
        onClose={() => {}}
        onConfirm={onConfirm}
        role="implementation"
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /^end/i }));
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
```

Run: `cd ui && npm test EndSessionModal`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/modals/EndSessionModal.tsx`:

```tsx
import { useState } from "react";
import { Modal } from "./Modal";

interface Props {
  open: boolean;
  onClose: () => void;
  onConfirm: () => Promise<unknown>;
  role: string;
}

export function EndSessionModal({ open, onClose, onConfirm, role }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handle() {
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="End Session">
      <div className="flex flex-col gap-3 text-sm text-neutral-300">
        <p>
          Send SIGTERM to this <strong>{role}</strong> session. History is
          preserved.
        </p>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            className="px-3 py-1 text-sm bg-red-700 hover:bg-red-600 disabled:opacity-50 rounded text-white"
            onClick={handle}
            disabled={busy}
          >
            {busy ? "Ending…" : "End"}
          </button>
        </div>
      </div>
    </Modal>
  );
}
```

- [ ] **Step 3: Run + commit**

```
cd ui && npm test EndSessionModal
git add ui/src/components/modals/EndSessionModal.tsx ui/src/components/modals/EndSessionModal.test.tsx
git commit -m "feat(ui): EndSessionModal"
```

---

### Task 19: StartSessionModal

**Files:**
- Create: `ui/src/components/modals/StartSessionModal.tsx`
- Create: `ui/src/components/modals/StartSessionModal.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { StartSessionModal } from "./StartSessionModal";

describe("StartSessionModal", () => {
  it("submits task + role + command", async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={onSubmit}
        projectId="proj-1"
      />,
    );
    await userEvent.type(screen.getByLabelText(/task/i), "refactor api");
    const roleInput = screen.getByLabelText(/role/i);
    await userEvent.clear(roleInput);
    await userEvent.type(roleInput, "scout");
    const cmdInput = screen.getByLabelText(/command/i);
    await userEvent.clear(cmdInput);
    await userEvent.type(cmdInput, "claude");
    await userEvent.click(screen.getByRole("button", { name: /start/i }));

    expect(onSubmit).toHaveBeenCalledWith({
      project_id: "proj-1",
      task: "refactor api",
      role: "scout",
      claude_cmd: "claude",
    });
  });

  it("disables start with empty task", () => {
    render(
      <StartSessionModal
        open
        onClose={() => {}}
        onSubmit={vi.fn()}
        projectId="p"
      />,
    );
    expect(screen.getByRole("button", { name: /start/i })).toBeDisabled();
  });
});
```

Run: `cd ui && npm test StartSessionModal`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/modals/StartSessionModal.tsx`:

```tsx
import { useState } from "react";
import { Modal } from "./Modal";
import type { CreateSessionReq } from "../../types";

interface Props {
  open: boolean;
  onClose: () => void;
  onSubmit: (req: CreateSessionReq) => Promise<unknown>;
  projectId: string;
}

export function StartSessionModal({ open, onClose, onSubmit, projectId }: Props) {
  const [task, setTask] = useState("");
  const [role, setRole] = useState("implementation");
  const [cmd, setCmd] = useState("claude");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canSubmit = task.trim().length > 0 && !busy;

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!canSubmit) return;
    setBusy(true);
    setError(null);
    try {
      await onSubmit({
        project_id: projectId,
        task: task.trim(),
        role: role.trim(),
        claude_cmd: cmd.trim(),
      });
      setTask("");
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Start Session">
      <form onSubmit={handleSubmit} className="flex flex-col gap-3">
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Task</span>
          <input
            className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100"
            value={task}
            onChange={(e) => setTask(e.target.value)}
            autoFocus
            placeholder="Short description of what this session should do"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Role</span>
          <input
            className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100"
            value={role}
            onChange={(e) => setRole(e.target.value)}
          />
        </label>
        <label className="flex flex-col gap-1 text-xs">
          <span className="text-neutral-400">Command</span>
          <input
            className="bg-neutral-800 border border-neutral-700 rounded px-2 py-1 text-sm text-neutral-100 font-mono"
            value={cmd}
            onChange={(e) => setCmd(e.target.value)}
          />
        </label>
        {error && (
          <p className="text-xs text-red-400" role="alert">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <button
            type="button"
            className="px-3 py-1 text-sm text-neutral-400 hover:text-neutral-100"
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="px-3 py-1 text-sm bg-blue-600 hover:bg-blue-500 disabled:opacity-50 rounded text-white"
            disabled={!canSubmit}
          >
            {busy ? "Starting…" : "Start"}
          </button>
        </div>
      </form>
    </Modal>
  );
}
```

- [ ] **Step 3: Run + commit**

```
cd ui && npm test StartSessionModal
git add ui/src/components/modals/StartSessionModal.tsx ui/src/components/modals/StartSessionModal.test.tsx
git commit -m "feat(ui): StartSessionModal"
```

---

### Task 20: Wire Start Session trigger into Dashboard

**Files:**
- Modify: `ui/src/components/Dashboard.tsx`

- [ ] **Step 1: Add Start Session button + modal state**

In `Dashboard.tsx`:

```tsx
import { PlayCircle } from "lucide-react";
import { StartSessionModal } from "./modals/StartSessionModal";
// ...
const [startOpen, setStartOpen] = useState(false);
const { createSession } = useSessions();

// in header:
{selectedProjectId && (
  <button
    className="flex items-center gap-1 text-xs text-neutral-300 hover:text-neutral-100 border border-neutral-700 rounded px-2 py-1"
    onClick={() => setStartOpen(true)}
  >
    <PlayCircle size={12} /> Start Session
  </button>
)}

// mount modal
{selectedProjectId && (
  <StartSessionModal
    open={startOpen}
    onClose={() => setStartOpen(false)}
    onSubmit={(req) => createSession(req)}
    projectId={selectedProjectId}
  />
)}
```

`selectedProjectId` is whatever the existing Dashboard uses to track the active project (check current source — match that name).

- [ ] **Step 2: TypeCheck + build**

Run: `cd ui && npx tsc --noEmit && npm run build`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/Dashboard.tsx
git commit -m "feat(ui): wire StartSessionModal into Dashboard"
```

---

## Phase 6 — Session rail + expanded view

### Task 21: SessionRail icon strip

**Files:**
- Create: `ui/src/components/SessionRail.tsx`
- Create: `ui/src/components/SessionRail.test.tsx`

- [ ] **Step 1: Write failing test**

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { SessionRail } from "./SessionRail";

describe("SessionRail", () => {
  const baseProps = {
    autopilot: false,
    onSend: vi.fn(),
    onHandoff: vi.fn(),
    onToggleAutopilot: vi.fn(),
    onEnd: vi.fn(),
  };

  it("renders four action icons", () => {
    render(<SessionRail {...baseProps} />);
    expect(screen.getByRole("button", { name: /send/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /handoff/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /autopilot/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /end session/i })).toBeInTheDocument();
  });

  it("fires onSend when send icon clicked", async () => {
    const onSend = vi.fn();
    render(<SessionRail {...baseProps} onSend={onSend} />);
    await userEvent.click(screen.getByRole("button", { name: /send/i }));
    expect(onSend).toHaveBeenCalledOnce();
  });

  it("fires onToggleAutopilot with opposite state", async () => {
    const onToggle = vi.fn();
    render(
      <SessionRail {...baseProps} autopilot={false} onToggleAutopilot={onToggle} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /autopilot/i }));
    expect(onToggle).toHaveBeenCalledWith(true);
  });

  it("expands when expand handle clicked", async () => {
    render(<SessionRail {...baseProps} />);
    await userEvent.click(screen.getByRole("button", { name: /expand rail/i }));
    expect(screen.getByText(/End session/)).toBeInTheDocument();
  });
});
```

Run: `cd ui && npm test SessionRail`
Expected: FAIL.

- [ ] **Step 2: Implement**

Create `ui/src/components/SessionRail.tsx`:

```tsx
import { useState } from "react";
import { Send, ArrowLeftRight, Zap, Power, ChevronLeft, ChevronRight } from "lucide-react";

interface Props {
  autopilot: boolean;
  onSend: (origin: DOMRect) => void;
  onHandoff: (origin: DOMRect) => void;
  onToggleAutopilot: (next: boolean) => void;
  onEnd: (origin: DOMRect) => void;
}

export function SessionRail({
  autopilot,
  onSend,
  onHandoff,
  onToggleAutopilot,
  onEnd,
}: Props) {
  const [expanded, setExpanded] = useState(false);

  function trigger(cb: (r: DOMRect) => void) {
    return (e: React.MouseEvent) => {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      cb(rect);
    };
  }

  const iconBtn =
    "flex items-center gap-2 text-xs text-neutral-300 hover:text-neutral-100 rounded px-2 py-2";

  return (
    <aside
      className={`border-l border-neutral-800 bg-neutral-950 flex flex-col transition-[width] duration-200 ${
        expanded ? "w-28" : "w-9"
      }`}
    >
      <button
        className={iconBtn}
        aria-label="Send to session"
        onClick={trigger(onSend)}
      >
        <Send size={16} />
        {expanded && <span>Send</span>}
      </button>
      <button
        className={iconBtn}
        aria-label="Handoff"
        onClick={trigger(onHandoff)}
      >
        <ArrowLeftRight size={16} />
        {expanded && <span>Handoff</span>}
      </button>
      <button
        className={`${iconBtn} ${autopilot ? "text-yellow-300" : ""}`}
        aria-label="Autopilot"
        aria-pressed={autopilot}
        onClick={() => onToggleAutopilot(!autopilot)}
      >
        <Zap size={16} />
        {expanded && <span>Autopilot</span>}
      </button>
      <button
        className={`${iconBtn} text-red-400 hover:text-red-300`}
        aria-label="End session"
        onClick={trigger(onEnd)}
      >
        <Power size={16} />
        {expanded && <span>End session</span>}
      </button>
      <div className="flex-1" />
      <button
        className={iconBtn}
        aria-label={expanded ? "Collapse rail" : "Expand rail"}
        onClick={() => setExpanded((v) => !v)}
      >
        {expanded ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
      </button>
    </aside>
  );
}
```

- [ ] **Step 3: Run tests**

Run: `cd ui && npm test SessionRail`
Expected: 4 PASS.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/SessionRail.tsx ui/src/components/SessionRail.test.tsx
git commit -m "feat(ui): SessionRail collapsible icon strip"
```

---

### Task 22: `useExpandedSession` + `ExpandedSessionView`

**Files:**
- Create: `ui/src/hooks/useExpandedSession.ts`
- Create: `ui/src/components/ExpandedSessionView.tsx`
- Create: `ui/src/components/ExpandedSessionView.test.tsx`

- [ ] **Step 1: Write failing test**

Create `ui/src/components/ExpandedSessionView.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi } from "vitest";
import { ExpandedSessionView } from "./ExpandedSessionView";
import type { Session } from "../types";

const stubSession: Session = {
  id: "sess-1",
  project_id: "p-1",
  role: "scout",
  branch: null,
  worktree_path: "/tmp/p",
  pty_pid: 123,
  state: "working",
  autopilot: false,
  parent_session_id: null,
  task_summary: "hello",
  created_at: "2026-04-17T00:00:00Z",
  ended_at: null,
};

describe("ExpandedSessionView", () => {
  it("renders session metadata", () => {
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={() => {}}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onToggleAutopilot={vi.fn()}
        onEnd={vi.fn()}
      />,
    );
    expect(screen.getByText(/scout/)).toBeInTheDocument();
    expect(screen.getByText(/sess-1/)).toBeInTheDocument();
  });

  it("closes on backdrop click", async () => {
    const onClose = vi.fn();
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={onClose}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onToggleAutopilot={vi.fn()}
        onEnd={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByTestId("expanded-backdrop"));
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes on ESC", async () => {
    const onClose = vi.fn();
    render(
      <ExpandedSessionView
        session={stubSession}
        onClose={onClose}
        onSend={vi.fn()}
        onHandoff={vi.fn()}
        onToggleAutopilot={vi.fn()}
        onEnd={vi.fn()}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });
});
```

Run: `cd ui && npm test ExpandedSessionView`
Expected: FAIL.

- [ ] **Step 2: Implement the hook**

Create `ui/src/hooks/useExpandedSession.ts`:

```ts
import { useEffect, useState } from "react";

export function useExpandedSession() {
  const [expandedId, setExpandedId] = useState<string | null>(null);

  useEffect(() => {
    if (!expandedId) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setExpandedId(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [expandedId]);

  return { expandedId, expand: setExpandedId, collapse: () => setExpandedId(null) };
}
```

- [ ] **Step 3: Implement `ExpandedSessionView`**

Create `ui/src/components/ExpandedSessionView.tsx`:

```tsx
import { useEffect } from "react";
import { motion } from "framer-motion";
import type { Session } from "../types";
import { TerminalPreview } from "./TerminalPreview";
import { SessionRail } from "./SessionRail";

interface Props {
  session: Session;
  onClose: () => void;
  onSend: (origin: DOMRect) => void;
  onHandoff: (origin: DOMRect) => void;
  onEnd: (origin: DOMRect) => void;
  onToggleAutopilot: (next: boolean) => void;
}

export function ExpandedSessionView({
  session,
  onClose,
  onSend,
  onHandoff,
  onEnd,
  onToggleAutopilot,
}: Props) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <motion.div
      data-testid="expanded-backdrop"
      className="fixed inset-0 z-40 bg-black/70 flex items-center justify-center"
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      onClick={onClose}
    >
      <motion.div
        className="bg-neutral-950 border border-neutral-800 rounded-lg shadow-2xl flex flex-col"
        style={{ width: "92vw", height: "92vh" }}
        initial={{ scale: 0.85, opacity: 0 }}
        animate={{ scale: 1, opacity: 1 }}
        exit={{ scale: 0.85, opacity: 0 }}
        transition={{ type: "spring", stiffness: 240, damping: 24 }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-4 py-2 border-b border-neutral-800 flex items-center justify-between text-xs text-neutral-300">
          <div>
            <strong className="text-neutral-100">{session.role}</strong>{" "}
            <span className="opacity-50">· {session.id} · {session.state}</span>
          </div>
          <div className="opacity-60">ESC or click outside to close</div>
        </header>
        <div className="flex flex-1 min-h-0">
          <div className="flex-1 min-w-0">
            <TerminalPreview sessionId={session.id} />
          </div>
          <SessionRail
            autopilot={session.autopilot}
            onSend={onSend}
            onHandoff={onHandoff}
            onEnd={onEnd}
            onToggleAutopilot={onToggleAutopilot}
          />
        </div>
      </motion.div>
    </motion.div>
  );
}
```

Note: `TerminalPreview` is the existing component from Plan 1b. If its prop name differs from `sessionId`, match the existing signature.

- [ ] **Step 4: Run tests**

Run: `cd ui && npm test ExpandedSessionView`
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useExpandedSession.ts ui/src/components/ExpandedSessionView.tsx ui/src/components/ExpandedSessionView.test.tsx
git commit -m "feat(ui): ExpandedSessionView + useExpandedSession"
```

---

### Task 23: Wire expand + rail actions end-to-end in Dashboard

**Files:**
- Modify: `ui/src/components/Dashboard.tsx`
- Modify: `ui/src/components/SessionRow.tsx`

- [ ] **Step 1: Make SessionRow click emit an expand request**

In `SessionRow.tsx`, add an optional `onExpand(session)` prop. Wrap the row in a clickable element that calls `onExpand` on click. Preserve existing row content.

```tsx
interface Props {
  session: Session;
  onExpand?: (s: Session) => void;
}

<button
  type="button"
  className="block w-full text-left"
  onClick={() => onExpand?.(session)}
>
  {/* existing row content */}
</button>
```

- [ ] **Step 2: Wire state in Dashboard**

In `Dashboard.tsx`:

```tsx
import { useExpandedSession } from "../hooks/useExpandedSession";
import { ExpandedSessionView } from "./ExpandedSessionView";
import { SendModal } from "./modals/SendModal";
import { HandoffModal } from "./modals/HandoffModal";
import { EndSessionModal } from "./modals/EndSessionModal";

const { expandedId, expand, collapse } = useExpandedSession();
const expanded = sessions.find((s) => s.id === expandedId) ?? null;
const { sendText, triggerHandoff, killSession, toggleAutopilot } = useSessions();

const [modal, setModal] = useState<null | "send" | "handoff" | "end">(null);

// Pass onExpand={expand ↦ (s) => expand(s.id)} to each SessionRow
<SessionRow session={s} onExpand={(s) => expand(s.id)} />

// Render the expanded overlay (outside normal dashboard flow)
{expanded && (
  <ExpandedSessionView
    session={expanded}
    onClose={collapse}
    onSend={() => setModal("send")}
    onHandoff={() => setModal("handoff")}
    onEnd={() => setModal("end")}
    onToggleAutopilot={(next) => toggleAutopilot(expanded.id, next)}
  />
)}

{expanded && (
  <>
    <SendModal
      open={modal === "send"}
      onClose={() => setModal(null)}
      onSubmit={(text) => sendText(expanded.id, text)}
    />
    <HandoffModal
      open={modal === "handoff"}
      onClose={() => setModal(null)}
      onConfirm={() => triggerHandoff(expanded.id)}
    />
    <EndSessionModal
      open={modal === "end"}
      onClose={() => setModal(null)}
      onConfirm={async () => {
        await killSession(expanded.id);
        collapse();
      }}
      role={expanded.role}
    />
  </>
)}
```

- [ ] **Step 3: TypeCheck + build + tests**

Run: `cd ui && npx tsc --noEmit && npm run build && npm test`
Expected: everything green.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/Dashboard.tsx ui/src/components/SessionRow.tsx
git commit -m "feat(ui): wire expand-to-focus + rail-driven modals"
```

---

## Phase 7 — Integration, cleanup, docs

### Task 24: Full stack smoke test via Tauri dev

**Files:** none (verification only)

- [ ] **Step 1: Run full test suite**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cd ui && npm test && npm run build && cd ..`
Expected: all green.

- [ ] **Step 2: Launch Tauri dev**

Run: `cargo tauri dev`
Expected: axum binds 127.0.0.1:3001, Vite on :5173, webview opens the dashboard.

- [ ] **Step 3: Manual smoke checklist**

Verify each action works from the GUI without touching a terminal:

- [ ] Click **Add Project** → modal springs open → enter name + a valid path → click Add → project appears in the list.
- [ ] Click **Start Session** (with a project selected) → modal opens → fill task/role/command (use `echo` as the command if you don't have claude installed) → Start → new session appears.
- [ ] Click a session row's preview → expands to near-fullscreen with dimmed backdrop and rail visible.
- [ ] Click rail **Send** icon → SendModal pops → type text → Send → terminal shows the input.
- [ ] Click rail **Autopilot** icon → zap icon turns yellow; refresh the dashboard and confirm the state persisted.
- [ ] Click rail **Handoff** icon → HandoffModal → Trigger → session state updates (or clean error if the session isn't in a ritual).
- [ ] Click rail **End session** icon → EndSessionModal → End → session transitions to killed, expanded view closes.
- [ ] Press ESC while expanded → collapses.
- [ ] Click backdrop while expanded → collapses.
- [ ] Hover a project in the selector → trash icon appears → click → RemoveProjectModal shows correct session count → Remove → project + its sessions gone.

- [ ] **Step 4: Note any issues found**

If anything fails, file a micro-task and fix before proceeding. Do NOT move to Task 25 with failing flows.

---

### Task 25: Update README quickstart

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README to reflect GUI parity**

In the Plan 1b quickstart section (currently notes `send`/`handoff`/`kill` are CLI-only when using non-Tauri mode), update to clarify:

- All actions are GUI-native in the Tauri desktop app.
- CLI remains available but is a debugging/scripting escape hatch.
- Add a bullet noting the new endpoints: `DELETE /api/projects/:id` and `PUT /api/sessions/:id/autopilot`.

Example addition under the "Quickstart" section:

```markdown
### Plan 2 additions

- Every API action is accessible from the dashboard. Add/Remove projects, Start sessions, Send text, Handoff, End session, Autopilot toggle all live in modals reachable within two clicks.
- Click any session preview on the dashboard to open a near-fullscreen focused view with a collapsible action rail.
- New endpoints:
  - `DELETE /api/projects/:id` — cascade-deletes the project, its sessions, cycles, and signals. Directory on disk is not touched.
  - `PUT /api/sessions/:id/autopilot` — toggles the `autopilot` flag on a session. State-only in Plan 2; Plan 3 wires behavior.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: Plan 2 README update — GUI parity + new endpoints"
```

---

## Verification Summary

Once all 25 tasks are complete, run the full verification battery:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd ui && npm test && npm run build && cd ..
cargo tauri build  # optional release bundle check
```

All green = Plan 2 ready to merge. Use the `superpowers:finishing-a-development-branch` skill to merge back to `main`.
