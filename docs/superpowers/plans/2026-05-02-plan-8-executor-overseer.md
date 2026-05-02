# Plan 8 — Plan Executor + Overseer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship two intertwined subsystems — (a) `POST /api/projects/:id/execute-plan` that derives a spawn graph from `plan.md` + `directive.md` and walks phases honoring autonomy tiers, (b) per-project Overseer background task using Kimi K2 that watches the bus + artifacts and emits actionable recommendations the operator can dispatch through the orchestrator.

**Architecture:** Plan Executor is a stateless module + REST handler; once children are spawned the existing fan-out/rejoin machinery owns them. Overseer is a long-lived `tokio::spawn` per project that subscribes to the existing `EventBroadcaster`, maintains a `ContextSnapshot`, calls Kimi on debounced triggers, and persists recommendations to a new `recommendations` table. Operator "Make" actions append rendered rec text to a new `sessions.pending_overseer_recs` column that mirrors the existing `pending_rejoin` lane; the orchestrator's `UserPromptSubmit` hook fetches + clears it on its next turn.

**Tech Stack:** Rust 2021 (axum 0.7, tokio, rusqlite, reqwest, mockito for tests), React 18 + TypeScript + vitest UI, sqlite via r2d2.

**Spec:** `docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md`

**Note on size:** 25 tasks. If executed via subagent-driven-development, dispatch tasks 1–8 (foundations + Plan Executor) as one cycle, then 9–25 (Overseer + UI + wiring) as a second cycle. Both subsystems must ship together to honor the brainstorm decision (spec §7 row 5), but the two halves don't share tight integration until Task 20 (orchestrator wiring) and Task 25 (smoke).

---

## File Map

**New Rust files:**
- `migrations/0004_overseer_executor.sql` — schema delta
- `src/plan_executor.rs` — pure spawn-graph derivation + execute-loop
- `src/api/execute_plan.rs` — `POST /api/projects/:id/execute-plan` + phase-approve handler
- `src/recommendations.rs` — DB CRUD + content_hash dedupe
- `src/api/recommendations.rs` — REST list/act/defer/explain
- `src/overseer/mod.rs` — `OverseerSupervisor` + `OverseerTask` lifecycle
- `src/overseer/observer.rs` — bus subscriber + `ContextSnapshot` ring buffers
- `src/overseer/kimi.rs` — Kimi K2 HTTP client (OpenAI-compat)
- `src/overseer/recommender.rs` — `pack_prompt` + `parse_kimi_response` + `review` driver
- `tests/plan_executor_*.rs`, `tests/overseer_*.rs`, `tests/recommendations_*.rs`

**Modified Rust files:**
- `src/lib.rs` — register new modules
- `src/db.rs` — append migration 0004 to `MIGRATIONS`; add `pending_overseer_recs` to `Session` struct + read paths
- `src/api/mod.rs` — register new routes
- `src/api/events.rs` — extend `DomainEvent` with 6 new variants
- `src/api/hook_handlers/turn.rs` — extend `on_user_prompt_submit` to fetch/clear `pending_overseer_recs` for orchestrator sessions
- `src/claude_md.rs` — add rec-routing rule to `ORCH_PROFILE`
- `src/main.rs` — start `OverseerSupervisor` after `SessionManager` init; start phase-advance reactor
- `src/session_manager.rs` — add `set_pending_overseer_recs(append: true)` helper

**New TypeScript files:**
- `ui/src/components/ExecutePlanButton.tsx`
- `ui/src/components/RecBell.tsx`
- `ui/src/components/RecModal.tsx`
- `ui/src/components/RecBell.test.tsx`
- `ui/src/components/RecModal.test.tsx`
- `ui/src/components/ExecutePlanButton.test.tsx`

**Modified TypeScript files:**
- `ui/src/types.ts` — add `Recommendation`, `Phase`, new `DomainEvent` variants
- `ui/src/api.ts` — add executePlan, listRecs, actRecs, deferRec, explainRec, approvePhase
- `ui/src/components/Dashboard.tsx` — mount `<RecBell>`, replace/sit-beside SpawnControls with `<ExecutePlanButton>`

---

## Task 1: DB migration 0004 — recommendations + pending_overseer_recs

**Files:**
- Create: `migrations/0004_overseer_executor.sql`
- Modify: `src/db.rs` (lines 13–23 — `MIGRATIONS` const)
- Modify: `src/db.rs` (lines 142–162 — `Session` struct + reader)
- Test: `tests/db_v3_pre_overseer_migration.rs` (new — verify migration applies idempotently and adds the column)

- [ ] **Step 1: Write the failing test**

Create `tests/db_v3_pre_overseer_migration.rs`:

```rust
use galvolite::db::Db;

#[test]
fn migration_0004_adds_recommendations_and_pending_overseer_recs() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let conn = db.pool().get().unwrap();

    let cnt: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='recommendations'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(cnt, 1, "recommendations table missing");

    let has_col: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info('sessions') WHERE name='pending_overseer_recs'",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(has_col, 1, "sessions.pending_overseer_recs missing");
}

#[test]
fn migration_0004_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("g.db");
    let _ = Db::open(&path).unwrap();
    // Reopen — should not re-apply.
    let db = Db::open(&path).unwrap();
    let conn = db.pool().get().unwrap();
    let n: i64 = conn.query_row(
        "SELECT count(*) FROM schema_version WHERE version = 4",
        [], |r| r.get(0),
    ).unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test db_v3_pre_overseer_migration`
Expected: FAIL with "no such table: recommendations" or "no such column".

- [ ] **Step 3: Create migration file**

Create `migrations/0004_overseer_executor.sql`:

```sql
-- Plan 8 — Overseer + Plan Executor schema delta (schema v0004)
-- See docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md §3.

CREATE TABLE recommendations (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL,
    kind            TEXT NOT NULL,
    severity        TEXT NOT NULL CHECK (severity IN ('info','warn','urgent')),
    title           TEXT NOT NULL,
    body            TEXT NOT NULL,
    rationale       TEXT NOT NULL,
    evidence_json   TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('pending','acted','dismissed','deferred')),
    created_at      TEXT NOT NULL,
    deferred_until  TEXT NULL,
    acted_at        TEXT NULL,
    content_hash    TEXT NOT NULL UNIQUE
);
CREATE INDEX recommendations_project_status_idx
    ON recommendations(project_id, status, created_at);

ALTER TABLE sessions ADD COLUMN pending_overseer_recs TEXT NULL;
```

- [ ] **Step 4: Register migration in src/db.rs**

In `src/db.rs`, edit the `MIGRATIONS` block:

```rust
const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");
const SCHEMA_V2: &str = include_str!("../migrations/0002_hooks_mcp_foundations.sql");
const SCHEMA_V3: &str = include_str!("../migrations/0003_session_spawn.sql");
const SCHEMA_V4: &str = include_str!("../migrations/0004_overseer_executor.sql");

const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, SCHEMA_V2),
    (3, SCHEMA_V3),
    (4, SCHEMA_V4),
];
```

- [ ] **Step 5: Add `pending_overseer_recs` field to `Session`**

In `src/db.rs`, add to `Session`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
pub pending_overseer_recs: Option<String>,
```

Add to `insert_session`'s returned struct: `pending_overseer_recs: None`.

In `get_session` and `list_sessions` SELECT lists, append `, pending_overseer_recs` to the column list and bump the index. Read it as `r.get(15)?` (after `terminal_status` at index 14).

- [ ] **Step 6: Run tests**

Run: `cargo test --test db_v3_pre_overseer_migration`
Expected: PASS (both tests).

Run: `cargo test --test db_init` (existing migration test must still pass).
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add migrations/0004_overseer_executor.sql src/db.rs tests/db_v3_pre_overseer_migration.rs
git commit -m "feat(db): migration 0004 — recommendations table + pending_overseer_recs"
```

---

## Task 2: Feature flags + Kimi config helper

**Files:**
- Create: `src/overseer/config.rs` (will move into `overseer/` later; make `pub mod config;` from a stub `src/overseer/mod.rs`)
- Test: `tests/overseer_config.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/overseer_config.rs`:

```rust
use galvolite::overseer::config::{OverseerConfig, executor_enabled, overseer_enabled};

#[test]
fn defaults_off_when_env_unset() {
    std::env::remove_var("GALVOLITE_OVERSEER");
    std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
    assert!(!overseer_enabled());
    assert!(!executor_enabled());
}

#[test]
fn enabled_when_env_set_truthy() {
    std::env::set_var("GALVOLITE_OVERSEER", "1");
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "true");
    assert!(overseer_enabled());
    assert!(executor_enabled());
    std::env::remove_var("GALVOLITE_OVERSEER");
    std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
}

#[test]
fn config_from_env_returns_none_when_key_missing() {
    std::env::remove_var("KIMI_API_KEY");
    std::env::remove_var("KIMI_BASE_URL");
    assert!(OverseerConfig::from_env().is_none());
}

#[test]
fn config_from_env_uses_default_base_url_when_only_key_set() {
    std::env::set_var("KIMI_API_KEY", "k");
    std::env::remove_var("KIMI_BASE_URL");
    let c = OverseerConfig::from_env().unwrap();
    assert_eq!(c.api_key, "k");
    assert_eq!(c.base_url, "https://api.moonshot.cn/v1");
    assert_eq!(c.daily_token_budget, 100_000);
    std::env::remove_var("KIMI_API_KEY");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test overseer_config`
Expected: FAIL — `overseer::config` module does not exist.

- [ ] **Step 3: Create stub module + config**

Create `src/overseer/mod.rs`:

```rust
//! Overseer subsystem — per-project background task using Kimi K2.
//! See docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md §3.

pub mod config;
```

Create `src/overseer/config.rs`:

```rust
//! Env-config helpers for Overseer + Plan Executor feature flags
//! and Kimi K2 API credentials.

const DEFAULT_KIMI_BASE_URL: &str = "https://api.moonshot.cn/v1";
const DEFAULT_DAILY_TOKEN_BUDGET: u64 = 100_000;

pub fn overseer_enabled() -> bool {
    truthy(std::env::var("GALVOLITE_OVERSEER").ok().as_deref())
}

pub fn executor_enabled() -> bool {
    truthy(std::env::var("GALVOLITE_PLAN_EXECUTOR").ok().as_deref())
}

fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(str::to_ascii_lowercase).as_deref(), Some("1" | "true" | "yes" | "on"))
}

#[derive(Debug, Clone)]
pub struct OverseerConfig {
    pub api_key: String,
    pub base_url: String,
    pub daily_token_budget: u64,
}

impl OverseerConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("KIMI_API_KEY").ok()?;
        let base_url = std::env::var("KIMI_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_KIMI_BASE_URL.to_string());
        let daily_token_budget = std::env::var("KIMI_DAILY_TOKEN_BUDGET")
            .ok().and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_DAILY_TOKEN_BUDGET);
        Some(Self { api_key, base_url, daily_token_budget })
    }
}
```

Register in `src/lib.rs`:

```rust
pub mod overseer;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test overseer_config`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/lib.rs src/overseer/mod.rs src/overseer/config.rs tests/overseer_config.rs
git commit -m "feat(overseer): config helper + feature-flag accessors"
```

---

## Task 3: Plan Executor — derive_spawn_graph (pure logic)

**Files:**
- Create: `src/plan_executor.rs`
- Modify: `src/lib.rs` (register module)
- Test: `tests/plan_executor_derive.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/plan_executor_derive.rs`:

```rust
use galvolite::plan_executor::{derive_spawn_graph, DeriveError, SpawnGraph};

const PLAN: &str = "\
- [ ] First step
- [ ] Second step
- [ ] Third step
";

const DIRECTIVE: &str = "\
# Directive: demo
Plan: .galvolite/plan.md

## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | full | — |
| p2 | 2, 3 | review | p1 |

## Notes
none
";

#[test]
fn happy_path_returns_phases_in_topo_order() {
    let g: SpawnGraph = derive_spawn_graph(PLAN, DIRECTIVE).unwrap();
    assert_eq!(g.phases.len(), 2);
    assert_eq!(g.phases[0].id, "p1");
    assert_eq!(g.phases[0].part_titles, vec![(1u32, "First step".to_string())]);
    assert_eq!(g.phases[1].id, "p2");
    assert_eq!(
        g.phases[1].part_titles,
        vec![(2u32, "Second step".into()), (3u32, "Third step".into())]
    );
}

#[test]
fn orphan_part_rejected() {
    let bad = "\
# Directive: x
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 7 | full | — |
## Notes
";
    let err = derive_spawn_graph(PLAN, bad).unwrap_err();
    assert!(matches!(err, DeriveError::OrphanParts(parts) if parts == vec![7u32]));
}

#[test]
fn cycle_rejected() {
    let bad = "\
# Directive: x
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | full | p2 |
| p2 | 2 | full | p1 |
## Notes
";
    let err = derive_spawn_graph(PLAN, bad).unwrap_err();
    assert!(matches!(err, DeriveError::Directive(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test plan_executor_derive`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement plan_executor::derive_spawn_graph**

Create `src/plan_executor.rs`:

```rust
//! Plan Executor — pure derivation of spawn graph from plan.md + directive.md.
//! Spec: docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md §3, §4 Flow A.

use crate::directive::{self, Autonomy, Directive, ParseError as DirectiveError};
use crate::plan_steps::parse_plan_markdown;

#[derive(Debug, thiserror::Error)]
pub enum DeriveError {
    #[error("directive parse: {0}")]
    Directive(#[from] DirectiveError),
    #[error("parts referenced in directive but absent from plan.md: {0:?}")]
    OrphanParts(Vec<u32>),
}

#[derive(Debug, Clone)]
pub struct SpawnGraph {
    pub phases: Vec<PhasePlan>,
}

#[derive(Debug, Clone)]
pub struct PhasePlan {
    pub id: String,
    pub autonomy: Autonomy,
    pub depends_on: Vec<String>,
    /// `(part_number, step_title)` pairs in the order they appear in the directive.
    pub part_titles: Vec<(u32, String)>,
}

pub fn derive_spawn_graph(plan_md: &str, directive_md: &str) -> Result<SpawnGraph, DeriveError> {
    let dir: Directive = directive::parse(directive_md)?;
    let steps = parse_plan_markdown(plan_md);

    // 1-based part lookup: ordinal+1 → title.
    let title_for_part: std::collections::HashMap<u32, &str> = steps
        .iter()
        .map(|s| ((s.ordinal as u32) + 1, s.title.as_str()))
        .collect();

    // Validate every referenced part exists.
    let mut orphans: Vec<u32> = Vec::new();
    for phase in &dir.phases {
        for p in &phase.parts {
            if !title_for_part.contains_key(p) { orphans.push(*p); }
        }
    }
    if !orphans.is_empty() {
        orphans.sort_unstable();
        orphans.dedup();
        return Err(DeriveError::OrphanParts(orphans));
    }

    // Topological sort over phase deps. (Directive parser already rejected cycles.)
    let ordered = topo_sort(&dir.phases);
    let phases = ordered
        .into_iter()
        .map(|ph| PhasePlan {
            id: ph.id.clone(),
            autonomy: ph.autonomy,
            depends_on: ph.depends_on.clone(),
            part_titles: ph
                .parts
                .iter()
                .map(|p| (*p, title_for_part[p].to_string()))
                .collect(),
        })
        .collect();

    Ok(SpawnGraph { phases })
}

fn topo_sort(phases: &[directive::Phase]) -> Vec<&directive::Phase> {
    use std::collections::{HashMap, HashSet};
    let by_id: HashMap<&str, &directive::Phase> =
        phases.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut visited: HashSet<&str> = HashSet::new();
    let mut out: Vec<&directive::Phase> = Vec::new();
    fn visit<'a>(
        p: &'a directive::Phase,
        by_id: &HashMap<&'a str, &'a directive::Phase>,
        visited: &mut HashSet<&'a str>,
        out: &mut Vec<&'a directive::Phase>,
    ) {
        if visited.contains(p.id.as_str()) { return; }
        visited.insert(p.id.as_str());
        for dep in &p.depends_on {
            if let Some(dp) = by_id.get(dep.as_str()) {
                visit(dp, by_id, visited, out);
            }
        }
        out.push(p);
    }
    for p in phases { visit(p, &by_id, &mut visited, &mut out); }
    out
}
```

Register in `src/lib.rs`:

```rust
pub mod plan_executor;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test plan_executor_derive`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/plan_executor.rs src/lib.rs tests/plan_executor_derive.rs
git commit -m "feat(plan-executor): derive_spawn_graph pure logic"
```

---

## Task 4: Plan Executor — execute() driver + REST handler

**Files:**
- Modify: `src/plan_executor.rs` (add `execute_plan` async fn)
- Create: `src/api/execute_plan.rs`
- Modify: `src/api/mod.rs` (register module + route)
- Modify: `src/session_manager.rs` (add `find_orchestrator_for_project` helper)
- Modify: `src/api/events.rs` (add `PhaseReviewNeeded` + `PhaseAdvanced` variants)
- Test: `tests/execute_plan_e2e.rs`, `tests/execute_plan_validation.rs`

- [ ] **Step 1: Extend DomainEvent**

In `src/api/events.rs`, add to the `DomainEvent` enum:

```rust
#[serde(rename = "phase.review_needed")]
PhaseReviewNeeded { project_id: Uuid, phase_id: String },
#[serde(rename = "phase.advanced")]
PhaseAdvanced { project_id: Uuid, phase_id: String },
```

Extend `applies_to`:

```rust
DomainEvent::PhaseReviewNeeded { .. } | DomainEvent::PhaseAdvanced { .. } => false,
```

- [ ] **Step 2: Write the failing happy-path test**

Create `tests/execute_plan_e2e.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;

#[tokio::test]
async fn execute_plan_spawns_one_child_per_part_in_topo_order() {
    let app = spawn_test_app().await;
    let project = app.create_project_with_plan_and_directive(
        "\
- [ ] alpha
- [ ] beta
- [ ] gamma
",
        "\
# Directive: demo
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | manual | — |
| p2 | 2, 3 | manual | p1 |
## Notes
",
    ).await;

    // Spawn an orchestrator so the executor has a parent to attach children to.
    app.spawn_orchestrator(project.id).await;

    let resp = app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;
    assert_eq!(resp.status(), 200);

    // First phase only — autonomy=manual stops at boundary.
    let kids = app.list_children_of_orch(project.id).await;
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].task_summary, "alpha");
}
```

(`common::test_app` is the existing test harness used by `tests/api_*.rs`; reuse `spawn_test_app`. If a helper named exactly `create_project_with_plan_and_directive` doesn't exist yet, add it as part of this task to `tests/common/test_app.rs` — see Step 4.)

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --test execute_plan_e2e`
Expected: FAIL — endpoint missing → 404.

- [ ] **Step 4: Add test-app helpers if missing**

In `tests/common/test_app.rs`, add (paraphrased — adapt to existing helper signatures):

```rust
impl TestApp {
    pub async fn create_project_with_plan_and_directive(
        &self,
        plan_md: &str,
        directive_md: &str,
    ) -> galvolite::db::Project {
        let project = self.create_project_in_temp_repo("plan8").await;
        let dir = std::path::Path::new(&project.path).join(".galvolite");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plan.md"), plan_md).unwrap();
        std::fs::write(dir.join("directive.md"), directive_md).unwrap();
        project
    }

    pub async fn spawn_orchestrator(&self, project_id: uuid::Uuid) -> galvolite::db::Session {
        self.mgr
            .spawn_session(project_id, "orchestrator task".into(), "orchestrator".into(),
                           "true".into(), vec![]).await.unwrap()
    }

    pub async fn list_children_of_orch(&self, project_id: uuid::Uuid) -> Vec<galvolite::db::Session> {
        self.mgr.list_sessions().unwrap()
            .into_iter()
            .filter(|s| s.project_id == project_id && s.parent_session_id.is_some())
            .collect()
    }
}
```

- [ ] **Step 5: Add SessionManager helper**

In `src/session_manager.rs`, add:

```rust
/// Find the most recent non-terminal orchestrator session for a project.
/// Returns `None` if no orchestrator is currently running.
pub fn find_orchestrator_for_project(&self, project_id: uuid::Uuid) -> Option<crate::db::Session> {
    self.db.list_sessions().ok()?.into_iter()
        .filter(|s| s.project_id == project_id && s.role == "orchestrator" && s.ended_at.is_none())
        .max_by_key(|s| s.created_at)
}
```

- [ ] **Step 6: Implement execute_plan driver**

Append to `src/plan_executor.rs`:

```rust
use crate::api::events::{DomainEvent, EventBroadcaster};
use crate::db::Db;
use crate::session_manager::SessionManager;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error("missing artifact: {0}")]
    MissingArtifact(&'static str),
    #[error("derive: {0}")]
    Derive(#[from] DeriveError),
    #[error("no orchestrator session for project {0}")]
    NoOrchestrator(Uuid),
    #[error("orchestrator already has running children — refusing to re-execute")]
    AlreadyRunning,
    #[error("spawn failed: {0}")]
    Spawn(#[from] crate::error::Error),
}

pub async fn execute_plan(
    mgr: Arc<SessionManager>,
    project_id: Uuid,
) -> Result<Vec<Uuid>, ExecuteError> {
    let project = mgr.db().get_project(project_id).map_err(ExecuteError::Spawn)?;
    let proj_path = std::path::Path::new(&project.path);
    let plan = std::fs::read_to_string(proj_path.join(".galvolite/plan.md"))
        .map_err(|_| ExecuteError::MissingArtifact("plan.md"))?;
    let directive = std::fs::read_to_string(proj_path.join(".galvolite/directive.md"))
        .map_err(|_| ExecuteError::MissingArtifact("directive.md"))?;

    let graph = derive_spawn_graph(&plan, &directive)?;

    let orch = mgr.find_orchestrator_for_project(project_id)
        .ok_or(ExecuteError::NoOrchestrator(project_id))?;

    // Idempotency: refuse if any non-terminal child already exists for this orch.
    let has_running = mgr.db().list_sessions()?.iter()
        .any(|s| s.parent_session_id == Some(orch.id) && s.ended_at.is_none());
    if has_running { return Err(ExecuteError::AlreadyRunning); }

    let bus = mgr.event_bus().clone();
    let mut spawned: Vec<Uuid> = Vec::new();

    // Spawn ONLY the first phase synchronously here; subsequent phases are
    // advanced by the phase reactor (Task 6) on rejoin or operator approval.
    if let Some(first) = graph.phases.first() {
        let kids = spawn_phase(&mgr, &orch, first, &bus, project_id).await?;
        spawned.extend(kids.iter().copied());
        mgr.set_waiting_on(orch.id, &kids).await?;
        store_pending_phases(mgr.db(), orch.id, &graph.phases[1..])?;
        if matches!(first.autonomy, Autonomy::Review | Autonomy::Manual) {
            bus.publish(DomainEvent::PhaseReviewNeeded {
                project_id, phase_id: first.id.clone(),
            });
        }
    }
    Ok(spawned)
}

async fn spawn_phase(
    mgr: &Arc<SessionManager>,
    orch: &crate::db::Session,
    phase: &PhasePlan,
    bus: &EventBroadcaster,
    project_id: Uuid,
) -> Result<Vec<Uuid>, crate::error::Error> {
    let mut kids = Vec::new();
    for (part, title) in &phase.part_titles {
        let s = mgr.spawn_child(orch.id, "implementation", title, Some(*part)).await?;
        kids.push(s.id);
    }
    bus.publish(DomainEvent::PhaseAdvanced { project_id, phase_id: phase.id.clone() });
    Ok(kids)
}

/// Persist remaining phases as JSON in a side-table for the phase reactor.
/// Phase queue lives in `sessions.handoff_path`-shaped column? No —
/// reuse a JSON column on the orchestrator: serialize `Vec<PhasePlan>` into
/// `sessions.pending_phases_json`.
fn store_pending_phases(
    db: &Db,
    orch_id: Uuid,
    remaining: &[PhasePlan],
) -> Result<(), crate::error::Error> {
    let serializable: Vec<_> = remaining.iter().map(|p| serde_json::json!({
        "id": p.id,
        "autonomy": match p.autonomy { Autonomy::Full => "full", Autonomy::Review => "review", Autonomy::Manual => "manual" },
        "depends_on": p.depends_on,
        "part_titles": p.part_titles,
    })).collect();
    let json = serde_json::to_string(&serializable).unwrap();
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE sessions SET pending_phases_json = ?1 WHERE id = ?2",
        rusqlite::params![json, orch_id.to_string()],
    )?;
    Ok(())
}
```

Add to `migrations/0004_overseer_executor.sql`:

```sql
ALTER TABLE sessions ADD COLUMN pending_phases_json TEXT NULL;
```

(Update Task 1's migration file before running — or land as a follow-up commit before running this task. If Task 1 already committed without this column, add a separate `migrations/0005_pending_phases.sql` instead — keep migrations append-only.)

> **Decision rule:** if Task 1 has already been committed and pushed, create `migrations/0005_pending_phases.sql` with just the `ALTER TABLE`. Otherwise edit Task 1's file. Always check `git log -- migrations/` before deciding.

- [ ] **Step 7: Implement REST endpoint**

Create `src/api/execute_plan.rs`:

```rust
use axum::{extract::{Path, State}, http::StatusCode, response::{IntoResponse, Response}, Json};
use std::sync::Arc;
use uuid::Uuid;

use crate::plan_executor::{execute_plan, ExecuteError};
use crate::session_manager::SessionManager;

pub async fn handler(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
) -> Response {
    if !crate::overseer::config::executor_enabled() {
        return (StatusCode::FORBIDDEN, "GALVOLITE_PLAN_EXECUTOR=0").into_response();
    }
    match execute_plan(mgr, project_id).await {
        Ok(ids) => Json(serde_json::json!({"spawned": ids})).into_response(),
        Err(ExecuteError::MissingArtifact(name)) =>
            (StatusCode::BAD_REQUEST, format!("missing artifact: {name}")).into_response(),
        Err(ExecuteError::Derive(e)) =>
            (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(ExecuteError::NoOrchestrator(_)) =>
            (StatusCode::CONFLICT, "no orchestrator session — spawn one first").into_response(),
        Err(ExecuteError::AlreadyRunning) =>
            (StatusCode::CONFLICT, "orchestrator already has running children").into_response(),
        Err(ExecuteError::Spawn(e)) => crate::api::err_to_response(e),
    }
}
```

In `src/api/mod.rs`, register:

```rust
pub mod execute_plan;
// in router():
.route("/api/projects/:id/execute-plan", post(execute_plan::handler))
```

- [ ] **Step 8: Run tests**

Run: `GALVOLITE_PLAN_EXECUTOR=1 cargo test --test execute_plan_e2e`
Expected: PASS.

- [ ] **Step 9: Add validation tests**

Create `tests/execute_plan_validation.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;

#[tokio::test]
async fn missing_plan_returns_400() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("plan8nv").await;
    let resp = app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn double_execute_returns_409() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = spawn_test_app().await;
    let project = app.create_project_with_plan_and_directive(
        "- [ ] alpha\n",
        "# Directive: x\nPlan: .galvolite/plan.md\n## Phases\n| ID | Parts | Autonomy | Depends on |\n| --- | --- | --- | --- |\n| p1 | 1 | manual | — |\n## Notes\n",
    ).await;
    app.spawn_orchestrator(project.id).await;
    let r1 = app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;
    assert_eq!(r1.status(), 200);
    let r2 = app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;
    assert_eq!(r2.status(), 409);
}

#[tokio::test]
async fn no_orchestrator_returns_409() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = spawn_test_app().await;
    let project = app.create_project_with_plan_and_directive(
        "- [ ] alpha\n",
        "# Directive: x\nPlan: .galvolite/plan.md\n## Phases\n| ID | Parts | Autonomy | Depends on |\n| --- | --- | --- | --- |\n| p1 | 1 | full | — |\n## Notes\n",
    ).await;
    let resp = app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;
    assert_eq!(resp.status(), 409);
}
```

Run: `GALVOLITE_PLAN_EXECUTOR=1 cargo test --test execute_plan_validation`
Expected: PASS (3 tests).

- [ ] **Step 10: Commit**

```bash
git add src/plan_executor.rs src/api/execute_plan.rs src/api/mod.rs src/api/events.rs \
        src/session_manager.rs migrations/ tests/execute_plan_e2e.rs \
        tests/execute_plan_validation.rs tests/common/test_app.rs
git commit -m "feat(plan-executor): execute_plan driver + REST endpoint + validation"
```

---

## Task 5: Phase reactor — advance on rejoin (autonomy=full) + approval endpoint

**Files:**
- Modify: `src/plan_executor.rs` (add `advance_next_phase` + `start_phase_reactor`)
- Modify: `src/api/execute_plan.rs` (add `approve_phase` handler)
- Modify: `src/api/mod.rs` (register approve route)
- Modify: `src/main.rs` (start phase reactor)
- Test: `tests/execute_plan_phase_reactor.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/execute_plan_phase_reactor.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;

#[tokio::test]
async fn full_autonomy_advances_on_rejoin() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = spawn_test_app().await;
    let project = app.create_project_with_plan_and_directive(
        "- [ ] one\n- [ ] two\n",
        "# Directive: x\nPlan: .galvolite/plan.md\n## Phases\n| ID | Parts | Autonomy | Depends on |\n| --- | --- | --- | --- |\n| p1 | 1 | full | — |\n| p2 | 2 | full | p1 |\n## Notes\n",
    ).await;
    let orch = app.spawn_orchestrator(project.id).await;
    app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;

    // Simulate child completion → triggers rejoin → reactor advances p2.
    let kids_p1 = app.list_children_of_orch(project.id).await;
    for k in &kids_p1 { app.complete_child(k.id).await; }
    app.await_pending_rejoin(orch.id).await;

    let kids_all = app.list_children_of_orch(project.id).await;
    let titles: Vec<_> = kids_all.iter().map(|s| s.task_summary.as_str()).collect();
    assert!(titles.contains(&"two"), "p2 should have been spawned automatically: {titles:?}");
}

#[tokio::test]
async fn review_autonomy_blocks_until_approve() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = spawn_test_app().await;
    let project = app.create_project_with_plan_and_directive(
        "- [ ] one\n- [ ] two\n",
        "# Directive: x\nPlan: .galvolite/plan.md\n## Phases\n| ID | Parts | Autonomy | Depends on |\n| --- | --- | --- | --- |\n| p1 | 1 | full | — |\n| p2 | 2 | review | p1 |\n## Notes\n",
    ).await;
    let orch = app.spawn_orchestrator(project.id).await;
    app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;
    for k in &app.list_children_of_orch(project.id).await { app.complete_child(k.id).await; }
    app.await_pending_rejoin(orch.id).await;

    // p2 must NOT have spawned — autonomy=review blocks until approve.
    let kids = app.list_children_of_orch(project.id).await;
    assert_eq!(kids.len(), 1, "p2 should not auto-spawn under review");

    // Approve.
    let resp = app.post(&format!("/api/projects/{}/phases/p2/approve", project.id), serde_json::json!({})).await;
    assert_eq!(resp.status(), 200);

    // Now p2 spawned.
    let kids = app.list_children_of_orch(project.id).await;
    assert_eq!(kids.len(), 2);
}
```

(Helpers `complete_child` and `await_pending_rejoin` may already exist in the existing rejoin tests — reuse from `tests/pending_rejoin_endpoint.rs` or add wrappers in `tests/common/test_app.rs`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test execute_plan_phase_reactor`
Expected: FAIL — reactor not implemented; approve endpoint missing.

- [ ] **Step 3: Implement reactor + approve**

Append to `src/plan_executor.rs`:

```rust
/// Advance the next phase from `pending_phases_json`. Pops the head, spawns it,
/// re-saves the tail. Emits `PhaseReviewNeeded` if non-`full` autonomy.
pub async fn advance_next_phase(
    mgr: &Arc<SessionManager>,
    orch_id: Uuid,
    project_id: Uuid,
) -> Result<bool, crate::error::Error> {
    let bus = mgr.event_bus().clone();
    let conn = mgr.db().pool().get()?;
    let raw: Option<String> = conn.query_row(
        "SELECT pending_phases_json FROM sessions WHERE id = ?1",
        [orch_id.to_string()], |r| r.get(0),
    ).ok().flatten();
    drop(conn);
    let Some(json) = raw else { return Ok(false); };
    let mut queue: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
    if queue.is_empty() { return Ok(false); }
    let head = queue.remove(0);
    let phase = PhasePlan {
        id: head["id"].as_str().unwrap_or("").to_string(),
        autonomy: match head["autonomy"].as_str().unwrap_or("manual") {
            "full" => Autonomy::Full,
            "review" => Autonomy::Review,
            _ => Autonomy::Manual,
        },
        depends_on: vec![],
        part_titles: serde_json::from_value(head["part_titles"].clone()).unwrap_or_default(),
    };
    let orch = mgr.db().get_session(orch_id)?;
    let kids = spawn_phase(mgr, &orch, &phase, &bus, project_id).await?;
    mgr.set_waiting_on(orch_id, &kids).await?;

    let conn = mgr.db().pool().get()?;
    conn.execute(
        "UPDATE sessions SET pending_phases_json = ?1 WHERE id = ?2",
        rusqlite::params![serde_json::to_string(&queue).unwrap(), orch_id.to_string()],
    )?;

    if matches!(phase.autonomy, Autonomy::Review | Autonomy::Manual) {
        bus.publish(DomainEvent::PhaseReviewNeeded { project_id, phase_id: phase.id });
    }
    Ok(true)
}

/// Background reactor: when an orchestrator's pending_rejoin is set (children
/// resolved) AND the next pending phase has autonomy=full, auto-advance.
pub fn start_phase_reactor(mgr: Arc<SessionManager>) {
    let mut rx = mgr.event_bus().subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = rx.recv().await {
            if let DomainEvent::SessionChildrenResolved { session_id, .. } = ev {
                let Ok(orch) = mgr.db().get_session(session_id) else { continue };
                if orch.role != "orchestrator" { continue; }
                // Peek queue: if next is `full`, advance; else leave to operator.
                let conn = match mgr.db().pool().get() { Ok(c) => c, Err(_) => continue };
                let raw: Option<String> = conn.query_row(
                    "SELECT pending_phases_json FROM sessions WHERE id = ?1",
                    [session_id.to_string()], |r| r.get(0),
                ).ok().flatten();
                drop(conn);
                let Some(json) = raw else { continue };
                let queue: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
                let Some(head) = queue.first() else { continue };
                if head["autonomy"].as_str() == Some("full") {
                    let _ = advance_next_phase(&mgr, session_id, orch.project_id).await;
                }
            }
        }
    });
}
```

In `src/api/execute_plan.rs`, add:

```rust
pub async fn approve_phase(
    State(mgr): State<Arc<SessionManager>>,
    Path((project_id, phase_id)): Path<(Uuid, String)>,
) -> Response {
    if !crate::overseer::config::executor_enabled() {
        return (StatusCode::FORBIDDEN, "GALVOLITE_PLAN_EXECUTOR=0").into_response();
    }
    let orch = match mgr.find_orchestrator_for_project(project_id) {
        Some(o) => o,
        None => return (StatusCode::CONFLICT, "no orchestrator").into_response(),
    };
    // Verify head of pending_phases matches `phase_id`.
    let conn = match mgr.db().pool().get() {
        Ok(c) => c,
        Err(e) => return crate::api::err_to_response(crate::error::Error::Db(e.into())),
    };
    let raw: Option<String> = conn.query_row(
        "SELECT pending_phases_json FROM sessions WHERE id = ?1",
        [orch.id.to_string()], |r| r.get(0),
    ).ok().flatten();
    drop(conn);
    let queue: Vec<serde_json::Value> = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if queue.first().and_then(|p| p["id"].as_str()) != Some(phase_id.as_str()) {
        return (StatusCode::CONFLICT, "phase is not next in queue").into_response();
    }
    match crate::plan_executor::advance_next_phase(&mgr, orch.id, project_id).await {
        Ok(_) => Json(serde_json::json!({"approved": phase_id})).into_response(),
        Err(e) => crate::api::err_to_response(e),
    }
}
```

In `src/api/mod.rs`:

```rust
.route("/api/projects/:id/phases/:phase_id/approve", post(execute_plan::approve_phase))
```

In `src/main.rs`, after `mgr.start_terminal_reactor()`:

```rust
galvolite::plan_executor::start_phase_reactor(Arc::clone(&mgr));
```

- [ ] **Step 4: Run tests**

Run: `GALVOLITE_PLAN_EXECUTOR=1 cargo test --test execute_plan_phase_reactor`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/plan_executor.rs src/api/execute_plan.rs src/api/mod.rs src/main.rs \
        tests/execute_plan_phase_reactor.rs
git commit -m "feat(plan-executor): phase reactor + approve endpoint"
```

---

## Task 6: ExecutePlanButton React component

**Files:**
- Modify: `ui/src/types.ts` (add Phase + add PhaseReviewNeeded variant)
- Modify: `ui/src/api.ts` (add `executePlan`, `approvePhase`)
- Create: `ui/src/components/ExecutePlanButton.tsx`
- Create: `ui/src/components/ExecutePlanButton.test.tsx`

- [ ] **Step 1: Add types + api client**

In `ui/src/types.ts`, append:

```ts
export interface Phase {
  id: string;
  parts: number[];
  autonomy: "full" | "review" | "manual";
  depends_on: string[];
}
```

Add to `DomainEvent` union (find existing union in same file):

```ts
| { type: "phase.review_needed"; project_id: string; phase_id: string }
| { type: "phase.advanced"; project_id: string; phase_id: string }
```

In `ui/src/api.ts`, append inside `api = { ... }`:

```ts
executePlan: (projectId: string) =>
  request<{ spawned: string[] }>(
    `/api/projects/${projectId}/execute-plan`,
    { method: "POST", body: "{}" },
  ),
approvePhase: (projectId: string, phaseId: string) =>
  request<{ approved: string }>(
    `/api/projects/${projectId}/phases/${encodeURIComponent(phaseId)}/approve`,
    { method: "POST", body: "{}" },
  ),
```

- [ ] **Step 2: Write the failing test**

Create `ui/src/components/ExecutePlanButton.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { ExecutePlanButton } from "./ExecutePlanButton";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    getPlan: vi.fn(),
    getDirective: vi.fn(),
    executePlan: vi.fn(),
  },
}));

describe("ExecutePlanButton", () => {
  beforeEach(() => vi.clearAllMocks());

  it("disabled when plan or directive missing", async () => {
    (api.getPlan as any).mockResolvedValue(null);
    (api.getDirective as any).mockResolvedValue(null);
    render(<ExecutePlanButton projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /execute plan/i });
    expect(btn).toBeDisabled();
  });

  it("enabled when both present and fires API on click", async () => {
    (api.getPlan as any).mockResolvedValue({ markdown: "x" });
    (api.getDirective as any).mockResolvedValue({ name: "x" });
    (api.executePlan as any).mockResolvedValue({ spawned: ["a", "b"] });
    render(<ExecutePlanButton projectId="p1" />);
    const btn = await screen.findByRole("button", { name: /execute plan/i });
    await waitFor(() => expect(btn).toBeEnabled());
    fireEvent.click(btn);
    await waitFor(() => expect(api.executePlan).toHaveBeenCalledWith("p1"));
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd ui && npm test -- ExecutePlanButton`
Expected: FAIL — component does not exist.

- [ ] **Step 4: Create component**

Create `ui/src/components/ExecutePlanButton.tsx`:

```tsx
import { useEffect, useState } from "react";
import { PlayCircle } from "lucide-react";
import { api } from "../api";

interface Props { projectId: string; onExecute?: () => void; }

type GateState =
  | { kind: "loading" }
  | { kind: "missing"; reasons: string[] }
  | { kind: "ready" };

export function ExecutePlanButton({ projectId, onExecute }: Props) {
  const [state, setState] = useState<GateState>({ kind: "loading" });
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    Promise.all([api.getPlan(projectId), api.getDirective(projectId)]).then(
      ([plan, dir]) => {
        if (cancelled) return;
        const reasons: string[] = [];
        if (plan === null) reasons.push("plan.md required");
        if (dir === null) reasons.push("directive.md required");
        setState(reasons.length ? { kind: "missing", reasons } : { kind: "ready" });
      },
      () => !cancelled && setState({ kind: "missing", reasons: ["plan.md required", "directive.md required"] }),
    );
    return () => { cancelled = true; };
  }, [projectId]);

  const disabled = state.kind !== "ready" || busy;
  const cls = disabled
    ? "px-3 py-1 rounded bg-neutral-700 text-neutral-400 text-xs cursor-not-allowed flex items-center gap-1"
    : "px-3 py-1 rounded bg-emerald-700 hover:bg-emerald-600 text-white text-xs flex items-center gap-1";

  return (
    <div className="flex items-center gap-2">
      <button
        type="button"
        disabled={disabled}
        className={cls}
        onClick={async () => {
          setBusy(true); setErr(null);
          try { await api.executePlan(projectId); onExecute?.(); }
          catch (e: any) { setErr(e?.message ?? "execute failed"); }
          finally { setBusy(false); }
        }}
      >
        <PlayCircle size={14} /> Execute plan
      </button>
      {state.kind === "missing" && (
        <span className="text-xs text-neutral-500">{state.reasons.join(" + ")}</span>
      )}
      {err && <span className="text-xs text-red-400">{err}</span>}
    </div>
  );
}
```

- [ ] **Step 5: Run tests**

Run: `cd ui && npm test -- ExecutePlanButton`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add ui/src/types.ts ui/src/api.ts ui/src/components/ExecutePlanButton.tsx \
        ui/src/components/ExecutePlanButton.test.tsx
git commit -m "feat(ui): ExecutePlanButton with plan+directive gate"
```

---

## Task 7: Recommendations DB module + dedupe

**Files:**
- Create: `src/recommendations.rs`
- Modify: `src/lib.rs` (register module)
- Test: `tests/recommendations_db.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/recommendations_db.rs`:

```rust
use galvolite::db::Db;
use galvolite::recommendations::{self, NewRec, RecStatus};
use uuid::Uuid;

fn open_db() -> Db {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    Box::leak(Box::new(tmp));
    db
}

fn sample(project_id: Uuid) -> NewRec {
    NewRec {
        project_id,
        kind: "stuck_session".into(),
        severity: "warn".into(),
        title: "Session A appears stuck".into(),
        body: "no progress for 10m".into(),
        rationale: "no events in 600s".into(),
        evidence_json: serde_json::json!({"sids":["x"]}),
    }
}

#[test]
fn insert_returns_id_and_dedupes_on_content_hash() {
    let db = open_db();
    let pid = Uuid::new_v4();
    let id1 = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    let dup = recommendations::insert(&db, sample(pid)).unwrap();
    assert!(dup.is_none(), "second insert with identical content should dedupe");
    let list = recommendations::list_by_status(&db, pid, RecStatus::Pending).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id1);
}

#[test]
fn act_marks_status_acted_and_sets_acted_at() {
    let db = open_db();
    let pid = Uuid::new_v4();
    let id = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    recommendations::set_status(&db, id, RecStatus::Acted).unwrap();
    let r = recommendations::get(&db, id).unwrap();
    assert_eq!(r.status, "acted");
    assert!(r.acted_at.is_some());
}

#[test]
fn defer_with_until_resurfaces_after_expiry_sweep() {
    let db = open_db();
    let pid = Uuid::new_v4();
    let id = recommendations::insert(&db, sample(pid)).unwrap().unwrap();
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    recommendations::defer(&db, id, past).unwrap();
    let n = recommendations::resurface_expired(&db, chrono::Utc::now()).unwrap();
    assert_eq!(n, 1);
    let r = recommendations::get(&db, id).unwrap();
    assert_eq!(r.status, "pending");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test recommendations_db`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement recommendations module**

Create `src/recommendations.rs`:

```rust
//! Plan 8 — recommendations DB CRUD with content_hash dedupe.

use crate::db::Db;
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecStatus { Pending, Acted, Dismissed, Deferred }

impl RecStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending", Self::Acted => "acted",
            Self::Dismissed => "dismissed", Self::Deferred => "deferred",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewRec {
    pub project_id: Uuid,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub rationale: String,
    pub evidence_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub rationale: String,
    pub evidence_json: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub acted_at: Option<DateTime<Utc>>,
    pub deferred_until: Option<DateTime<Utc>>,
}

pub fn content_hash(kind: &str, title: &str, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes()); h.update(b"|");
    h.update(title.as_bytes()); h.update(b"|");
    h.update(body.as_bytes());
    hex::encode(h.finalize())
}

/// Insert a recommendation. Returns `Ok(Some(id))` on insert, `Ok(None)` if
/// `content_hash` already present (dedupe).
pub fn insert(db: &Db, n: NewRec) -> Result<Option<Uuid>> {
    let hash = content_hash(&n.kind, &n.title, &n.body);
    let id = Uuid::new_v4();
    let now = Utc::now();
    let conn = db.pool().get()?;
    let rows = conn.execute(
        "INSERT OR IGNORE INTO recommendations
         (id, project_id, kind, severity, title, body, rationale, evidence_json,
          status, created_at, content_hash)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,?10)",
        params![
            id.to_string(), n.project_id.to_string(), n.kind, n.severity,
            n.title, n.body, n.rationale,
            serde_json::to_string(&n.evidence_json).map_err(|e| Error::Config(e.to_string()))?,
            now.to_rfc3339(), hash,
        ],
    )?;
    Ok(if rows == 0 { None } else { Some(id) })
}

pub fn list_by_status(db: &Db, project_id: Uuid, status: RecStatus) -> Result<Vec<Recommendation>> {
    let conn = db.pool().get()?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, kind, severity, title, body, rationale,
                evidence_json, status, created_at, acted_at, deferred_until
         FROM recommendations WHERE project_id=?1 AND status=?2
         ORDER BY created_at DESC")?;
    let rows = stmt.query_map(params![project_id.to_string(), status.as_str()], row_to_rec)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(db: &Db, id: Uuid) -> Result<Recommendation> {
    let conn = db.pool().get()?;
    conn.query_row(
        "SELECT id, project_id, kind, severity, title, body, rationale,
                evidence_json, status, created_at, acted_at, deferred_until
         FROM recommendations WHERE id=?1",
        params![id.to_string()], row_to_rec,
    ).map_err(Error::Db)
}

pub fn set_status(db: &Db, id: Uuid, status: RecStatus) -> Result<()> {
    let conn = db.pool().get()?;
    let acted_at = if matches!(status, RecStatus::Acted) {
        Some(Utc::now().to_rfc3339())
    } else { None };
    conn.execute(
        "UPDATE recommendations SET status=?1, acted_at=COALESCE(?2, acted_at) WHERE id=?3",
        params![status.as_str(), acted_at, id.to_string()],
    )?;
    Ok(())
}

pub fn defer(db: &Db, id: Uuid, until: DateTime<Utc>) -> Result<()> {
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE recommendations SET status='deferred', deferred_until=?1 WHERE id=?2",
        params![until.to_rfc3339(), id.to_string()],
    )?;
    Ok(())
}

pub fn resurface_expired(db: &Db, now: DateTime<Utc>) -> Result<usize> {
    let conn = db.pool().get()?;
    let n = conn.execute(
        "UPDATE recommendations SET status='pending', deferred_until=NULL
         WHERE status='deferred' AND deferred_until IS NOT NULL AND deferred_until <= ?1",
        params![now.to_rfc3339()],
    )?;
    Ok(n)
}

/// Update rationale (used by Better-explanation Kimi response).
pub fn update_rationale(db: &Db, id: Uuid, rationale: &str) -> Result<()> {
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE recommendations SET rationale=?1 WHERE id=?2",
        params![rationale, id.to_string()],
    )?;
    Ok(())
}

fn row_to_rec(r: &rusqlite::Row) -> rusqlite::Result<Recommendation> {
    Ok(Recommendation {
        id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
        project_id: Uuid::parse_str(&r.get::<_, String>(1)?).unwrap(),
        kind: r.get(2)?,
        severity: r.get(3)?,
        title: r.get(4)?,
        body: r.get(5)?,
        rationale: r.get(6)?,
        evidence_json: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::Value::Null),
        status: r.get(8)?,
        created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(9)?)
            .unwrap().with_timezone(&Utc),
        acted_at: r.get::<_, Option<String>>(10)?.and_then(|s|
            DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
        deferred_until: r.get::<_, Option<String>>(11)?.and_then(|s|
            DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
    })
}
```

Register in `src/lib.rs`:

```rust
pub mod recommendations;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test recommendations_db`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add src/recommendations.rs src/lib.rs tests/recommendations_db.rs
git commit -m "feat(recommendations): DB CRUD with content_hash dedupe"
```

---

## Task 8: Kimi K2 HTTP client (with mockito)

**Files:**
- Create: `src/overseer/kimi.rs`
- Modify: `src/overseer/mod.rs` (re-export)
- Test: `tests/overseer_kimi_mocked.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/overseer_kimi_mocked.rs`:

```rust
use galvolite::overseer::kimi::{KimiClient, KimiError};

#[tokio::test]
async fn happy_path_returns_text() {
    let mut server = mockito::Server::new_async().await;
    let m = server.mock("POST", "/chat/completions")
        .match_header("authorization", "Bearer test-key")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"choices":[{"message":{"content":"hello"}}],
                       "usage":{"total_tokens":42}}"#)
        .create_async().await;

    let c = KimiClient::new("test-key", &server.url());
    let (text, tokens) = c.chat("system", "user").await.unwrap();
    assert_eq!(text, "hello");
    assert_eq!(tokens, 42);
    m.assert_async().await;
}

#[tokio::test]
async fn http_5xx_returns_transient_error() {
    let mut server = mockito::Server::new_async().await;
    server.mock("POST", "/chat/completions").with_status(503).create_async().await;
    let c = KimiClient::new("k", &server.url());
    let err = c.chat("s", "u").await.unwrap_err();
    assert!(matches!(err, KimiError::Transient(_)));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test overseer_kimi_mocked`
Expected: FAIL — `kimi` module missing.

- [ ] **Step 3: Implement Kimi client**

Create `src/overseer/kimi.rs`:

```rust
//! Kimi K2 HTTP client (OpenAI-compatible).
//! Endpoint: <base_url>/chat/completions

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum KimiError {
    #[error("transient: {0}")] Transient(String),
    #[error("permanent: {0}")] Permanent(String),
    #[error("malformed json: {0}")] Malformed(String),
}

pub struct KimiClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

#[derive(Serialize)]
struct ChatReq<'a> {
    model: &'a str,
    messages: Vec<ChatMsg<'a>>,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMsg<'a> { role: &'a str, content: &'a str }

#[derive(Deserialize)]
struct ChatResp {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice { message: ChoiceMessage }
#[derive(Deserialize)]
struct ChoiceMessage { content: String }
#[derive(Deserialize)]
struct Usage { total_tokens: u64 }

impl KimiClient {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build().unwrap(),
        }
    }

    pub async fn chat(&self, system: &str, user: &str) -> Result<(String, u64), KimiError> {
        let body = ChatReq {
            model: "kimi-k2-0711-preview",
            messages: vec![
                ChatMsg { role: "system", content: system },
                ChatMsg { role: "user",   content: user   },
            ],
            temperature: 0.2,
        };
        let url = format!("{}/chat/completions", self.base_url);
        let resp = self.http.post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await
            .map_err(|e| KimiError::Transient(e.to_string()))?;
        let status = resp.status();
        if status.is_server_error() {
            return Err(KimiError::Transient(format!("{status}")));
        }
        if !status.is_success() {
            return Err(KimiError::Permanent(format!("{status}")));
        }
        let parsed: ChatResp = resp.json().await
            .map_err(|e| KimiError::Malformed(e.to_string()))?;
        let text = parsed.choices.into_iter().next()
            .ok_or_else(|| KimiError::Malformed("no choices".into()))?
            .message.content;
        let tokens = parsed.usage.map(|u| u.total_tokens).unwrap_or(0);
        Ok((text, tokens))
    }
}
```

In `src/overseer/mod.rs`, add:

```rust
pub mod kimi;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test overseer_kimi_mocked`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/overseer/mod.rs src/overseer/kimi.rs tests/overseer_kimi_mocked.rs
git commit -m "feat(overseer): Kimi K2 HTTP client (OpenAI-compat) with mocked tests"
```

---

## Task 9: ContextSnapshot + observer (rolling rings, bus subscription)

**Files:**
- Create: `src/overseer/observer.rs`
- Modify: `src/overseer/mod.rs`
- Test: `tests/overseer_observer.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/overseer_observer.rs`:

```rust
use galvolite::api::events::{DomainEvent, EventBroadcaster};
use galvolite::overseer::observer::{ContextSnapshot, Observer};
use galvolite::state::SessionState;
use uuid::Uuid;

#[tokio::test]
async fn captures_session_event_appended_into_ring() {
    let bus = EventBroadcaster::new(64);
    let pid = Uuid::new_v4();
    let mut obs = Observer::new(pid, bus.subscribe());
    let sid = Uuid::new_v4();
    bus.publish(DomainEvent::SessionEventAppended {
        session_id: sid, event_id: 1, kind: "mcp.status_report".into(),
        source: "mcp".into(), ts: "2026-05-02T00:00:00Z".into(),
        payload: serde_json::json!({"kind":"stuck"}),
    });
    obs.drain_pending(50).await;
    let snap: ContextSnapshot = obs.snapshot();
    assert_eq!(snap.events.len(), 1);
    assert_eq!(snap.events[0].kind, "mcp.status_report");
}

#[tokio::test]
async fn ring_caps_at_capacity() {
    let bus = EventBroadcaster::new(1024);
    let pid = Uuid::new_v4();
    let mut obs = Observer::new(pid, bus.subscribe());
    for i in 0..250 {
        bus.publish(DomainEvent::SessionEventAppended {
            session_id: Uuid::new_v4(), event_id: i, kind: "k".into(),
            source: "system".into(), ts: "t".into(), payload: serde_json::json!({}),
        });
    }
    obs.drain_pending(500).await;
    let snap = obs.snapshot();
    assert!(snap.events.len() <= 200, "events ring should cap at 200");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test overseer_observer`
Expected: FAIL — observer module missing.

- [ ] **Step 3: Implement Observer**

Create `src/overseer/observer.rs`:

```rust
//! Bus subscriber that maintains a rolling `ContextSnapshot` for one project.
//! Read-only on the bus.

use crate::api::events::DomainEvent;
use std::collections::VecDeque;
use tokio::sync::broadcast;
use uuid::Uuid;

pub const EVENT_RING_CAP: usize = 200;
pub const PTY_TAIL_LINE_CAP: usize = 50;

#[derive(Debug, Clone)]
pub struct EventRecord {
    pub session_id: Uuid,
    pub kind: String,
    pub source: String,
    pub ts: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct ContextSnapshot {
    pub project_id: Option<Uuid>,
    pub events: Vec<EventRecord>,
    pub plan_md: Option<String>,
    pub directive_md: Option<String>,
    pub recent_postmortems: Vec<crate::postmortem::Postmortem>,
    pub recent_decisions: Vec<crate::decisions::Decision>,
}

pub struct Observer {
    project_id: Uuid,
    rx: broadcast::Receiver<DomainEvent>,
    events: VecDeque<EventRecord>,
}

impl Observer {
    pub fn new(project_id: Uuid, rx: broadcast::Receiver<DomainEvent>) -> Self {
        Self { project_id, rx, events: VecDeque::with_capacity(EVENT_RING_CAP) }
    }

    pub fn project_id(&self) -> Uuid { self.project_id }

    /// Drain up to `max` queued events from the broadcast receiver into the ring.
    /// Non-blocking; returns count drained. Lagged frames are silently skipped.
    pub async fn drain_pending(&mut self, max: usize) -> usize {
        let mut n = 0;
        for _ in 0..max {
            match self.rx.try_recv() {
                Ok(ev) => {
                    if let Some(rec) = filter_event(ev, self.project_id) {
                        if self.events.len() == EVENT_RING_CAP { self.events.pop_front(); }
                        self.events.push_back(rec);
                        n += 1;
                    }
                }
                Err(_) => break,
            }
        }
        n
    }

    pub fn snapshot(&self) -> ContextSnapshot {
        ContextSnapshot {
            project_id: Some(self.project_id),
            events: self.events.iter().cloned().collect(),
            plan_md: None,
            directive_md: None,
            recent_postmortems: vec![],
            recent_decisions: vec![],
        }
    }

    /// Hydrate snapshot with on-disk artifacts. Best-effort: missing files
    /// leave the corresponding snapshot field empty.
    pub fn hydrate_artifacts(&self, snap: &mut ContextSnapshot, project_path: &std::path::Path) {
        snap.plan_md = std::fs::read_to_string(project_path.join(".galvolite/plan.md")).ok();
        snap.directive_md = std::fs::read_to_string(project_path.join(".galvolite/directive.md")).ok();
        if let Ok(mut pms) = crate::postmortem::list_all(project_path) {
            pms.truncate(5);
            snap.recent_postmortems = pms;
        }
        if let Ok(decs) = crate::decisions::list_recent(project_path, 10) {
            snap.recent_decisions = decs;
        }
    }
}

fn filter_event(ev: DomainEvent, _project_id: Uuid) -> Option<EventRecord> {
    // The bus is global; we cannot per-project filter here without a project_id
    // on every event. For now, accept all event variants the Overseer cares about.
    match ev {
        DomainEvent::SessionEventAppended { session_id, kind, source, ts, payload, .. } =>
            Some(EventRecord { session_id, kind, source, ts, payload }),
        _ => None,
    }
}
```

In `src/overseer/mod.rs`:

```rust
pub mod observer;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test overseer_observer`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/overseer/mod.rs src/overseer/observer.rs tests/overseer_observer.rs
git commit -m "feat(overseer): Observer with event ring + artifact hydration"
```

---

## Task 10: Recommender — pack_prompt + parse_kimi_response

**Files:**
- Create: `src/overseer/recommender.rs`
- Modify: `src/overseer/mod.rs`
- Test: `tests/overseer_recommender.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/overseer_recommender.rs`:

```rust
use galvolite::overseer::observer::ContextSnapshot;
use galvolite::overseer::recommender::{pack_prompt, parse_kimi_response};

#[test]
fn pack_prompt_includes_recent_event_kinds() {
    let mut snap = ContextSnapshot::default();
    snap.events.push(galvolite::overseer::observer::EventRecord {
        session_id: uuid::Uuid::nil(), kind: "mcp.status_report".into(),
        source: "mcp".into(), ts: "t".into(), payload: serde_json::json!({"kind":"stuck"}),
    });
    let (sys, user) = pack_prompt(&snap);
    assert!(sys.contains("Overseer"));
    assert!(user.contains("mcp.status_report"));
}

#[test]
fn pack_prompt_caps_below_8k_chars() {
    let mut snap = ContextSnapshot::default();
    snap.plan_md = Some("x".repeat(20_000));
    let (_, user) = pack_prompt(&snap);
    assert!(user.len() < 8_192, "len={}", user.len());
}

#[test]
fn parse_kimi_well_formed_returns_recs() {
    let json = r#"{
      "recommendations": [
        {"kind":"stuck","severity":"warn","title":"t","body":"b","rationale":"r","evidence_refs":[]}
      ]
    }"#;
    let recs = parse_kimi_response(json).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].title, "t");
}

#[test]
fn parse_kimi_malformed_returns_empty() {
    assert!(parse_kimi_response("not json").unwrap_or_default().is_empty());
}

#[test]
fn parse_kimi_extracts_from_fenced_block() {
    let s = "Sure, here:\n```json\n{\"recommendations\":[]}\n```";
    let recs = parse_kimi_response(s).unwrap();
    assert!(recs.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test overseer_recommender`
Expected: FAIL — recommender missing.

- [ ] **Step 3: Implement recommender**

Create `src/overseer/recommender.rs`:

```rust
//! Prompt packing + response parsing for the Overseer Kimi calls.

use serde::Deserialize;

use super::observer::ContextSnapshot;

const PROMPT_BUDGET: usize = 8_000;

const SYSTEM: &str = "\
You are the GalvoLite Overseer. You watch a Claude-driven coding project from the side, \
using the bus events, plan, directive, postmortems, and decisions provided. You do NOT \
write code. Your only output is a JSON object with key `recommendations`, each item \
having {kind, severity (info|warn|urgent), title, body, rationale, evidence_refs}. Be \
concise: one or two sentences per body. Skip noise — emit at most 5 recommendations.";

pub fn pack_prompt(snap: &ContextSnapshot) -> (String, String) {
    let mut user = String::new();
    user.push_str("# Recent events\n\n");
    for e in snap.events.iter().rev().take(40) {
        user.push_str(&format!("- {} ({}, {}) {}\n",
            e.kind, e.source, &e.ts, summarize_payload(&e.payload)));
    }
    if let Some(p) = &snap.plan_md {
        user.push_str("\n# plan.md\n\n");
        push_capped(&mut user, p, 1500);
    }
    if let Some(d) = &snap.directive_md {
        user.push_str("\n# directive.md\n\n");
        push_capped(&mut user, d, 1500);
    }
    if !snap.recent_postmortems.is_empty() {
        user.push_str("\n# Recent postmortems\n\n");
        for pm in snap.recent_postmortems.iter().take(3) {
            user.push_str(&format!("- part {} ({}): gotchas={}\n",
                pm.plan_part, pm.outcome, first_line(&pm.gotchas)));
        }
    }
    if !snap.recent_decisions.is_empty() {
        user.push_str("\n# Recent decisions\n\n");
        for d in snap.recent_decisions.iter().take(5) {
            user.push_str(&format!("- Q: {}\n  A: {}\n", first_line(&d.question), first_line(&d.answer)));
        }
    }
    if user.len() > PROMPT_BUDGET { user.truncate(PROMPT_BUDGET); }
    (SYSTEM.to_string(), user)
}

fn push_capped(out: &mut String, s: &str, cap: usize) {
    if s.len() <= cap { out.push_str(s); } else { out.push_str(&s[..cap]); out.push_str("\n…(truncated)\n"); }
}

fn first_line(s: &str) -> &str { s.lines().next().unwrap_or("") }

fn summarize_payload(v: &serde_json::Value) -> String {
    if let Some(o) = v.as_object() {
        let kvs: Vec<String> = o.iter().take(3)
            .map(|(k, v)| format!("{k}={}", v.to_string().chars().take(40).collect::<String>()))
            .collect();
        kvs.join(" ")
    } else { String::new() }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParsedRec {
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub rationale: String,
    #[serde(default)]
    pub evidence_refs: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct Wrapped { recommendations: Vec<ParsedRec> }

/// Parse Kimi's response. Tolerates fenced code blocks and surrounding chatter.
/// Returns Ok([]) on malformed input — never propagates parse error.
pub fn parse_kimi_response(s: &str) -> Result<Vec<ParsedRec>, std::convert::Infallible> {
    let candidate = extract_json_block(s).unwrap_or(s);
    Ok(serde_json::from_str::<Wrapped>(candidate)
        .map(|w| w.recommendations)
        .unwrap_or_default())
}

fn extract_json_block(s: &str) -> Option<&str> {
    if let Some(start) = s.find("```json") {
        let after = &s[start + 7..];
        let end = after.find("```")?;
        return Some(after[..end].trim());
    }
    if let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) {
        if a < b { return Some(&s[a..=b]); }
    }
    None
}
```

In `src/overseer/mod.rs`:

```rust
pub mod recommender;
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test overseer_recommender`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add src/overseer/mod.rs src/overseer/recommender.rs tests/overseer_recommender.rs
git commit -m "feat(overseer): pack_prompt + parse_kimi_response"
```

---

## Task 11: Overseer review() driver — debounce, rate cap, budget, degraded

**Files:**
- Modify: `src/overseer/recommender.rs` (add `review` async fn + `OverseerState`)
- Modify: `src/api/events.rs` (add `RecommendationCreated`, `OverseerDegraded`, `OverseerRecovered`)
- Test: `tests/overseer_review.rs`

- [ ] **Step 1: Extend DomainEvent**

In `src/api/events.rs`, add:

```rust
#[serde(rename = "recommendation.created")]
RecommendationCreated { rec_id: Uuid, project_id: Uuid, severity: String },
#[serde(rename = "overseer.degraded")]
OverseerDegraded { project_id: Uuid, reason: String },
#[serde(rename = "overseer.recovered")]
OverseerRecovered { project_id: Uuid },
```

Extend `applies_to`:

```rust
DomainEvent::RecommendationCreated { .. }
| DomainEvent::OverseerDegraded { .. }
| DomainEvent::OverseerRecovered { .. } => false,
```

- [ ] **Step 2: Write the failing test**

Create `tests/overseer_review.rs`:

```rust
use galvolite::db::Db;
use galvolite::api::events::{DomainEvent, EventBroadcaster};
use galvolite::overseer::observer::ContextSnapshot;
use galvolite::overseer::recommender::{review, OverseerState};
use galvolite::recommendations::{self, RecStatus};
use uuid::Uuid;

#[tokio::test]
async fn happy_path_inserts_and_emits() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let bus = EventBroadcaster::new(32);
    let mut sub = bus.subscribe();
    let pid = Uuid::new_v4();

    let mut state = OverseerState::new(100_000);
    let kimi_text = r#"{"recommendations":[
        {"kind":"stuck","severity":"warn","title":"T","body":"B","rationale":"R","evidence_refs":[]}
    ]}"#;
    let snap = ContextSnapshot::default();
    review(&db, &bus, pid, &snap, &mut state, |_, _| async move {
        Ok::<_, galvolite::overseer::kimi::KimiError>((kimi_text.to_string(), 50u64))
    }).await.unwrap();

    let recs = recommendations::list_by_status(&db, pid, RecStatus::Pending).unwrap();
    assert_eq!(recs.len(), 1);
    let ev = sub.try_recv().unwrap();
    assert!(matches!(ev, DomainEvent::RecommendationCreated { .. }));
    assert_eq!(state.tokens_used, 50);
}

#[tokio::test]
async fn five_failures_in_a_row_emit_degraded() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let bus = EventBroadcaster::new(32);
    let mut sub = bus.subscribe();
    let pid = Uuid::new_v4();
    let mut state = OverseerState::new(100_000);
    let snap = ContextSnapshot::default();

    for _ in 0..5 {
        let _ = review(&db, &bus, pid, &snap, &mut state, |_, _| async move {
            Err::<(String, u64), _>(galvolite::overseer::kimi::KimiError::Transient("nope".into()))
        }).await;
    }
    let mut saw_degraded = false;
    while let Ok(ev) = sub.try_recv() {
        if matches!(ev, DomainEvent::OverseerDegraded { .. }) { saw_degraded = true; }
    }
    assert!(saw_degraded);
}

#[tokio::test]
async fn token_budget_blocks_call() {
    let tmp = tempfile::tempdir().unwrap();
    let db = Db::open(tmp.path().join("g.db")).unwrap();
    let bus = EventBroadcaster::new(32);
    let pid = Uuid::new_v4();
    let mut state = OverseerState::new(10);
    state.tokens_used = 50;
    let snap = ContextSnapshot::default();
    let res = review(&db, &bus, pid, &snap, &mut state, |_, _| async move {
        panic!("should not be called");
    }).await;
    assert!(res.is_err());
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --test overseer_review`
Expected: FAIL — `review` and `OverseerState` not defined.

- [ ] **Step 4: Implement review() driver**

Append to `src/overseer/recommender.rs`:

```rust
use crate::api::events::{DomainEvent, EventBroadcaster};
use crate::db::Db;
use crate::recommendations::{insert, NewRec};
use std::future::Future;
use uuid::Uuid;

pub struct OverseerState {
    pub daily_budget: u64,
    pub tokens_used: u64,
    pub consecutive_failures: u32,
    pub degraded: bool,
}

impl OverseerState {
    pub fn new(daily_budget: u64) -> Self {
        Self { daily_budget, tokens_used: 0, consecutive_failures: 0, degraded: false }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error("budget exhausted ({used}/{budget})")] Budget { used: u64, budget: u64 },
    #[error("kimi: {0}")] Kimi(#[from] crate::overseer::kimi::KimiError),
    #[error("db: {0}")] Db(#[from] crate::error::Error),
}

/// Run one review cycle. `kimi_call` is a closure injected for testability —
/// in production it wraps `KimiClient::chat`.
pub async fn review<F, Fut>(
    db: &Db,
    bus: &EventBroadcaster,
    project_id: Uuid,
    snap: &ContextSnapshot,
    state: &mut OverseerState,
    kimi_call: F,
) -> Result<usize, ReviewError>
where
    F: FnOnce(String, String) -> Fut,
    Fut: Future<Output = Result<(String, u64), crate::overseer::kimi::KimiError>>,
{
    if state.tokens_used >= state.daily_budget {
        return Err(ReviewError::Budget { used: state.tokens_used, budget: state.daily_budget });
    }
    let (sys, user) = pack_prompt(snap);
    match kimi_call(sys, user).await {
        Ok((text, tokens)) => {
            state.tokens_used = state.tokens_used.saturating_add(tokens);
            if state.degraded {
                bus.publish(DomainEvent::OverseerRecovered { project_id });
                state.degraded = false;
            }
            state.consecutive_failures = 0;
            let recs = parse_kimi_response(&text).unwrap_or_default();
            let mut inserted = 0;
            for r in recs {
                let nr = NewRec {
                    project_id,
                    kind: r.kind, severity: r.severity, title: r.title,
                    body: r.body, rationale: r.rationale,
                    evidence_json: serde_json::Value::Array(r.evidence_refs),
                };
                let sev = nr.severity.clone();
                if let Some(id) = insert(db, nr)? {
                    bus.publish(DomainEvent::RecommendationCreated {
                        rec_id: id, project_id, severity: sev,
                    });
                    inserted += 1;
                }
            }
            Ok(inserted)
        }
        Err(e) => {
            state.consecutive_failures += 1;
            if state.consecutive_failures >= 5 && !state.degraded {
                bus.publish(DomainEvent::OverseerDegraded {
                    project_id, reason: format!("{e}"),
                });
                state.degraded = true;
            }
            Err(ReviewError::Kimi(e))
        }
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test overseer_review`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add src/overseer/recommender.rs src/api/events.rs tests/overseer_review.rs
git commit -m "feat(overseer): review() driver with budget + degraded states"
```

---

## Task 12: OverseerSupervisor — per-project lifecycle

**Files:**
- Modify: `src/overseer/mod.rs` (add `OverseerSupervisor`)
- Modify: `src/main.rs` (start supervisor)
- Test: `tests/overseer_supervisor.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/overseer_supervisor.rs`:

```rust
use galvolite::overseer::OverseerSupervisor;
use std::sync::Arc;

#[tokio::test]
async fn start_spawns_one_task_per_existing_project() {
    let app = common::test_app::spawn_test_app().await;
    let _p1 = app.create_project_in_temp_repo("a").await;
    let _p2 = app.create_project_in_temp_repo("b").await;
    let sup = OverseerSupervisor::new(Arc::clone(&app.mgr));
    sup.spawn_for_existing_projects().await;
    assert_eq!(sup.task_count(), 2);
}

#[tokio::test]
async fn project_deleted_drops_task() {
    let app = common::test_app::spawn_test_app().await;
    let p = app.create_project_in_temp_repo("a").await;
    let sup = OverseerSupervisor::new(Arc::clone(&app.mgr));
    sup.spawn_for_existing_projects().await;
    assert_eq!(sup.task_count(), 1);
    app.mgr.delete_project(p.id).await.unwrap();
    // Allow event loop a tick.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(sup.task_count(), 0);
}

mod common {
    pub mod test_app {
        pub use crate::common::test_app::*;
    }
}
```

(If the test scaffold complains about `mod common`, follow the pattern from existing tests like `tests/spawn_e2e.rs`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test overseer_supervisor`
Expected: FAIL — supervisor missing.

- [ ] **Step 3: Implement supervisor**

Append to `src/overseer/mod.rs`:

```rust
use crate::api::events::DomainEvent;
use crate::session_manager::SessionManager;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;
use uuid::Uuid;

pub struct OverseerSupervisor {
    mgr: Arc<SessionManager>,
    tasks: Arc<Mutex<HashMap<Uuid, JoinHandle<()>>>>,
}

impl OverseerSupervisor {
    pub fn new(mgr: Arc<SessionManager>) -> Self {
        Self { mgr, tasks: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn task_count(&self) -> usize { self.tasks.lock().unwrap().len() }

    pub async fn spawn_for_existing_projects(&self) {
        if !crate::overseer::config::overseer_enabled() { return; }
        let Some(cfg) = crate::overseer::config::OverseerConfig::from_env() else {
            tracing::warn!("Overseer enabled but KIMI_API_KEY missing — skipping spawn");
            return;
        };
        let projects = self.mgr.db().list_projects().unwrap_or_default();
        for p in projects { self.spawn_one(p.id, cfg.clone()).await; }
        self.start_lifecycle_listener();
    }

    fn spawn_one(&self, project_id: Uuid, cfg: crate::overseer::config::OverseerConfig)
        -> impl std::future::Future<Output = ()> + '_
    {
        async move {
            if self.tasks.lock().unwrap().contains_key(&project_id) { return; }
            let mgr = Arc::clone(&self.mgr);
            let handle = tokio::spawn(run_overseer_loop(mgr, project_id, cfg));
            self.tasks.lock().unwrap().insert(project_id, handle);
        }
    }

    fn start_lifecycle_listener(&self) {
        // Note: there is no DomainEvent::ProjectDeleted today. As a stop-gap,
        // the supervisor polls list_projects() every 30s and reconciles.
        let mgr = Arc::clone(&self.mgr);
        let tasks = Arc::clone(&self.tasks);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let live: std::collections::HashSet<Uuid> = mgr.db().list_projects()
                    .unwrap_or_default().into_iter().map(|p| p.id).collect();
                let mut t = tasks.lock().unwrap();
                t.retain(|pid, h| {
                    if !live.contains(pid) { h.abort(); false } else { true }
                });
            }
        });
    }
}

async fn run_overseer_loop(
    mgr: Arc<SessionManager>,
    project_id: Uuid,
    cfg: crate::overseer::config::OverseerConfig,
) {
    use std::time::Duration;
    let mut state = crate::overseer::recommender::OverseerState::new(cfg.daily_token_budget);
    let mut obs = crate::overseer::observer::Observer::new(project_id, mgr.event_bus().subscribe());
    let kimi = crate::overseer::kimi::KimiClient::new(&cfg.api_key, &cfg.base_url);

    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let mut periodic = tokio::time::interval(Duration::from_secs(600));
    interval.tick().await; periodic.tick().await;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                obs.drain_pending(500).await;
                if obs.snapshot().events.is_empty() { continue; }
            }
            _ = periodic.tick() => {
                obs.drain_pending(500).await;
            }
        }
        let mut snap = obs.snapshot();
        if let Ok(p) = mgr.db().get_project(project_id) {
            obs.hydrate_artifacts(&mut snap, std::path::Path::new(&p.path));
        }
        let bus = mgr.event_bus().clone();
        let kc = &kimi;
        let _ = crate::overseer::recommender::review(
            mgr.db(), &bus, project_id, &snap, &mut state,
            move |s, u| async move { kc.chat(&s, &u).await }
        ).await;
    }
}
```

In `src/main.rs`, after the phase-reactor start:

```rust
if galvolite::overseer::config::overseer_enabled() {
    let sup = galvolite::overseer::OverseerSupervisor::new(Arc::clone(&mgr));
    sup.spawn_for_existing_projects().await;
    Box::leak(Box::new(sup));
}
```

- [ ] **Step 4: Run tests**

Run: `GALVOLITE_OVERSEER=1 KIMI_API_KEY=test cargo test --test overseer_supervisor`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/overseer/mod.rs src/main.rs tests/overseer_supervisor.rs
git commit -m "feat(overseer): per-project supervisor lifecycle"
```

---

## Task 13: Recommendations REST — list endpoint

**Files:**
- Create: `src/api/recommendations.rs`
- Modify: `src/api/mod.rs` (register module + route)
- Test: `tests/api_recommendations_list.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/api_recommendations_list.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;

#[tokio::test]
async fn list_returns_pending_recs_for_project() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("recs").await;
    let id = galvolite::recommendations::insert(app.mgr.db(), galvolite::recommendations::NewRec {
        project_id: project.id, kind: "k".into(), severity: "warn".into(),
        title: "t".into(), body: "b".into(), rationale: "r".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let resp = app.get(&format!("/api/projects/{}/recommendations", project.id)).await;
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], id.to_string());
}

#[tokio::test]
async fn list_filters_by_status_query_param() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("recs2").await;
    let id = galvolite::recommendations::insert(app.mgr.db(), galvolite::recommendations::NewRec {
        project_id: project.id, kind: "k".into(), severity: "info".into(),
        title: "t".into(), body: "b".into(), rationale: "r".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    galvolite::recommendations::set_status(app.mgr.db(), id, galvolite::recommendations::RecStatus::Acted).unwrap();
    let resp = app.get(&format!("/api/projects/{}/recommendations?status=acted", project.id)).await;
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_recommendations_list`
Expected: FAIL — endpoint missing → 404.

- [ ] **Step 3: Implement endpoint**

Create `src/api/recommendations.rs`:

```rust
use axum::{extract::{Path, Query, State}, http::StatusCode, response::{IntoResponse, Response}, Json};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::recommendations::{self, RecStatus};
use crate::session_manager::SessionManager;

#[derive(Deserialize)]
pub struct ListQ { pub status: Option<String> }

pub async fn list(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<ListQ>,
) -> Response {
    let status = match q.status.as_deref() {
        None | Some("pending") => RecStatus::Pending,
        Some("acted") => RecStatus::Acted,
        Some("dismissed") => RecStatus::Dismissed,
        Some("deferred") => RecStatus::Deferred,
        _ => return (StatusCode::BAD_REQUEST, "bad status").into_response(),
    };
    match recommendations::list_by_status(mgr.db(), project_id, status) {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => crate::api::err_to_response(e),
    }
}
```

In `src/api/mod.rs`:

```rust
pub mod execute_plan;
pub mod recommendations;

// in router():
.route("/api/projects/:id/recommendations", get(recommendations::list))
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_recommendations_list`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/api/recommendations.rs src/api/mod.rs tests/api_recommendations_list.rs
git commit -m "feat(api): GET /api/projects/:id/recommendations"
```

---

## Task 14: Recommendations REST — act endpoint (Make / Dismiss)

**Files:**
- Modify: `src/api/recommendations.rs` (add `act` handler + concat helper)
- Modify: `src/api/mod.rs` (register route)
- Modify: `src/session_manager.rs` (add `append_pending_overseer_recs`)
- Test: `tests/api_recommendations_act.rs`

- [ ] **Step 1: Add SessionManager helper**

In `src/session_manager.rs`:

```rust
/// Append a block to `pending_overseer_recs` for a session. Concatenates
/// with `\n\n---\n\n` if there is existing content (append-only — never
/// silently overwrites).
pub fn append_pending_overseer_recs(&self, session_id: Uuid, block: &str) -> Result<()> {
    let conn = self.db.pool().get()?;
    let prior: Option<String> = conn.query_row(
        "SELECT pending_overseer_recs FROM sessions WHERE id = ?1",
        [session_id.to_string()], |r| r.get(0),
    ).ok().flatten();
    let merged = match prior {
        Some(p) if !p.is_empty() => format!("{p}\n\n---\n\n{block}"),
        _ => block.to_string(),
    };
    conn.execute(
        "UPDATE sessions SET pending_overseer_recs = ?1 WHERE id = ?2",
        rusqlite::params![merged, session_id.to_string()],
    )?;
    Ok(())
}
```

- [ ] **Step 2: Write the failing test**

Create `tests/api_recommendations_act.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;
use galvolite::recommendations::{self, NewRec, RecStatus};

#[tokio::test]
async fn make_appends_block_to_orchestrator_pending_overseer_recs() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("act").await;
    let orch = app.spawn_orchestrator(project.id).await;

    let r1 = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "warn".into(),
        title: "Add tests".into(), body: "Body 1".into(), rationale: "Why 1".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let r2 = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "info".into(),
        title: "Refactor".into(), body: "Body 2".into(), rationale: "Why 2".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();

    let resp = app.post(&format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r1, r2], "action": "make"})).await;
    assert_eq!(resp.status(), 200);

    let s = app.mgr.db().get_session(orch.id).unwrap();
    let block = s.pending_overseer_recs.unwrap();
    assert!(block.contains("Add tests"));
    assert!(block.contains("Refactor"));

    let r1_row = recommendations::get(app.mgr.db(), r1).unwrap();
    assert_eq!(r1_row.status, "acted");
}

#[tokio::test]
async fn make_with_no_orchestrator_returns_409() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("act2").await;
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "warn".into(),
        title: "T".into(), body: "B".into(), rationale: "R".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let resp = app.post(&format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r], "action": "make"})).await;
    assert_eq!(resp.status(), 409);
}

#[tokio::test]
async fn dismiss_marks_status_only() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("dismiss").await;
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "info".into(),
        title: "T".into(), body: "B".into(), rationale: "R".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let resp = app.post(&format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r], "action": "dismiss"})).await;
    assert_eq!(resp.status(), 200);
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    assert_eq!(row.status, "dismissed");
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test --test api_recommendations_act`
Expected: FAIL — handler missing → 404.

- [ ] **Step 4: Implement act handler**

Append to `src/api/recommendations.rs`:

```rust
#[derive(Deserialize)]
pub struct ActReq { pub rec_ids: Vec<Uuid>, pub action: String }

pub async fn act(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<ActReq>,
) -> Response {
    if req.action == "make" {
        let Some(orch) = mgr.find_orchestrator_for_project(project_id) else {
            return (StatusCode::CONFLICT, "no orchestrator").into_response();
        };
        // Filter to currently-pending recs only (race protection).
        let pending: Vec<recommendations::Recommendation> = req.rec_ids.iter()
            .filter_map(|id| recommendations::get(mgr.db(), *id).ok())
            .filter(|r| r.status == "pending" && r.project_id == project_id)
            .collect();

        if pending.is_empty() {
            return (StatusCode::OK, Json(serde_json::json!({"results": []}))).into_response();
        }
        let block = concat_for_injection(&pending);
        if let Err(e) = mgr.append_pending_overseer_recs(orch.id, &block) {
            return crate::api::err_to_response(e);
        }
        let mut results = Vec::new();
        for r in &pending {
            let outcome = match recommendations::set_status(mgr.db(), r.id, RecStatus::Acted) {
                Ok(()) => "acted", Err(_) => "error",
            };
            results.push(serde_json::json!({"rec_id": r.id, "outcome": outcome}));
            mgr.event_bus().publish(crate::api::events::DomainEvent::RecommendationActed {
                rec_id: r.id, action: "make".into(), route: Some(orch.id),
            });
        }
        return Json(serde_json::json!({"results": results})).into_response();
    }
    if req.action == "dismiss" {
        for id in &req.rec_ids {
            let _ = recommendations::set_status(mgr.db(), *id, RecStatus::Dismissed);
            mgr.event_bus().publish(crate::api::events::DomainEvent::RecommendationActed {
                rec_id: *id, action: "dismiss".into(), route: None,
            });
        }
        return Json(serde_json::json!({"dismissed": req.rec_ids.len()})).into_response();
    }
    (StatusCode::BAD_REQUEST, "action must be 'make' or 'dismiss'").into_response()
}

pub fn concat_for_injection(recs: &[recommendations::Recommendation]) -> String {
    let mut s = format!("## Apply these {} recommendation(s):\n\n", recs.len());
    for (i, r) in recs.iter().enumerate() {
        s.push_str(&format!("### Rec {}: {}\n\n{}\n\n_Rationale: {}_\n", i + 1, r.title, r.body, r.rationale));
        if i + 1 < recs.len() { s.push_str("\n---\n\n"); }
    }
    s
}
```

In `src/api/events.rs`, add:

```rust
#[serde(rename = "recommendation.acted")]
RecommendationActed { rec_id: Uuid, action: String, route: Option<Uuid> },
```

Extend `applies_to`:

```rust
DomainEvent::RecommendationActed { .. } => false,
```

In `src/api/mod.rs`:

```rust
.route("/api/projects/:id/recommendations/act", post(recommendations::act))
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test api_recommendations_act`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add src/api/recommendations.rs src/api/mod.rs src/api/events.rs src/session_manager.rs \
        tests/api_recommendations_act.rs
git commit -m "feat(api): POST /recommendations/act — make/dismiss with append-only injection"
```

---

## Task 15: Recommendations REST — defer endpoint + cron sweep

**Files:**
- Modify: `src/api/recommendations.rs` (add `defer` handler)
- Modify: `src/api/mod.rs`
- Modify: `src/main.rs` (start sweep task)
- Test: `tests/api_recommendations_defer.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/api_recommendations_defer.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;
use galvolite::recommendations::{self, NewRec, RecStatus};

#[tokio::test]
async fn defer_default_24h_sets_deferred_until() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("defer").await;
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "info".into(),
        title: "T".into(), body: "B".into(), rationale: "R".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let resp = app.post(&format!("/api/projects/{}/recommendations/defer", project.id),
        serde_json::json!({"rec_id": r})).await;
    assert_eq!(resp.status(), 200);
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    assert_eq!(row.status, "deferred");
    assert!(row.deferred_until.is_some());
}

#[tokio::test]
async fn defer_with_explicit_hours_overrides_default() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("defer2").await;
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "info".into(),
        title: "T".into(), body: "B".into(), rationale: "R".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let resp = app.post(&format!("/api/projects/{}/recommendations/defer", project.id),
        serde_json::json!({"rec_id": r, "hours": 1})).await;
    assert_eq!(resp.status(), 200);
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    let until = row.deferred_until.unwrap();
    let now = chrono::Utc::now();
    let delta = (until - now).num_minutes();
    assert!((50..=70).contains(&delta), "expected ~60min, got {delta}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_recommendations_defer`
Expected: FAIL — handler missing.

- [ ] **Step 3: Implement defer handler**

Append to `src/api/recommendations.rs`:

```rust
#[derive(Deserialize)]
pub struct DeferReq { pub rec_id: Uuid, pub hours: Option<i64> }

pub async fn defer_handler(
    State(mgr): State<Arc<SessionManager>>,
    Path(_project_id): Path<Uuid>,
    Json(req): Json<DeferReq>,
) -> Response {
    let until = chrono::Utc::now() + chrono::Duration::hours(req.hours.unwrap_or(24));
    match recommendations::defer(mgr.db(), req.rec_id, until) {
        Ok(()) => {
            mgr.event_bus().publish(crate::api::events::DomainEvent::RecommendationActed {
                rec_id: req.rec_id, action: "defer".into(), route: None,
            });
            Json(serde_json::json!({"deferred_until": until.to_rfc3339()})).into_response()
        }
        Err(e) => crate::api::err_to_response(e),
    }
}
```

In `src/api/mod.rs`:

```rust
.route("/api/projects/:id/recommendations/defer", post(recommendations::defer_handler))
```

In `src/main.rs`, add a sweep task after Overseer init:

```rust
{
    let mgr = Arc::clone(&mgr);
    tokio::spawn(async move {
        let mut t = tokio::time::interval(std::time::Duration::from_secs(300));
        t.tick().await;
        loop {
            t.tick().await;
            let _ = galvolite::recommendations::resurface_expired(mgr.db(), chrono::Utc::now());
        }
    });
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_recommendations_defer`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/api/recommendations.rs src/api/mod.rs src/main.rs \
        tests/api_recommendations_defer.rs
git commit -m "feat(api): POST /recommendations/defer + sweep task"
```

---

## Task 16: Recommendations REST — explain endpoint

**Files:**
- Modify: `src/api/recommendations.rs` (add `explain` handler)
- Modify: `src/api/mod.rs`
- Test: `tests/api_recommendations_explain.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/api_recommendations_explain.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;
use galvolite::recommendations::{self, NewRec};

#[tokio::test]
async fn explain_updates_rationale_field() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("xpl").await;
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "info".into(),
        title: "T".into(), body: "B".into(), rationale: "old".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let resp = app.post(&format!("/api/projects/{}/recommendations/explain", project.id),
        serde_json::json!({"rec_id": r, "rationale": "new and better"})).await;
    assert_eq!(resp.status(), 200);
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    assert_eq!(row.rationale, "new and better");
    assert_eq!(row.status, "pending");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_recommendations_explain`
Expected: FAIL — handler missing.

- [ ] **Step 3: Implement explain handler**

Append to `src/api/recommendations.rs`:

```rust
#[derive(Deserialize)]
pub struct ExplainReq { pub rec_id: Uuid, pub rationale: String }

pub async fn explain(
    State(mgr): State<Arc<SessionManager>>,
    Path(_project_id): Path<Uuid>,
    Json(req): Json<ExplainReq>,
) -> Response {
    match recommendations::update_rationale(mgr.db(), req.rec_id, &req.rationale) {
        Ok(()) => {
            mgr.event_bus().publish(crate::api::events::DomainEvent::RecommendationActed {
                rec_id: req.rec_id, action: "explain".into(), route: None,
            });
            Json(serde_json::json!({"updated": true})).into_response()
        }
        Err(e) => crate::api::err_to_response(e),
    }
}
```

In `src/api/mod.rs`:

```rust
.route("/api/projects/:id/recommendations/explain", post(recommendations::explain))
```

> **Note on the spec's "ask Kimi for a better explanation":** Plan 8 keeps the
> explain endpoint as a write-only updater; the trigger to re-prompt Kimi is
> deferred to a follow-up so as not to block this plan. Operators can pass
> any rationale string; the Overseer's next periodic tick will refresh the
> rationale naturally if it discovers a better one.

- [ ] **Step 4: Run tests**

Run: `cargo test --test api_recommendations_explain`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/api/recommendations.rs src/api/mod.rs tests/api_recommendations_explain.rs
git commit -m "feat(api): POST /recommendations/explain — update rationale"
```

---

## Task 17: Orchestrator UserPromptSubmit hook fetches/clears pending_overseer_recs

**Files:**
- Modify: `src/api/hook_handlers/turn.rs` (extend `on_user_prompt_submit`)
- Test: `tests/orch_pending_overseer_recs_consumed.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/orch_pending_overseer_recs_consumed.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;

#[tokio::test]
async fn user_prompt_submit_for_orch_drains_pending_overseer_recs_into_inject_text() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("orch-drain").await;
    let orch = app.spawn_orchestrator(project.id).await;
    app.mgr.append_pending_overseer_recs(orch.id, "## Apply these 1 recommendation(s):\n\n### Rec 1: T\n\nB").unwrap();

    // Fire UserPromptSubmit hook for orch.
    let resp = app.post_hook(orch.id, galvolite::hooks::types::HookKind::UserPromptSubmit,
        serde_json::json!({"est_tokens": 0})).await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let inject = body.get("inject_text").and_then(|v| v.as_str()).unwrap_or("");
    assert!(inject.contains("Apply these 1 recommendation"));

    // Verify field cleared.
    let s = app.mgr.db().get_session(orch.id).unwrap();
    assert!(s.pending_overseer_recs.is_none());
}

#[tokio::test]
async fn non_orch_session_is_not_affected() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("impl").await;
    let s = app.mgr.spawn_session(project.id, "task".into(), "implementation".into(),
        "true".into(), vec![]).await.unwrap();
    app.mgr.append_pending_overseer_recs(s.id, "should not be drained").unwrap();
    let _ = app.post_hook(s.id, galvolite::hooks::types::HookKind::UserPromptSubmit,
        serde_json::json!({"est_tokens": 0})).await;
    let row = app.mgr.db().get_session(s.id).unwrap();
    assert_eq!(row.pending_overseer_recs.as_deref(), Some("should not be drained"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test orch_pending_overseer_recs_consumed`
Expected: FAIL — drain not implemented.

- [ ] **Step 3: Extend on_user_prompt_submit**

In `src/api/hook_handlers/turn.rs`, replace the body of `on_user_prompt_submit`:

```rust
pub fn on_user_prompt_submit(
    db: &Db,
    session_id: Uuid,
    payload: &serde_json::Value,
) -> Option<String> {
    let est = payload.get("est_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
    let conn = db.pool().get().ok()?;
    let _ = conn.execute(
        "UPDATE sessions SET turn_count = COALESCE(turn_count,0)+1,
                              est_tokens = COALESCE(est_tokens,0)+?2
         WHERE id = ?1",
        rusqlite::params![session_id.to_string(), est],
    );

    let total: i64 = conn.query_row(
        "SELECT COALESCE(est_tokens, 0) FROM sessions WHERE id = ?1",
        [session_id.to_string()], |r| r.get(0),
    ).unwrap_or(0);

    let warn = if total * 100 >= CONTEXT_LIMIT * 80 {
        Some(format!(
            "Context approaching limit ({total}/{CONTEXT_LIMIT} tokens, {}%). \
             Consider write_handoff before context runs out.",
            (total * 100) / CONTEXT_LIMIT
        ))
    } else { None };

    // Drain pending_overseer_recs for orchestrator sessions.
    let role: Option<String> = conn.query_row(
        "SELECT role FROM sessions WHERE id = ?1",
        [session_id.to_string()], |r| r.get(0),
    ).ok();
    if role.as_deref() == Some("orchestrator") {
        let pending: Option<String> = conn.query_row(
            "SELECT pending_overseer_recs FROM sessions WHERE id = ?1",
            [session_id.to_string()], |r| r.get(0),
        ).ok().flatten();
        if let Some(block) = pending {
            let _ = conn.execute(
                "UPDATE sessions SET pending_overseer_recs = NULL WHERE id = ?1",
                [session_id.to_string()],
            );
            let combined = match warn {
                Some(w) => format!("{w}\n\n{block}"),
                None => block,
            };
            return Some(combined);
        }
    }
    warn
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test orch_pending_overseer_recs_consumed`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src/api/hook_handlers/turn.rs tests/orch_pending_overseer_recs_consumed.rs
git commit -m "feat(orch-hook): drain pending_overseer_recs into UserPromptSubmit inject_text"
```

---

## Task 18: ORCH_PROFILE — rec-routing rule (>3 → spawn_child)

**Files:**
- Modify: `src/claude_md.rs` (extend `ORCH_PROFILE`)
- Test: `tests/claude_md_orch_rec_routing.rs`

- [ ] **Step 1: Write the failing test**

Create `tests/claude_md_orch_rec_routing.rs`:

```rust
#[test]
fn orch_profile_documents_rec_routing_rule() {
    let p = galvolite::claude_md::orchestrator_profile();
    assert!(p.contains("Overseer recommendations"), "ORCH_PROFILE missing Overseer section");
    assert!(p.contains("≤ 3"), "ORCH_PROFILE missing inline-vs-fanout threshold");
    assert!(p.contains("spawn_child"), "ORCH_PROFILE missing spawn_child fan-out instruction");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test claude_md_orch_rec_routing`
Expected: FAIL — section missing.

- [ ] **Step 3: Add Overseer section to ORCH_PROFILE**

In `src/claude_md.rs`, find the `ORCH_PROFILE` constant and append a new section before its closing `"`:

```
## Overseer recommendations

When you receive `additionalContext` containing a block headed `## Apply
these N recommendation(s):`, treat each one as a routing decision, NOT
a task to execute yourself.

- If N ≤ 3: handle inline. Walk the recs in order; for each, decide
  whether it's a tweak you can hand to an existing waiting child or a
  new fan-out. Use `send_to_session` for tweaks, `spawn_child` for new
  work. Do not write code yourself.
- If N > 3: fan out via `spawn_child` per rec. Use the rec title as the
  child's task summary. The implementation profile owns the actual fix.

The Overseer is advisory. You may decline a rec; if you do, leave a
`write_postmortem` note explaining why so the operator can review.
```

- [ ] **Step 4: Run tests**

Run: `cargo test --test claude_md_orch_rec_routing`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/claude_md.rs tests/claude_md_orch_rec_routing.rs
git commit -m "feat(claude-md): ORCH_PROFILE rec-routing rule (≤3 inline, >3 fan out)"
```

---

## Task 19: RecBell React component

**Files:**
- Modify: `ui/src/types.ts` (add `Recommendation` type + new DomainEvent variants)
- Modify: `ui/src/api.ts` (add `listRecs`, `actRecs`, `deferRec`, `explainRec`)
- Create: `ui/src/components/RecBell.tsx`
- Create: `ui/src/components/RecBell.test.tsx`

- [ ] **Step 1: Add types**

In `ui/src/types.ts`:

```ts
export interface Recommendation {
  id: string;
  project_id: string;
  kind: string;
  severity: "info" | "warn" | "urgent";
  title: string;
  body: string;
  rationale: string;
  evidence_json: unknown;
  status: "pending" | "acted" | "dismissed" | "deferred";
  created_at: string;
  acted_at: string | null;
  deferred_until: string | null;
}
```

Add to the `DomainEvent` union:

```ts
| { type: "recommendation.created"; rec_id: string; project_id: string; severity: string }
| { type: "recommendation.acted"; rec_id: string; action: string; route: string | null }
| { type: "overseer.degraded"; project_id: string; reason: string }
| { type: "overseer.recovered"; project_id: string }
```

- [ ] **Step 2: Add API client**

In `ui/src/api.ts`:

```ts
listRecs: (projectId: string, status: string = "pending") =>
  request<Recommendation[]>(
    `/api/projects/${projectId}/recommendations?status=${status}`,
  ),

actRecs: (projectId: string, recIds: string[], action: "make" | "dismiss") =>
  request<{ results?: { rec_id: string; outcome: string }[]; dismissed?: number }>(
    `/api/projects/${projectId}/recommendations/act`,
    { method: "POST", body: JSON.stringify({ rec_ids: recIds, action }) },
  ),

deferRec: (projectId: string, recId: string, hours?: number) =>
  request<{ deferred_until: string }>(
    `/api/projects/${projectId}/recommendations/defer`,
    { method: "POST", body: JSON.stringify({ rec_id: recId, ...(hours != null ? { hours } : {}) }) },
  ),

explainRec: (projectId: string, recId: string, rationale: string) =>
  request<{ updated: boolean }>(
    `/api/projects/${projectId}/recommendations/explain`,
    { method: "POST", body: JSON.stringify({ rec_id: recId, rationale }) },
  ),
```

Add `Recommendation` to the imports at the top of `api.ts`.

- [ ] **Step 3: Write the failing test**

Create `ui/src/components/RecBell.test.tsx`:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RecBell } from "./RecBell";
import { api } from "../api";

vi.mock("../api", () => ({
  api: { listRecs: vi.fn() },
}));

describe("RecBell", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders count badge for pending recs", async () => {
    (api.listRecs as any).mockResolvedValue([
      { id: "r1", severity: "info" }, { id: "r2", severity: "warn" },
    ]);
    render(<RecBell projectId="p1" onOpen={() => {}} degraded={false} />);
    expect(await screen.findByText("2")).toBeInTheDocument();
  });

  it("hides badge when count is 0", async () => {
    (api.listRecs as any).mockResolvedValue([]);
    render(<RecBell projectId="p1" onOpen={() => {}} degraded={false} />);
    expect(await screen.findByLabelText(/recommendations/i)).toBeInTheDocument();
    expect(screen.queryByText(/^[0-9]+$/)).toBeNull();
  });

  it("calls onOpen when clicked", async () => {
    (api.listRecs as any).mockResolvedValue([{ id: "r1", severity: "info" }]);
    const onOpen = vi.fn();
    render(<RecBell projectId="p1" onOpen={onOpen} degraded={false} />);
    const btn = await screen.findByRole("button", { name: /recommendations/i });
    fireEvent.click(btn);
    expect(onOpen).toHaveBeenCalled();
  });

  it("shows degraded styling when prop set", async () => {
    (api.listRecs as any).mockResolvedValue([]);
    const { container } = render(<RecBell projectId="p1" onOpen={() => {}} degraded={true} />);
    expect(container.querySelector(".text-yellow-400, .text-yellow-500")).toBeTruthy();
  });
});
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cd ui && npm test -- RecBell`
Expected: FAIL — component missing.

- [ ] **Step 5: Create component**

Create `ui/src/components/RecBell.tsx`:

```tsx
import { useEffect, useState } from "react";
import { Bell } from "lucide-react";
import { api } from "../api";

interface Props { projectId: string; onOpen: () => void; degraded: boolean; }

export function RecBell({ projectId, onOpen, degraded }: Props) {
  const [count, setCount] = useState(0);

  useEffect(() => {
    let cancelled = false;
    api.listRecs(projectId, "pending")
      .then((rs) => !cancelled && setCount(rs.length))
      .catch(() => !cancelled && setCount(0));
    return () => { cancelled = true; };
  }, [projectId]);

  const color = degraded ? "text-yellow-400" : "text-neutral-300";
  return (
    <button
      type="button"
      aria-label="recommendations"
      onClick={onOpen}
      className="relative px-2 py-1 hover:bg-neutral-800 rounded"
    >
      <Bell size={16} className={color} />
      {count > 0 && (
        <span className="absolute -top-1 -right-1 bg-red-600 text-white text-[10px] rounded-full w-4 h-4 flex items-center justify-center">
          {count}
        </span>
      )}
    </button>
  );
}
```

- [ ] **Step 6: Run tests**

Run: `cd ui && npm test -- RecBell`
Expected: PASS (4 tests).

- [ ] **Step 7: Commit**

```bash
git add ui/src/types.ts ui/src/api.ts ui/src/components/RecBell.tsx ui/src/components/RecBell.test.tsx
git commit -m "feat(ui): RecBell with count badge + degraded styling"
```

---

## Task 20: RecModal React component

**Files:**
- Create: `ui/src/components/RecModal.tsx`
- Create: `ui/src/components/RecModal.test.tsx`

- [ ] **Step 1: Write the failing test**

Create `ui/src/components/RecModal.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { RecModal } from "./RecModal";
import { api } from "../api";

vi.mock("../api", () => ({
  api: {
    listRecs: vi.fn(),
    actRecs: vi.fn(),
    deferRec: vi.fn(),
  },
}));

const sample = (id: string, sev: any = "warn") => ({
  id, project_id: "p1", kind: "k", severity: sev,
  title: `Rec ${id}`, body: "B", rationale: "R", evidence_json: [],
  status: "pending", created_at: "t", acted_at: null, deferred_until: null,
});

describe("RecModal", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders pending recs with severity badges", async () => {
    (api.listRecs as any).mockResolvedValue([sample("r1", "warn"), sample("r2", "urgent")]);
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    expect(await screen.findByText("Rec r1")).toBeInTheDocument();
    expect(screen.getByText("Rec r2")).toBeInTheDocument();
    expect(screen.getAllByText(/warn|urgent/i).length).toBeGreaterThanOrEqual(2);
  });

  it("multi-select Make sends rec_ids array to actRecs", async () => {
    (api.listRecs as any).mockResolvedValue([sample("r1"), sample("r2")]);
    (api.actRecs as any).mockResolvedValue({ results: [] });
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    await screen.findByText("Rec r1");
    fireEvent.click(screen.getByLabelText("select Rec r1"));
    fireEvent.click(screen.getByLabelText("select Rec r2"));
    fireEvent.click(screen.getByRole("button", { name: /make selected/i }));
    await waitFor(() => expect(api.actRecs).toHaveBeenCalledWith("p1", ["r1", "r2"], "make"));
  });

  it("Defer button calls deferRec for clicked row", async () => {
    (api.listRecs as any).mockResolvedValue([sample("r1")]);
    (api.deferRec as any).mockResolvedValue({ deferred_until: "x" });
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    await screen.findByText("Rec r1");
    fireEvent.click(screen.getByRole("button", { name: /defer/i }));
    await waitFor(() => expect(api.deferRec).toHaveBeenCalledWith("p1", "r1"));
  });

  it("empty state shown when no recs", async () => {
    (api.listRecs as any).mockResolvedValue([]);
    render(<RecModal projectId="p1" open onClose={() => {}} />);
    expect(await screen.findByText(/no pending/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- RecModal`
Expected: FAIL — component missing.

- [ ] **Step 3: Create component**

Create `ui/src/components/RecModal.tsx`:

```tsx
import { useEffect, useState } from "react";
import { api } from "../api";
import type { Recommendation } from "../types";

interface Props { projectId: string; open: boolean; onClose: () => void; }

export function RecModal({ projectId, open, onClose }: Props) {
  const [recs, setRecs] = useState<Recommendation[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    api.listRecs(projectId, "pending").then(setRecs).catch(() => setRecs([]));
  }, [open, projectId]);

  if (!open) return null;

  const refresh = () => api.listRecs(projectId, "pending").then(setRecs);

  const sevColor = (s: string) =>
    s === "urgent" ? "bg-red-700" : s === "warn" ? "bg-yellow-700" : "bg-neutral-700";

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-neutral-900 rounded-lg shadow-xl p-4 w-[640px] max-h-[80vh] overflow-y-auto">
        <div className="flex items-center justify-between mb-3">
          <h2 className="text-lg font-semibold">Overseer recommendations</h2>
          <button onClick={onClose} className="text-neutral-400 hover:text-white">×</button>
        </div>
        {recs.length === 0 && (
          <div className="text-neutral-500 text-sm py-8 text-center">No pending recommendations.</div>
        )}
        {recs.map((r) => (
          <div key={r.id} className="border border-neutral-700 rounded p-3 mb-2">
            <div className="flex items-start gap-2">
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
              <div className="flex flex-col gap-1">
                <button
                  className="text-xs px-2 py-0.5 rounded bg-neutral-700 hover:bg-neutral-600"
                  disabled={busy}
                  onClick={async () => { setBusy(true); await api.deferRec(projectId, r.id); await refresh(); setBusy(false); }}
                >Defer</button>
              </div>
            </div>
          </div>
        ))}
        {recs.length > 0 && (
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

- [ ] **Step 4: Run tests**

Run: `cd ui && npm test -- RecModal`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/RecModal.tsx ui/src/components/RecModal.test.tsx
git commit -m "feat(ui): RecModal with multi-select Make/Defer/Dismiss"
```

---

## Task 21: Wire RecBell + RecModal + ExecutePlanButton into Dashboard

**Files:**
- Modify: `ui/src/components/Dashboard.tsx`

- [ ] **Step 1: Read existing Dashboard**

Read `ui/src/components/Dashboard.tsx` to find the top-bar render block (search for "ProjectSelector" or "SpawnControls").

- [ ] **Step 2: Modify Dashboard**

In `ui/src/components/Dashboard.tsx`:

1. Add imports near the top:

```tsx
import { RecBell } from "./RecBell";
import { RecModal } from "./RecModal";
import { ExecutePlanButton } from "./ExecutePlanButton";
```

2. Add state hooks inside `Dashboard`:

```tsx
const [recModalOpen, setRecModalOpen] = useState(false);
const [overseerDegraded, setOverseerDegraded] = useState(false);
```

3. In the existing global event handler (`bridgeHandler` or `onGlobalEvent` callback path), append:

```tsx
if (e.type === "overseer.degraded") setOverseerDegraded(true);
if (e.type === "overseer.recovered") setOverseerDegraded(false);
```

4. In the top-bar JSX (next to `SpawnControls`), insert:

```tsx
{selectedProject && (
  <>
    <ExecutePlanButton projectId={selectedProject} />
    <RecBell projectId={selectedProject} degraded={overseerDegraded} onOpen={() => setRecModalOpen(true)} />
    <RecModal projectId={selectedProject} open={recModalOpen} onClose={() => setRecModalOpen(false)} />
  </>
)}
```

> Decide between **replacing** SpawnControls or sitting beside it: per spec §3 module map, `ExecutePlanButton` "Replaces / sits beside SpawnControls; same gate logic." Sit beside in Plan 8 — keep existing `SpawnControls` as the manual escape hatch.

- [ ] **Step 3: Verify build**

Run: `cd ui && npm run build`
Expected: build succeeds with no type errors.

- [ ] **Step 4: Commit**

```bash
git add ui/src/components/Dashboard.tsx
git commit -m "feat(ui): wire RecBell + RecModal + ExecutePlanButton into Dashboard top bar"
```

---

## Task 22: Recommendations lifecycle integration test

**Files:**
- Create: `tests/recommendations_lifecycle.rs`

- [ ] **Step 1: Write the integration test**

Create `tests/recommendations_lifecycle.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;
use galvolite::recommendations::{self, NewRec, RecStatus};

#[tokio::test]
async fn full_lifecycle_pending_acted_deferred_dismissed_resurfaced() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("lc").await;
    let _orch = app.spawn_orchestrator(project.id).await;

    // Insert.
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "warn".into(),
        title: "T".into(), body: "B".into(), rationale: "R".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();

    // Act make.
    let r1 = app.post(&format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r], "action": "make"})).await;
    assert_eq!(r1.status(), 200);
    assert_eq!(recommendations::get(app.mgr.db(), r).unwrap().status, "acted");

    // Insert another, defer.
    let r2 = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k2".into(), severity: "info".into(),
        title: "T2".into(), body: "B2".into(), rationale: "R2".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    app.post(&format!("/api/projects/{}/recommendations/defer", project.id),
        serde_json::json!({"rec_id": r2, "hours": 0})).await;

    // Sweep — past 0h-deferred should resurface immediately.
    let n = recommendations::resurface_expired(app.mgr.db(), chrono::Utc::now() + chrono::Duration::seconds(1)).unwrap();
    assert_eq!(n, 1);
    assert_eq!(recommendations::get(app.mgr.db(), r2).unwrap().status, "pending");

    // Dismiss it.
    app.post(&format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r2], "action": "dismiss"})).await;
    assert_eq!(recommendations::get(app.mgr.db(), r2).unwrap().status, "dismissed");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --test recommendations_lifecycle`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/recommendations_lifecycle.rs
git commit -m "test(recommendations): full lifecycle pending→acted→deferred→resurfaced→dismissed"
```

---

## Task 23: Plan executor + observer cross-integration test

**Files:**
- Create: `tests/plan_executor_with_observer.rs`

- [ ] **Step 1: Write the test**

Create `tests/plan_executor_with_observer.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;
use galvolite::api::events::DomainEvent;

#[tokio::test]
async fn execute_plan_emits_phase_advanced_into_observer_ring() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = spawn_test_app().await;
    let project = app.create_project_with_plan_and_directive(
        "- [ ] alpha\n",
        "# Directive: x\nPlan: .galvolite/plan.md\n## Phases\n| ID | Parts | Autonomy | Depends on |\n| --- | --- | --- | --- |\n| p1 | 1 | manual | — |\n## Notes\n",
    ).await;
    app.spawn_orchestrator(project.id).await;

    let mut rx = app.mgr.event_bus().subscribe();
    app.post(&format!("/api/projects/{}/execute-plan", project.id), serde_json::json!({})).await;

    let mut got_advanced = false;
    let mut got_review = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(DomainEvent::PhaseAdvanced { phase_id, .. })) if phase_id == "p1" => got_advanced = true,
            Ok(Ok(DomainEvent::PhaseReviewNeeded { phase_id, .. })) if phase_id == "p1" => got_review = true,
            _ => {}
        }
        if got_advanced && got_review { break; }
    }
    assert!(got_advanced, "PhaseAdvanced not emitted");
    assert!(got_review, "PhaseReviewNeeded not emitted (autonomy=manual)");
}
```

- [ ] **Step 2: Run the test**

Run: `GALVOLITE_PLAN_EXECUTOR=1 cargo test --test plan_executor_with_observer`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/plan_executor_with_observer.rs
git commit -m "test(plan-executor): emits PhaseAdvanced + PhaseReviewNeeded events"
```

---

## Task 24: Smoke test — Overseer end-to-end with mocked Kimi

**Files:**
- Create: `tests/overseer_e2e_mocked.rs`

- [ ] **Step 1: Write the smoke test**

Create `tests/overseer_e2e_mocked.rs`:

```rust
mod common;
use common::test_app::spawn_test_app;
use galvolite::overseer::observer::{ContextSnapshot, EventRecord, Observer};
use galvolite::overseer::recommender::{review, OverseerState};
use galvolite::recommendations::{self, RecStatus};
use uuid::Uuid;

#[tokio::test]
async fn overseer_review_with_mock_kimi_inserts_rec_then_make_appends_to_orch() {
    let app = spawn_test_app().await;
    let project = app.create_project_in_temp_repo("e2e").await;
    let orch = app.spawn_orchestrator(project.id).await;

    // Stage a snapshot with one event.
    let mut snap = ContextSnapshot::default();
    snap.events.push(EventRecord {
        session_id: orch.id, kind: "mcp.status_report".into(), source: "mcp".into(),
        ts: "t".into(), payload: serde_json::json!({"kind":"stuck"}),
    });

    let kimi_text = r#"{"recommendations":[
        {"kind":"stuck","severity":"warn","title":"Session stuck","body":"investigate","rationale":"no events","evidence_refs":[]}
    ]}"#;

    let mut state = OverseerState::new(100_000);
    let bus = app.mgr.event_bus().clone();
    review(app.mgr.db(), &bus, project.id, &snap, &mut state,
        |_, _| async move { Ok::<_, galvolite::overseer::kimi::KimiError>((kimi_text.to_string(), 50)) }
    ).await.unwrap();

    let pending = recommendations::list_by_status(app.mgr.db(), project.id, RecStatus::Pending).unwrap();
    assert_eq!(pending.len(), 1);

    // Operator clicks Make via API.
    let resp = app.post(&format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [pending[0].id], "action": "make"})).await;
    assert_eq!(resp.status(), 200);

    let s = app.mgr.db().get_session(orch.id).unwrap();
    assert!(s.pending_overseer_recs.unwrap().contains("Session stuck"));
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --test overseer_e2e_mocked`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/overseer_e2e_mocked.rs
git commit -m "test(overseer): e2e mock Kimi → rec → make → orch append"
```

---

## Task 25: Full-build verification + memory updates

**Files:**
- Modify: `/Users/michaelcurtis/.claude/projects/-Volumes-Main-External-Development-lucidity-GalvoLite/memory/MEMORY.md`
- Create: `/Users/michaelcurtis/.claude/projects/-Volumes-Main-External-Development-lucidity-GalvoLite/memory/project_plan8_status.md`

- [ ] **Step 1: Run the full backend test suite**

Run: `cargo test`
Expected: ALL pass. Existing 298 backend tests + ~30 new from Plan 8 = ~328 total.

- [ ] **Step 2: Run the full UI test suite**

Run: `cd ui && npm test`
Expected: ALL pass. Existing 123 UI tests + 9 new from Plan 8 = ~132 total.

- [ ] **Step 3: Run cargo clippy + cargo build --release**

Run: `cargo clippy --all-targets -- -D warnings && cargo build --release`
Expected: no warnings, release binary builds.

- [ ] **Step 4: Manual smoke (skip if no KIMI_API_KEY available)**

Note in commit message whether the manual smoke was performed. If KIMI_API_KEY is set:

```bash
GALVOLITE_OVERSEER=1 GALVOLITE_PLAN_EXECUTOR=1 \
  KIMI_API_KEY=$KIMI_API_KEY \
  cargo run --release -- serve --port 4242
```

In another terminal: open dashboard, create project, write plan.md + directive.md, click Execute Plan, watch fan-out tree, wait for Overseer rec, multi-select 1, click Make, observe orchestrator session next turn injects the rec block.

- [ ] **Step 5: Write the memory file**

Create `/Users/michaelcurtis/.claude/projects/-Volumes-Main-External-Development-lucidity-GalvoLite/memory/project_plan8_status.md`:

```markdown
---
name: Plan 8 status
description: Plan 8 (Plan Executor + Overseer) shipped on main YYYY-MM-DD; X commits
type: project
---
Sub-plan 8 shipped on main YYYY-MM-DD across X commits (FIRST..LAST). Adds:

- `POST /api/projects/:id/execute-plan` + `ExecutePlanButton` (gated on plan + directive)
- Phase autonomy: full (auto-advance on rejoin), review (operator approval),
  manual (operator dispatches each phase)
- Overseer subsystem: per-project bg task using Kimi K2; `recommendations` table
  with content_hash dedupe; lifecycle pending/acted/dismissed/deferred
- `pending_overseer_recs` field on sessions mirroring `pending_rejoin` lane
- Dashboard rec-bell + rec-modal (multi-select Make/Defer/Dismiss)
- Orchestrator profile rec-routing rule (≤3 inline, >3 spawn_child)
- Feature flags `GALVOLITE_PLAN_EXECUTOR` and `GALVOLITE_OVERSEER` (default off)
- ~30 new backend tests, 9 new UI tests — all green.
```

(Replace `YYYY-MM-DD`, `X`, `FIRST..LAST` with the actual values when committing.)

Edit `MEMORY.md`, replace the Plan 8 brainstorm pointer line:

```
- [Plan 8 status](project_plan8_status.md) — DONE on main YYYY-MM-DD; ~30 backend tests, 9 UI tests
```

Delete the brainstorm pointer line (`project_plan8_brainstormed.md`); the brainstorm file itself can be removed in the same commit since the spec is now committed and the implementation has shipped.

- [ ] **Step 6: Final commit**

```bash
git add /Users/michaelcurtis/.claude/projects/-Volumes-Main-External-Development-lucidity-GalvoLite/memory/
git commit -m "docs(memory): Plan 8 DONE — Plan Executor + Overseer shipped"
```

---

## Self-Review Checklist (run AFTER all tasks complete)

- Spec §2 in-scope items: ✅ executor (Tasks 3–6), Overseer (Tasks 8–12), recs DB (Task 7), REST CRUD (Tasks 13–16), `pending_overseer_recs` (Task 1, used in 14, 17), bell + modal (Tasks 19–21), orch profile (Task 18), feature flags (Task 2), integration tests (Tasks 22–24).
- Spec §3 module map: every module listed has a task. `src/overseer/observer.rs` covered by Task 9; `src/overseer/recommender.rs` by Tasks 10–11; `src/overseer/kimi.rs` by Task 8.
- Spec §4 Flow A (plan execution): Task 4 implements; Task 5 phase reactor; Task 23 verifies events.
- Spec §4 Flow B (Overseer continuous loop): Task 12 supervisor calls `run_overseer_loop` with debounce + periodic + Kimi rate via implicit interval ticking. Token budget enforced in Task 11. Daily reset at UTC midnight is **not** explicitly implemented — see follow-up note below.
- Spec §4 Flow C (Rec action): Task 14 implements; Task 17 drains via UserPromptSubmit; Task 18 routes via profile.
- Spec §5 error handling: missing artifacts (Task 4), cycles (Task 3), no orch / double-execute (Task 4), Kimi failures + degraded (Task 11), token budget (Task 11), rec race (Task 14 filters to pending only).
- Spec §6 testing: pure-function tests (3, 7, 10), integration tests (4, 5, 13–17, 22–24), UI tests (6, 19, 20).

**Known follow-up gaps (deferred from Plan 8 — flag in commit messages):**

1. Daily token-budget reset at UTC midnight is not auto-reset; operators must restart the server. Acceptable for stabilization.
2. Auto-archive sweep (7-day pending → dismissed) is not in this plan; only the deferred-resurface sweep ships. Add in a future plan.
3. The "Better explanation" endpoint (Task 16) updates rationale but does not re-prompt Kimi — it's a write-only updater. Spec note in Task 16 explains this.
4. Per-session PTY tail rings are not collected — `ContextSnapshot.tail_per_session` is omitted from Task 9. The events ring + on-disk artifacts cover the most operationally useful signal; PTY tail can be added later if Overseer's recall feels thin.
5. Dashboard does not yet hide deferred recs behind a toggle; `RecModal` only shows pending. Add toggle when needed.

These are all incremental improvements that don't block Plan 8's user-visible promise (click Execute, walk to desk, see actionable recs).

---

Plan complete and saved to `docs/superpowers/plans/2026-05-02-plan-8-executor-overseer.md`. Two execution options:

1. **Subagent-Driven (recommended)** — Dispatch a fresh subagent per task, review between tasks, fast iteration. Given the size, dispatch tasks 1–8 (foundations + Plan Executor) as one cycle and tasks 9–25 (Overseer + UI + wiring + smoke) as a second cycle.

2. **Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
