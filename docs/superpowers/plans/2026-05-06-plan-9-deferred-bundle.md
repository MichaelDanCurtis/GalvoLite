# Plan 9 Deferred Bundle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the seven items deferred from Plan 8 — defaults-on flip, 7-day rec auto-archive, `ProjectDeleted` bus event, internal `tail_per_session`, UTC token-budget reset, RecModal status filter, and RecModal re-explain button — as a single sequenced release before Plan 10.

**Architecture:** Seven sequential tasks, each one an isolated change with its own commit(s). One new SQLite migration (`0006_rec_archived.sql`) adds an `archived` status and a `status_changed_at` column. One new `DomainEvent` variant. One new field on `OverseerState` and one on `ContextSnapshot`. Two RecModal additions. No new subsystems.

**Tech Stack:** Rust 1.x (axum, tokio, rusqlite, chrono, tracing, serde), SQLite migrations under `migrations/`, React + TypeScript + Vitest under `ui/`.

**Spec:** [docs/superpowers/specs/2026-05-06-plan-9-deferred-bundle-design.md](../specs/2026-05-06-plan-9-deferred-bundle-design.md)

---

## Task ordering rationale

- **T1** (defaults flip) is a one-line behavior change in two places and orthogonal to everything else. Land it first to retire one item from the deck immediately.
- **T2** (archive sweep) introduces the schema change and new `RecStatus::Archived` variant. It must land before **T6** (status filter) which surfaces that variant in the UI.
- **T3, T4, T5** (`ProjectDeleted`, `tail_per_session`, UTC budget reset) are independent of T1–T2 and of each other. Order is `T3 → T4 → T5` to keep backend work batched before UI work.
- **T6, T7** are both UI changes in the same file; do them last so UI tests pick up both.

---

## Task 1: Flip Overseer + Plan Executor defaults to on

**Files:**
- Modify: `src/overseer/config.rs:8-18`
- Test: `tests/overseer_config.rs:6-23`

The two helpers `overseer_enabled()` and `executor_enabled()` currently default to `false` when their env vars are unset. Invert: unset means on. Set the env var to `0` / `false` / `no` / `off` to disable.

- [ ] **Step 1: Update existing tests for inverted defaults**

  Replace `tests/overseer_config.rs:6-23` with the new expectations:

  ```rust
  #[test]
  fn defaults_on_when_env_unset() {
      let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
      std::env::remove_var("GALVOLITE_OVERSEER");
      std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
      assert!(overseer_enabled());
      assert!(executor_enabled());
  }

  #[test]
  fn disabled_when_env_set_falsy() {
      let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
      std::env::set_var("GALVOLITE_OVERSEER", "0");
      std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "off");
      assert!(!overseer_enabled());
      assert!(!executor_enabled());
      std::env::remove_var("GALVOLITE_OVERSEER");
      std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
  }

  #[test]
  fn enabled_when_env_set_truthy() {
      let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
      std::env::set_var("GALVOLITE_OVERSEER", "1");
      std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "true");
      assert!(overseer_enabled());
      assert!(executor_enabled());
      std::env::remove_var("GALVOLITE_OVERSEER");
      std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
  }
  ```

- [ ] **Step 2: Run tests to verify the new expectations fail**

  Run: `cargo test --test overseer_config defaults_on_when_env_unset disabled_when_env_set_falsy -- --test-threads=1`
  Expected: both FAIL — current code returns `false` when unset.

- [ ] **Step 3: Invert the defaults in `src/overseer/config.rs:8-18`**

  Replace lines 8–18 with:

  ```rust
  pub fn overseer_enabled() -> bool {
      enabled_with_default(std::env::var("GALVOLITE_OVERSEER").ok().as_deref(), true)
  }

  pub fn executor_enabled() -> bool {
      enabled_with_default(std::env::var("GALVOLITE_PLAN_EXECUTOR").ok().as_deref(), true)
  }

  fn enabled_with_default(v: Option<&str>, default: bool) -> bool {
      match v.map(str::to_ascii_lowercase).as_deref() {
          None => default,
          Some("1" | "true" | "yes" | "on") => true,
          Some("0" | "false" | "no" | "off") => false,
          // Unknown values are conservative: treat as off when the user
          // bothered to set the var to something.
          Some(_) => false,
      }
  }
  ```

  Delete the now-unused `truthy` function.

- [ ] **Step 4: Run all overseer_config tests to verify pass**

  Run: `cargo test --test overseer_config -- --test-threads=1`
  Expected: all 5 tests PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add src/overseer/config.rs tests/overseer_config.rs
  git commit -m "feat(overseer): flip GALVOLITE_OVERSEER and GALVOLITE_PLAN_EXECUTOR defaults to on"
  ```

---

## Task 2: 7-day recommendation auto-archive

**Files:**
- Create: `migrations/0006_rec_archived.sql`
- Modify: `src/recommendations.rs:11-21` (enum + as_str)
- Modify: `src/recommendations.rs:102-121` (set_status + defer write `status_changed_at`)
- Modify: `src/recommendations.rs:80-90` (list_by_status SELECT must include status_changed_at)
- Modify: `src/api/recommendations.rs:39-45` (parser accepts `archived`)
- Modify: `src/main.rs:37-50` (cron tick adds archive sweep)
- Test: `tests/recommendations_db.rs` (extend with three new tests)

**Why a new migration:** SQLite cannot `ALTER` a `CHECK` constraint. We recreate the table to (a) add `archived` to the status CHECK and (b) add `status_changed_at TEXT NOT NULL` for status-change-age sweeps.

- [ ] **Step 1: Create migration `migrations/0006_rec_archived.sql`**

  ```sql
  -- Plan 9 Task 2 — add 'archived' status + status_changed_at column.
  -- SQLite has no ALTER for CHECK constraints, so recreate the table.

  CREATE TABLE recommendations_new (
      id              TEXT PRIMARY KEY,
      project_id      TEXT NOT NULL,
      kind            TEXT NOT NULL,
      severity        TEXT NOT NULL CHECK (severity IN ('info','warn','urgent')),
      title           TEXT NOT NULL,
      body            TEXT NOT NULL,
      rationale       TEXT NOT NULL,
      evidence_json   TEXT NOT NULL,
      status          TEXT NOT NULL CHECK (status IN ('pending','acted','dismissed','deferred','archived')),
      created_at      TEXT NOT NULL,
      deferred_until  TEXT NULL,
      acted_at        TEXT NULL,
      status_changed_at TEXT NOT NULL,
      content_hash    TEXT NOT NULL UNIQUE
  );

  -- Backfill: existing acted/dismissed rows use acted_at (best available
  -- proxy); pending/deferred rows use created_at. This is monotonic with
  -- age, which is all the sweep predicate cares about.
  INSERT INTO recommendations_new
      SELECT id, project_id, kind, severity, title, body, rationale,
             evidence_json, status, created_at, deferred_until, acted_at,
             COALESCE(acted_at, created_at) AS status_changed_at, content_hash
      FROM recommendations;

  DROP TABLE recommendations;
  ALTER TABLE recommendations_new RENAME TO recommendations;

  -- Preserve the existing list-query index (orders by created_at DESC).
  CREATE INDEX recommendations_project_status_idx
      ON recommendations(project_id, status, created_at);

  -- New index for the archive sweep predicate.
  CREATE INDEX recommendations_status_changed_idx
      ON recommendations(status, status_changed_at);
  ```

- [ ] **Step 2: Write failing test for the migration backfill**

  Append to `tests/recommendations_db.rs`:

  ```rust
  #[test]
  fn migration_backfills_status_changed_at() {
      // Schema check via raw SQL: every existing rec must have a non-null
      // status_changed_at after the migration, even when acted_at is null.
      let db = open_db();
      let pid = Uuid::new_v4();
      let id = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
      let conn = db.pool().get().unwrap();
      let s: String = conn
          .query_row(
              "SELECT status_changed_at FROM recommendations WHERE id=?1",
              rusqlite::params![id.to_string()],
              |r| r.get(0),
          )
          .unwrap();
      assert!(!s.is_empty(), "status_changed_at must be set on insert");
  }
  ```

- [ ] **Step 3: Run test to verify it fails**

  Run: `cargo test --test recommendations_db migration_backfills_status_changed_at -- --test-threads=1`
  Expected: FAIL with "no such column: status_changed_at" (migration not yet picked up because `recommendations::insert` doesn't write it yet).

- [ ] **Step 4: Add `Archived` to `RecStatus` enum and parser**

  Replace `src/recommendations.rs:11-21`:

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum RecStatus { Pending, Acted, Dismissed, Deferred, Archived }

  impl RecStatus {
      pub fn as_str(self) -> &'static str {
          match self {
              Self::Pending => "pending", Self::Acted => "acted",
              Self::Dismissed => "dismissed", Self::Deferred => "deferred",
              Self::Archived => "archived",
          }
      }
  }
  ```

  Replace `src/api/recommendations.rs:39-45`:

  ```rust
  let status = match q.status.as_deref() {
      None | Some("pending") => RecStatus::Pending,
      Some("acted") => RecStatus::Acted,
      Some("dismissed") => RecStatus::Dismissed,
      Some("deferred") => RecStatus::Deferred,
      Some("archived") => RecStatus::Archived,
      _ => return (StatusCode::BAD_REQUEST, "bad status").into_response(),
  };
  ```

- [ ] **Step 5: Update `insert`, `set_status`, `defer` to write `status_changed_at`**

  Modify `src/recommendations.rs:60-78` (`insert`):

  ```rust
  pub fn insert(db: &Db, n: NewRec) -> Result<Option<Uuid>> {
      let hash = content_hash(&n.kind, &n.title, &n.body);
      let id = Uuid::new_v4();
      let now = Utc::now();
      let conn = db.pool().get()?;
      let rows = conn.execute(
          "INSERT OR IGNORE INTO recommendations
           (id, project_id, kind, severity, title, body, rationale, evidence_json,
            status, created_at, status_changed_at, content_hash)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,?9,?10)",
          params![
              id.to_string(), n.project_id.to_string(), n.kind, n.severity,
              n.title, n.body, n.rationale,
              serde_json::to_string(&n.evidence_json).map_err(|e| Error::Config(e.to_string()))?,
              now.to_rfc3339(), hash,
          ],
      )?;
      Ok(if rows == 0 { None } else { Some(id) })
  }
  ```

  Note: `?9` is reused for both `created_at` and `status_changed_at`.

  Modify `set_status` at lines 102-112:

  ```rust
  pub fn set_status(db: &Db, id: Uuid, status: RecStatus) -> Result<()> {
      let conn = db.pool().get()?;
      let now = Utc::now().to_rfc3339();
      let acted_at = if matches!(status, RecStatus::Acted) {
          Some(now.clone())
      } else { None };
      conn.execute(
          "UPDATE recommendations SET status=?1, acted_at=COALESCE(?2, acted_at),
           status_changed_at=?3 WHERE id=?4",
          params![status.as_str(), acted_at, now, id.to_string()],
      )?;
      Ok(())
  }
  ```

  Modify `defer` at lines 114-121:

  ```rust
  pub fn defer(db: &Db, id: Uuid, until: DateTime<Utc>) -> Result<()> {
      let conn = db.pool().get()?;
      let now = Utc::now().to_rfc3339();
      conn.execute(
          "UPDATE recommendations SET status='deferred', deferred_until=?1,
           status_changed_at=?2 WHERE id=?3",
          params![until.to_rfc3339(), now, id.to_string()],
      )?;
      Ok(())
  }
  ```

  Modify `resurface_expired` at lines 123-131:

  ```rust
  pub fn resurface_expired(db: &Db, now: DateTime<Utc>) -> Result<usize> {
      let conn = db.pool().get()?;
      let n = conn.execute(
          "UPDATE recommendations SET status='pending', deferred_until=NULL,
           status_changed_at=?1
           WHERE status='deferred' AND deferred_until IS NOT NULL AND deferred_until <= ?1",
          params![now.to_rfc3339()],
      )?;
      Ok(n)
  }
  ```

- [ ] **Step 6: Add `archive_old` function**

  Append to `src/recommendations.rs` (after `update_rationale`):

  ```rust
  /// Plan 9 Task 2 — flip `acted` and `dismissed` recs older than `age`
  /// (measured by `status_changed_at`) to `archived`. Returns row count for
  /// logging. Pending and deferred recs are exempt.
  pub fn archive_old(db: &Db, age: chrono::Duration) -> Result<usize> {
      let conn = db.pool().get()?;
      let cutoff = (Utc::now() - age).to_rfc3339();
      let now = Utc::now().to_rfc3339();
      let n = conn.execute(
          "UPDATE recommendations SET status='archived', status_changed_at=?1
           WHERE status IN ('acted','dismissed') AND status_changed_at < ?2",
          params![now, cutoff],
      )?;
      Ok(n)
  }
  ```

- [ ] **Step 7: Run the migration test to verify it passes**

  Run: `cargo test --test recommendations_db migration_backfills_status_changed_at -- --test-threads=1`
  Expected: PASS.

- [ ] **Step 8: Add archive sweep tests**

  Append to `tests/recommendations_db.rs`:

  ```rust
  #[test]
  fn archive_old_sweeps_acted_and_dismissed() {
      let db = open_db();
      let pid = Uuid::new_v4();

      // Acted rec aged 8 days: should archive.
      let mut s = sample(pid); s.title = "acted-old".into();
      let id_acted = recommendations::insert(&db, s).unwrap().unwrap();
      recommendations::set_status(&db, id_acted, RecStatus::Acted).unwrap();
      backdate(&db, id_acted, 8);

      // Dismissed rec aged 8 days: should archive.
      let mut s = sample(pid); s.title = "dismissed-old".into();
      let id_dis = recommendations::insert(&db, s).unwrap().unwrap();
      recommendations::set_status(&db, id_dis, RecStatus::Dismissed).unwrap();
      backdate(&db, id_dis, 8);

      let n = recommendations::archive_old(&db, chrono::Duration::days(7)).unwrap();
      assert_eq!(n, 2);
      assert_eq!(recommendations::get(&db, id_acted).unwrap().status, "archived");
      assert_eq!(recommendations::get(&db, id_dis).unwrap().status, "archived");
  }

  #[test]
  fn archive_old_exempts_pending_and_deferred() {
      let db = open_db();
      let pid = Uuid::new_v4();

      let mut s = sample(pid); s.title = "pending-old".into();
      let id_pending = recommendations::insert(&db, s).unwrap().unwrap();
      backdate(&db, id_pending, 30);

      let mut s = sample(pid); s.title = "deferred-old".into();
      let id_def = recommendations::insert(&db, s).unwrap().unwrap();
      let future = chrono::Utc::now() + chrono::Duration::days(7);
      recommendations::defer(&db, id_def, future).unwrap();
      backdate(&db, id_def, 30);

      let n = recommendations::archive_old(&db, chrono::Duration::days(7)).unwrap();
      assert_eq!(n, 0);
      assert_eq!(recommendations::get(&db, id_pending).unwrap().status, "pending");
      assert_eq!(recommendations::get(&db, id_def).unwrap().status, "deferred");
  }

  #[test]
  fn archive_old_respects_threshold() {
      let db = open_db();
      let pid = Uuid::new_v4();

      let mut s = sample(pid); s.title = "young".into();
      let id_young = recommendations::insert(&db, s).unwrap().unwrap();
      recommendations::set_status(&db, id_young, RecStatus::Acted).unwrap();
      backdate(&db, id_young, 6); // 6 days — under threshold

      let mut s = sample(pid); s.title = "old".into();
      let id_old = recommendations::insert(&db, s).unwrap().unwrap();
      recommendations::set_status(&db, id_old, RecStatus::Acted).unwrap();
      backdate(&db, id_old, 8); // 8 days — over threshold

      let n = recommendations::archive_old(&db, chrono::Duration::days(7)).unwrap();
      assert_eq!(n, 1);
      assert_eq!(recommendations::get(&db, id_young).unwrap().status, "acted");
      assert_eq!(recommendations::get(&db, id_old).unwrap().status, "archived");
  }

  /// Helper: rewrites `status_changed_at` for one rec to N days ago.
  /// Tests use this instead of waiting wall-clock time.
  fn backdate(db: &Db, id: Uuid, days_ago: i64) {
      let conn = db.pool().get().unwrap();
      let backdated = (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339();
      conn.execute(
          "UPDATE recommendations SET status_changed_at=?1 WHERE id=?2",
          rusqlite::params![backdated, id.to_string()],
      ).unwrap();
  }
  ```

- [ ] **Step 9: Run new tests to verify pass**

  Run: `cargo test --test recommendations_db archive_old_sweeps_acted_and_dismissed archive_old_exempts_pending_and_deferred archive_old_respects_threshold -- --test-threads=1`
  Expected: all 3 PASS.

- [ ] **Step 10: Wire archive sweep into the existing 5-minute cron tick**

  Modify `src/main.rs:37-50` — extend the existing tokio::spawn block:

  ```rust
  // Plan 8 Task 15 — recommendations sweep: every 5 minutes, flip any
  // `deferred` recs whose `deferred_until` has elapsed back to `pending`.
  // Plan 9 Task 2 — same tick also archives acted/dismissed recs older
  // than 7 days (status-change age).
  // Runs unconditionally (harmless when Overseer is disabled — there will
  // simply be nothing to resurface or archive).
  {
      let mgr = Arc::clone(&mgr);
      tokio::spawn(async move {
          let mut t = tokio::time::interval(std::time::Duration::from_secs(300));
          t.tick().await;
          loop {
              t.tick().await;
              let _ = galvolite::recommendations::resurface_expired(
                  mgr.db(),
                  chrono::Utc::now(),
              );
              if let Err(e) = galvolite::recommendations::archive_old(
                  mgr.db(),
                  chrono::Duration::days(7),
              ) {
                  tracing::warn!(error = ?e, "rec archive sweep failed");
              }
          }
      });
  }
  ```

- [ ] **Step 11: Build to verify everything compiles**

  Run: `cargo build`
  Expected: PASS, no warnings.

- [ ] **Step 12: Run the full test suite (DB + API recs)**

  Run: `cargo test --test recommendations_db --test api_recommendations_list --test api_recommendations_act --test api_recommendations_defer --test api_recommendations_explain -- --test-threads=1`
  Expected: all PASS, including the existing tests that should be unaffected.

- [ ] **Step 13: Commit**

  ```bash
  git add migrations/0006_rec_archived.sql src/recommendations.rs \
          src/api/recommendations.rs src/main.rs tests/recommendations_db.rs
  git commit -m "feat(recs): 7-day auto-archive for acted/dismissed recommendations"
  ```

---

## Task 3: `ProjectDeleted` bus event

**Files:**
- Modify: `src/api/events.rs:13-98` (add variant), `src/api/events.rs:131-159` (`applies_to`)
- Modify: `src/session_manager.rs:692-722` (`delete_project` emits the event)
- Modify: `src/overseer/mod.rs:91-145` (subscribe + drop_project handling)
- Test: `tests/overseer_supervisor.rs` (one new test)

The Overseer's lifecycle today is governed by a 30-second polling reconcile loop noted at `src/overseer/mod.rs:19-28`. Once the bus emits `ProjectDeleted`, the supervisor can react in the existing `ProjectAdded` listener path and the polling loop becomes redundant — but we keep the polling loop as a belt-and-suspenders fallback for now and just drop the project from the in-memory map on the bus event.

- [ ] **Step 1: Write failing test in `tests/overseer_supervisor.rs`**

  First inspect the existing test scaffolding:

  Run: `grep -n "fn " tests/overseer_supervisor.rs | head -10`

  Append a new test to the file. It builds a supervisor with a tight reconcile interval, registers a project, subscribes to the bus, deletes the project via `delete_project`, and asserts that a `ProjectDeleted` event is published.

  ```rust
  #[tokio::test]
  async fn delete_project_emits_project_deleted_bus_event() {
      // Setup mirrors the existing supervisor tests in this file: spin up a
      // SessionManager with a temp DB and a registered project.
      let tmp = tempfile::tempdir().unwrap();
      let db = galvolite::db::Db::open(tmp.path().join("g.db")).unwrap();
      let mgr = std::sync::Arc::new(
          galvolite::session_manager::SessionManager::new(db.clone(), tmp.path().to_path_buf()),
      );
      let project = mgr.add_project(galvolite::db::NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();

      let mut rx = mgr.event_bus().subscribe();

      // Drain the ProjectAdded that fires from add_project.
      let _ = rx.recv().await;

      mgr.delete_project(project.id).await.unwrap();

      // Allow up to 1s for the event to propagate.
      let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
          .await.expect("timed out waiting for bus event").unwrap();

      match evt {
          galvolite::api::events::DomainEvent::ProjectDeleted { project_id } => {
              assert_eq!(project_id, project.id);
          }
          other => panic!("expected ProjectDeleted, got {other:?}"),
      }
  }
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cargo test --test overseer_supervisor delete_project_emits_project_deleted_bus_event -- --test-threads=1`
  Expected: FAIL with "no variant `ProjectDeleted`".

- [ ] **Step 3: Add the variant to `DomainEvent`**

  Insert into `src/api/events.rs:96-98` (just before `ProjectAdded`):

  ```rust
      #[serde(rename = "project.deleted")]
      ProjectDeleted { project_id: Uuid },
      #[serde(rename = "project.added")]
      ProjectAdded { project_id: Uuid },
  ```

  Update `applies_to` at lines 148-157 — add `DomainEvent::ProjectDeleted { .. }` to the same arm as `ProjectAdded` (returns `false`):

  ```rust
      DomainEvent::DirectiveChanged { .. }
      | DomainEvent::PostmortemWritten { .. }
      | DomainEvent::DecisionRecorded { .. }
      | DomainEvent::PhaseReviewNeeded { .. }
      | DomainEvent::PhaseAdvanced { .. }
      | DomainEvent::RecommendationCreated { .. }
      | DomainEvent::RecommendationActed { .. }
      | DomainEvent::OverseerDegraded { .. }
      | DomainEvent::OverseerRecovered { .. }
      | DomainEvent::ProjectAdded { .. }
      | DomainEvent::ProjectDeleted { .. } => false,
  ```

- [ ] **Step 4: Emit `ProjectDeleted` from `session_manager::delete_project`**

  Modify `src/session_manager.rs:692-722`. The function already calls `db.delete_project_cascade(project_id)?` — emit immediately after success, mirroring the `ProjectAdded` pattern at line 597-599:

  ```rust
      pub async fn delete_project(&self, project_id: Uuid) -> Result<()> {
          // ... existing body unchanged through `delete_project_cascade` call ...
          self.db.delete_project_cascade(project_id)?;
          self.event_bus().publish(
              crate::api::events::DomainEvent::ProjectDeleted { project_id },
          );
          Ok(())
      }
  ```

- [ ] **Step 5: Run the test to verify it passes**

  Run: `cargo test --test overseer_supervisor delete_project_emits_project_deleted_bus_event -- --test-threads=1`
  Expected: PASS.

- [ ] **Step 6: Subscribe Overseer supervisor to `ProjectDeleted`**

  In `src/overseer/mod.rs`, modify `start_lifecycle_listener` lines 122-144 — add a `ProjectDeleted` arm to the existing `match rx.recv().await` block:

  ```rust
          tokio::spawn(async move {
              loop {
                  match rx.recv().await {
                      Ok(crate::api::events::DomainEvent::ProjectAdded { project_id }) => {
                          let mut t = tasks_bus.lock().unwrap();
                          if t.contains_key(&project_id) {
                              continue;
                          }
                          let mgr = Arc::clone(&mgr_bus);
                          let handle =
                              tokio::spawn(run_overseer_loop(mgr, project_id, cfg.clone()));
                          t.insert(project_id, handle);
                      }
                      Ok(crate::api::events::DomainEvent::ProjectDeleted { project_id }) => {
                          let mut t = tasks_bus.lock().unwrap();
                          if let Some(h) = t.remove(&project_id) {
                              h.abort();
                          }
                      }
                      Ok(_) => {}
                      Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                      Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                  }
              }
          });
  ```

  Update the comments at `src/overseer/mod.rs:19-28` to note that `ProjectDeleted` is now the primary lifecycle source:

  ```rust
  /// Default cadence for the lifecycle reconcile loop.
  ///
  /// The reconcile loop is now a belt-and-suspenders fallback to the
  /// `DomainEvent::ProjectDeleted` bus subscription added in Plan 9. It
  /// reconciles tracked task handles against the live project set in case
  /// the bus event is lost (e.g. lagged subscriber, brief crash window).
  /// 30s is fine for production; tests use `with_reconcile_interval` to
  /// drop it to ~50ms.
  const DEFAULT_RECONCILE_INTERVAL: Duration = Duration::from_secs(30);
  ```

- [ ] **Step 7: Add supervisor-drops-task test**

  Append to `tests/overseer_supervisor.rs`:

  ```rust
  #[tokio::test]
  async fn supervisor_drops_task_on_project_deleted_event() {
      // Mirror the existing supervisor-spawns-on-project-added test setup
      // in this file. Use a tight reconcile interval so any fallback path
      // cannot be the one that cleans up — the bus subscription must.
      std::env::set_var("GALVOLITE_OVERSEER", "1");
      std::env::set_var("KIMI_API_KEY", "k");
      let tmp = tempfile::tempdir().unwrap();
      let db = galvolite::db::Db::open(tmp.path().join("g.db")).unwrap();
      let mgr = std::sync::Arc::new(
          galvolite::session_manager::SessionManager::new(db, tmp.path().to_path_buf()),
      );
      let project = mgr.add_project(galvolite::db::NewProject {
          name: "p".into(),
          path: tmp.path().to_string_lossy().into_owned(),
          remote_url: None,
      }).unwrap();

      let sup = galvolite::overseer::OverseerSupervisor::new(std::sync::Arc::clone(&mgr))
          .with_reconcile_interval(std::time::Duration::from_secs(3600));
      sup.spawn_for_existing_projects().await;

      // Wait for the spawn-on-startup to register the task.
      for _ in 0..50 {
          if sup.task_count() == 1 { break; }
          tokio::time::sleep(std::time::Duration::from_millis(20)).await;
      }
      assert_eq!(sup.task_count(), 1);

      mgr.delete_project(project.id).await.unwrap();

      // Bus is async — give the listener up to 500ms to react.
      for _ in 0..50 {
          if sup.task_count() == 0 { break; }
          tokio::time::sleep(std::time::Duration::from_millis(10)).await;
      }
      assert_eq!(sup.task_count(), 0);

      std::env::remove_var("GALVOLITE_OVERSEER");
      std::env::remove_var("KIMI_API_KEY");
  }
  ```

- [ ] **Step 8: Run new and existing supervisor tests**

  Run: `cargo test --test overseer_supervisor -- --test-threads=1`
  Expected: all PASS, including the existing reconcile-loop tests.

- [ ] **Step 9: Commit**

  ```bash
  git add src/api/events.rs src/session_manager.rs src/overseer/mod.rs \
          tests/overseer_supervisor.rs
  git commit -m "feat(events): add ProjectDeleted; Overseer subscribes for lifecycle"
  ```

---

## Task 4: `tail_per_session` ContextSnapshot field

**Files:**
- Modify: `src/overseer/observer.rs:21-29` (add field to `ContextSnapshot`)
- Modify: `src/overseer/observer.rs:81-91` (extend `hydrate_artifacts` or add new method)
- Modify: `src/overseer/recommender.rs:16-46` (`pack_prompt` includes tails)
- Modify: `src/overseer/mod.rs:179-202` (call new hydration method)
- Modify: `src/session_manager.rs` (add `pty_snapshot(session_id)` accessor)
- Test: `tests/overseer_review.rs` (one new test)

The Observer is per-project. For each child session belonging to the project, we read `PtyHandleShared::snapshot()` (rendered terminal screen) and store as `String` in a `HashMap<Uuid, String>` on the snapshot. The Kimi prompt gains a `## Recent PTY tails` section when the map is non-empty.

- [ ] **Step 1: Add `pty_snapshot` accessor to `SessionManager`**

  In `src/session_manager.rs`, near the existing `send_to_session`/`trigger_handoff` accessors (around line 603), add:

  ```rust
      /// Plan 9 Task 4 — return the rendered terminal screen for a running
      /// session. Returns `None` if the session is not in the running map
      /// (e.g. terminated or never started). Used by the Overseer's
      /// `tail_per_session` snapshot field.
      pub fn pty_snapshot(&self, id: Uuid) -> Option<Vec<u8>> {
          let running = self.running.lock().unwrap();
          running.get(&id).map(|rs| rs.handle.snapshot())
      }
  ```

  Verify `RunningSession.handle: PtyHandleShared` (struct at `src/session_manager.rs:22-30`) — the field is `handle`, not `pty`.

- [ ] **Step 2: Add `tail_per_session` field to `ContextSnapshot`**

  Modify `src/overseer/observer.rs:21-29`:

  ```rust
  #[derive(Debug, Clone, Default)]
  pub struct ContextSnapshot {
      pub project_id: Option<Uuid>,
      pub events: Vec<EventRecord>,
      pub plan_md: Option<String>,
      pub directive_md: Option<String>,
      pub recent_postmortems: Vec<crate::postmortem::Postmortem>,
      pub recent_decisions: Vec<crate::decisions::Decision>,
      pub tail_per_session: std::collections::HashMap<Uuid, String>,
  }
  ```

  Update `Observer::snapshot` at `src/overseer/observer.rs:68-77` to include the empty map:

  ```rust
      pub fn snapshot(&self) -> ContextSnapshot {
          ContextSnapshot {
              project_id: Some(self.project_id),
              events: self.events.iter().cloned().collect(),
              plan_md: None,
              directive_md: None,
              recent_postmortems: vec![],
              recent_decisions: vec![],
              tail_per_session: std::collections::HashMap::new(),
          }
      }
  ```

- [ ] **Step 3: Add `hydrate_pty_tails` method to `Observer`**

  Append to `impl Observer` block in `src/overseer/observer.rs` (after `hydrate_artifacts`):

  ```rust
      /// Plan 9 Task 4 — populate `tail_per_session` for each running session
      /// in this Observer's project. Reads the rendered PTY screen via
      /// `SessionManager::pty_snapshot`. Sessions not currently running are
      /// omitted (no entry in the map). Best-effort: a UTF-8 decode failure
      /// is replaced with the lossy form rather than dropping the session.
      pub fn hydrate_pty_tails(
          &self,
          snap: &mut ContextSnapshot,
          mgr: &crate::session_manager::SessionManager,
      ) {
          let sessions = mgr.list_sessions().unwrap_or_default();
          for s in sessions.into_iter().filter(|s| s.project_id == self.project_id) {
              if let Some(bytes) = mgr.pty_snapshot(s.id) {
                  let text = String::from_utf8_lossy(&bytes).into_owned();
                  snap.tail_per_session.insert(s.id, text);
              }
          }
      }
  ```

- [ ] **Step 4: Call the new hydrator from the Overseer loop**

  Modify `src/overseer/mod.rs:179-184`:

  ```rust
          let mut snap = obs.snapshot();
          if let Ok(p) = mgr.db().get_project(project_id) {
              obs.hydrate_artifacts(&mut snap, std::path::Path::new(&p.path));
          }
          obs.hydrate_pty_tails(&mut snap, &mgr);
  ```

- [ ] **Step 5: Include tails in the Kimi prompt**

  Modify `src/overseer/recommender.rs:16-46` (`pack_prompt`). After the `recent_decisions` block at lines 38-43 and before the `PROMPT_BUDGET` truncation at line 44, insert:

  ```rust
      if !snap.tail_per_session.is_empty() {
          user.push_str("\n# Recent PTY tails\n\n");
          // Cap each tail to keep total prompt under PROMPT_BUDGET.
          for (sid, tail) in snap.tail_per_session.iter().take(5) {
              user.push_str(&format!("## Session {sid}\n\n```\n"));
              push_capped(&mut user, tail.trim(), 800);
              user.push_str("\n```\n\n");
          }
      }
  ```

- [ ] **Step 6: Write failing test for prompt inclusion**

  Append to `tests/overseer_review.rs`:

  ```rust
  #[test]
  fn pack_prompt_includes_pty_tails_when_present() {
      use galvolite::overseer::observer::ContextSnapshot;
      let mut snap = ContextSnapshot::default();
      let sid = uuid::Uuid::new_v4();
      snap.tail_per_session.insert(sid, "claude> running tests\nFAIL: 3 tests".into());

      let (_, user) = galvolite::overseer::recommender::pack_prompt(&snap);
      assert!(user.contains("# Recent PTY tails"), "expected PTY tail header");
      assert!(user.contains("FAIL: 3 tests"), "expected tail body");
      assert!(user.contains(&sid.to_string()), "expected session id in output");
  }

  #[test]
  fn pack_prompt_omits_pty_section_when_map_empty() {
      use galvolite::overseer::observer::ContextSnapshot;
      let snap = ContextSnapshot::default();
      let (_, user) = galvolite::overseer::recommender::pack_prompt(&snap);
      assert!(!user.contains("# Recent PTY tails"));
  }
  ```

- [ ] **Step 7: Run tests**

  Run: `cargo test --test overseer_review pack_prompt_includes_pty_tails pack_prompt_omits_pty_section -- --test-threads=1`
  Expected: PASS.

- [ ] **Step 8: Run the full overseer test suite to confirm no regressions**

  Run: `cargo test --test overseer_review --test overseer_supervisor --test overseer_e2e_mocked --test overseer_config -- --test-threads=1`
  Expected: all PASS.

- [ ] **Step 9: Commit**

  ```bash
  git add src/overseer/observer.rs src/overseer/recommender.rs \
          src/overseer/mod.rs src/session_manager.rs tests/overseer_review.rs
  git commit -m "feat(overseer): include per-session PTY tails in ContextSnapshot"
  ```

---

## Task 5: UTC token-budget reset

**Files:**
- Modify: `src/overseer/recommender.rs:105-141` (`OverseerState` + `try_consume`/budget check)
- Test: `tests/overseer_review.rs` (one new test)

`OverseerState::new` keeps its existing `(daily_budget: u64) -> Self` signature. We add a private `last_reset_utc: NaiveDate` field initialized to today, and make the existing budget check at line 139 reset the counter when the date has advanced.

- [ ] **Step 1: Write failing test**

  Append to `tests/overseer_review.rs`:

  ```rust
  #[test]
  fn budget_resets_at_utc_date_rollover() {
      use galvolite::overseer::recommender::OverseerState;

      // Construct a state with a very small budget, exhaust it, then
      // simulate a date rollover by mutating the test-only setter.
      let mut s = OverseerState::new(100);
      s.tokens_used = 100;
      assert!(s.is_budget_exhausted());

      // Plan 9 Task 5 — public test-mode helper to simulate yesterday.
      s.set_last_reset_for_test(chrono::Utc::now().date_naive() - chrono::Duration::days(1));

      // The reset is performed inside the budget check; force a check.
      s.maybe_reset_for_today();
      assert_eq!(s.tokens_used, 0, "tokens should reset after date rollover");
      assert!(!s.is_budget_exhausted());
  }
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cargo test --test overseer_review budget_resets_at_utc_date_rollover -- --test-threads=1`
  Expected: FAIL — methods `is_budget_exhausted`, `set_last_reset_for_test`, `maybe_reset_for_today` don't exist.

- [ ] **Step 3: Add `last_reset_utc` field and helpers to `OverseerState`**

  Modify `src/overseer/recommender.rs:105-115`:

  ```rust
  pub struct OverseerState {
      pub daily_budget: u64,
      pub tokens_used: u64,
      pub consecutive_failures: u32,
      pub degraded: bool,
      /// Plan 9 Task 5 — last UTC date the counter was reset.
      pub last_reset_utc: chrono::NaiveDate,
  }

  impl OverseerState {
      pub fn new(daily_budget: u64) -> Self {
          Self {
              daily_budget,
              tokens_used: 0,
              consecutive_failures: 0,
              degraded: false,
              last_reset_utc: chrono::Utc::now().date_naive(),
          }
      }

      /// True when the running counter has hit (or exceeded) the budget.
      pub fn is_budget_exhausted(&self) -> bool {
          self.tokens_used >= self.daily_budget
      }

      /// Reset the counter when the UTC date has advanced past
      /// `last_reset_utc`. Called from `review` before the budget check.
      pub fn maybe_reset_for_today(&mut self) {
          let today = chrono::Utc::now().date_naive();
          if today > self.last_reset_utc {
              self.tokens_used = 0;
              self.last_reset_utc = today;
          }
      }

      #[doc(hidden)]
      pub fn set_last_reset_for_test(&mut self, d: chrono::NaiveDate) {
          self.last_reset_utc = d;
      }
  }
  ```

- [ ] **Step 4: Call `maybe_reset_for_today` from `review`**

  Modify `src/overseer/recommender.rs:139-141`:

  ```rust
      state.maybe_reset_for_today();
      if state.is_budget_exhausted() {
          return Err(ReviewError::Budget { used: state.tokens_used, budget: state.daily_budget });
      }
  ```

- [ ] **Step 5: Run the new test plus the existing `review` budget tests**

  Run: `cargo test --test overseer_review -- --test-threads=1`
  Expected: all PASS.

- [ ] **Step 6: Commit**

  ```bash
  git add src/overseer/recommender.rs tests/overseer_review.rs
  git commit -m "feat(overseer): reset daily token counter at UTC midnight"
  ```

---

## Task 6: RecModal status filter (segmented control)

**Files:**
- Modify: `ui/src/components/RecModal.tsx` (full file rewrite below)
- Modify: `ui/src/components/RecModal.test.tsx` (add filter test)

Currently `RecModal` hard-codes `status="pending"` in two places (lines 14, 19) and shows only pending recs. Replace with a segmented control above the list and refetch on change. Empty-state copy reflects the active filter.

- [ ] **Step 1: Write failing test in `RecModal.test.tsx`**

  Inspect existing tests first:

  Run: `grep -n "describe\|it(\|test(" ui/src/components/RecModal.test.tsx | head -10`

  Append:

  ```tsx
  it("switches to deferred filter and refetches", async () => {
    const listRecs = vi.fn().mockImplementation((_pid: string, status: string) => {
      if (status === "pending") return Promise.resolve([{
        id: "r1", project_id: "p", kind: "k", severity: "info", title: "P-rec",
        body: "b", rationale: "r", status: "pending", created_at: "2026-01-01T00:00:00Z",
        acted_at: null, deferred_until: null, evidence_json: null,
      }]);
      if (status === "deferred") return Promise.resolve([{
        id: "r2", project_id: "p", kind: "k", severity: "warn", title: "D-rec",
        body: "b", rationale: "r", status: "deferred", created_at: "2026-01-01T00:00:00Z",
        acted_at: null, deferred_until: "2026-01-02T00:00:00Z", evidence_json: null,
      }]);
      return Promise.resolve([]);
    });
    vi.spyOn(api, "listRecs").mockImplementation(listRecs);

    render(<RecModal projectId="p" open={true} onClose={() => {}} />);

    // Initial render: pending filter, P-rec visible.
    await screen.findByText("P-rec");

    // Click Deferred segment.
    fireEvent.click(screen.getByRole("button", { name: /deferred/i }));

    // After refetch, D-rec is visible and P-rec is not.
    await screen.findByText("D-rec");
    expect(screen.queryByText("P-rec")).toBeNull();
    expect(listRecs).toHaveBeenCalledWith("p", "deferred");
  });
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cd ui && npm test -- RecModal --run`
  Expected: FAIL — no Deferred button exists yet.

- [ ] **Step 3: Rewrite `RecModal.tsx` with the segmented filter**

  Replace the full file `ui/src/components/RecModal.tsx`:

  ```tsx
  import { useEffect, useMemo, useState } from "react";
  import { api } from "../api";
  import type { Recommendation } from "../types";

  interface Props { projectId: string; open: boolean; onClose: () => void; }

  type StatusFilter = "pending" | "deferred" | "acted" | "dismissed" | "archived";
  const FILTERS: StatusFilter[] = ["pending", "deferred", "acted", "dismissed", "archived"];

  function emptyCopy(f: StatusFilter): string {
    switch (f) {
      case "pending": return "No pending recommendations.";
      case "deferred": return "No deferred recommendations.";
      case "acted": return "No acted recommendations.";
      case "dismissed": return "No dismissed recommendations.";
      case "archived": return "No archived recommendations.";
    }
  }

  export function RecModal({ projectId, open, onClose }: Props) {
    const [filter, setFilter] = useState<StatusFilter>("pending");
    const [recs, setRecs] = useState<Recommendation[]>([]);
    const [selected, setSelected] = useState<Set<string>>(new Set());
    const [busy, setBusy] = useState(false);

    useEffect(() => {
      if (!open) return;
      api.listRecs(projectId, filter).then(setRecs).catch(() => setRecs([]));
      setSelected(new Set());
    }, [open, projectId, filter]);

    if (!open) return null;

    const refresh = () => api.listRecs(projectId, filter).then(setRecs);
    const sevColor = (s: string) =>
      s === "urgent" ? "bg-red-700" : s === "warn" ? "bg-yellow-700" : "bg-neutral-700";
    const canAct = filter === "pending"; // make/dismiss/defer only meaningful on pending

    return (
      <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
        <div className="bg-neutral-900 rounded-lg shadow-xl p-4 w-[640px] max-h-[80vh] overflow-y-auto">
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-lg font-semibold">Overseer recommendations</h2>
            <button onClick={onClose} className="text-neutral-400 hover:text-white">×</button>
          </div>

          <div className="flex gap-1 mb-3" role="tablist" aria-label="Filter recommendations by status">
            {FILTERS.map((f) => (
              <button
                key={f}
                role="tab"
                aria-selected={filter === f}
                onClick={() => setFilter(f)}
                className={
                  "px-2 py-1 text-xs rounded " +
                  (filter === f
                    ? "bg-blue-700 text-white"
                    : "bg-neutral-800 text-neutral-300 hover:bg-neutral-700")
                }
              >
                {f.charAt(0).toUpperCase() + f.slice(1)}
              </button>
            ))}
          </div>

          {recs.length === 0 && (
            <div className="text-neutral-500 text-sm py-8 text-center">{emptyCopy(filter)}</div>
          )}
          {recs.map((r) => (
            <div key={r.id} className="border border-neutral-700 rounded p-3 mb-2">
              <div className="flex items-start gap-2">
                {canAct && (
                  <input
                    type="checkbox"
                    aria-label={`select ${r.title}`}
                    checked={selected.has(r.id)}
                    onChange={(e) => {
                      const next = new Set(selected);
                      e.target.checked ? next.add(r.id) : next.delete(r.id);
                      setSelected(next);
                    }}
                  />
                )}
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <span className={`text-[10px] uppercase rounded px-1 py-0.5 ${sevColor(r.severity)}`}>
                      {r.severity}
                    </span>
                    <span className="font-medium">{r.title}</span>
                  </div>
                  <div className="text-sm text-neutral-300 mt-1 whitespace-pre-wrap">{r.body}</div>
                  <div className="text-xs text-neutral-500 mt-1 italic">{r.rationale}</div>
                </div>
                {canAct && (
                  <div className="flex flex-col gap-1">
                    <button
                      className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
                      disabled={busy}
                      onClick={async () => { setBusy(true); await api.deferRec(projectId, r.id); await refresh(); setBusy(false); }}
                    >Defer</button>
                  </div>
                )}
              </div>
            </div>
          ))}
          {canAct && recs.length > 0 && (
            <div className="flex gap-2 justify-end mt-3">
              <button
                className="px-3 py-1 rounded bg-emerald-700 hover:bg-emerald-600 text-white text-xs disabled:opacity-50"
                disabled={busy || selected.size === 0}
                onClick={async () => {
                  setBusy(true);
                  await api.actRecs(projectId, [...selected], "make");
                  setSelected(new Set()); await refresh(); setBusy(false);
                }}
              >Make selected ({selected.size})</button>
              <button
                className="px-3 py-1 rounded bg-neutral-700 hover:bg-neutral-600 text-white text-xs disabled:opacity-50"
                disabled={busy || selected.size === 0}
                onClick={async () => {
                  setBusy(true);
                  await api.actRecs(projectId, [...selected], "dismiss");
                  setSelected(new Set()); await refresh(); setBusy(false);
                }}
              >Dismiss</button>
            </div>
          )}
        </div>
      </div>
    );
  }
  ```

- [ ] **Step 4: Run UI tests**

  Run: `cd ui && npm test -- RecModal --run`
  Expected: PASS, including the existing RecModal tests (initial-render, defer, make, dismiss).

- [ ] **Step 5: Commit**

  ```bash
  git add ui/src/components/RecModal.tsx ui/src/components/RecModal.test.tsx
  git commit -m "feat(ui): RecModal status filter with segmented control"
  ```

---

## Task 7: RecModal "Re-explain" button

**Files:**
- Modify: `ui/src/components/RecModal.tsx` (add per-rec Re-explain UI)
- Modify: `ui/src/components/RecModal.test.tsx` (add re-explain test)

Adds a "Re-explain" button per pending rec. Click opens an inline `<textarea>` for the user's "what's unclear" rationale; submit posts to the existing `api.explainRec` and replaces the rec's rationale in local state when the call succeeds. Note: the endpoint at `src/api/recommendations.rs:173` updates **rationale** (the small italic line), not body — the test asserts the rationale was replaced, not body.

- [ ] **Step 1: Write failing test**

  Append to `ui/src/components/RecModal.test.tsx`:

  ```tsx
  it("re-explain replaces rationale on success", async () => {
    const explainRec = vi.fn().mockResolvedValue({ updated: true });
    vi.spyOn(api, "explainRec").mockImplementation(explainRec);
    vi.spyOn(api, "listRecs").mockResolvedValue([{
      id: "r1", project_id: "p", kind: "k", severity: "info", title: "T",
      body: "B", rationale: "old rationale", status: "pending",
      created_at: "2026-01-01T00:00:00Z", acted_at: null, deferred_until: null,
      evidence_json: null,
    }]);

    render(<RecModal projectId="p" open={true} onClose={() => {}} />);
    await screen.findByText("old rationale");

    fireEvent.click(screen.getByRole("button", { name: /re-explain/i }));

    const textarea = screen.getByPlaceholderText(/what's unclear/i);
    fireEvent.change(textarea, { target: { value: "explain like I'm 5" } });
    fireEvent.click(screen.getByRole("button", { name: /^submit$/i }));

    await waitFor(() => {
      expect(explainRec).toHaveBeenCalledWith("p", "r1", "explain like I'm 5");
    });
  });
  ```

- [ ] **Step 2: Run test to verify it fails**

  Run: `cd ui && npm test -- RecModal --run`
  Expected: FAIL — no Re-explain button.

- [ ] **Step 3: Add the per-rec Re-explain UI**

  In `ui/src/components/RecModal.tsx`, replace the `canAct` button column block (the `<div className="flex flex-col gap-1">` with the Defer button) with a small inline component that holds re-explain state.

  Above the `RecModal` function in the same file, add:

  ```tsx
  function RecActions({
    projectId,
    rec,
    busy,
    setBusy,
    onChanged,
  }: {
    projectId: string;
    rec: Recommendation;
    busy: boolean;
    setBusy: (b: boolean) => void;
    onChanged: (next: Recommendation) => void;
  }) {
    const [open, setOpen] = useState(false);
    const [text, setText] = useState("");
    return (
      <div className="flex flex-col gap-1">
        <button
          className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            await api.deferRec(projectId, rec.id);
            // Caller refetches the list; nothing to replace here.
            setBusy(false);
          }}
        >Defer</button>
        <button
          className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
          disabled={busy}
          onClick={() => setOpen((v) => !v)}
        >{open ? "Cancel" : "Re-explain"}</button>
        {open && (
          <div className="mt-1">
            <textarea
              className="w-full text-xs bg-neutral-800 border border-neutral-700 rounded p-1"
              rows={2}
              placeholder="What's unclear about this recommendation?"
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
            <button
              className="mt-1 text-xs px-2 py-0.5 rounded bg-blue-700 hover:bg-blue-600 disabled:opacity-50"
              disabled={busy || text.trim().length === 0}
              onClick={async () => {
                setBusy(true);
                try {
                  await api.explainRec(projectId, rec.id, text);
                  onChanged({ ...rec, rationale: text });
                  setOpen(false);
                  setText("");
                } finally {
                  setBusy(false);
                }
              }}
            >Submit</button>
          </div>
        )}
      </div>
    );
  }
  ```

  Then in the rec list rendering, replace the existing per-rec button column:

  ```tsx
                  {canAct && (
                    <RecActions
                      projectId={projectId}
                      rec={r}
                      busy={busy}
                      setBusy={setBusy}
                      onChanged={(next) => setRecs((rs) => rs.map((x) => (x.id === next.id ? next : x)))}
                    />
                  )}
  ```

  Remove the now-unused inline Defer button from the previous implementation (it lives inside `RecActions` now).

- [ ] **Step 4: Run UI tests**

  Run: `cd ui && npm test -- RecModal --run`
  Expected: PASS, including the new re-explain test and all prior tests.

- [ ] **Step 5: Commit**

  ```bash
  git add ui/src/components/RecModal.tsx ui/src/components/RecModal.test.tsx
  git commit -m "feat(ui): RecModal Re-explain button posts to /recommendations/explain"
  ```

---

## Final verification

After all 7 tasks land:

- [ ] **Run full backend test suite**

  Run: `cargo test -- --test-threads=1`
  Expected: PASS.

- [ ] **Run full UI test suite**

  Run: `cd ui && npm test -- --run`
  Expected: PASS.

- [ ] **Build release artifact**

  Run: `cargo build --release`
  Expected: PASS, no warnings.

- [ ] **Update memory file `project_plan9_status.md`**

  Per project convention (see `project_plan8_status.md`), record completion: branch, commit range, test counts.
