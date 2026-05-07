# Plan 9 — Deferred Bundle Design

**Date:** 2026-05-06
**Status:** Draft (awaiting user review before writing-plans phase)
**Scope:** Single PLAN.md, 7 sequential tasks. Clear the Plan 8 deferred-items deck before Plan 10.

---

## Goal

Ship the seven items deferred from Plan 8 as a single Plan 9 release. No new
subsystems. Every task individually testable; the whole bundle is one feature
branch with seven commits.

The seven items, with the design call decided for each:

| # | Item | Decision |
|---|---|---|
| 1 | Defaults default-on flip | Invert default for `GALVOLITE_OVERSEER` and `GALVOLITE_PLAN_EXECUTOR`. Env vars become kill-switches. |
| 2 | 7-day rec auto-archive | Status-change-age sweep over `acted` / `dismissed` only. Deferred and pending exempt. |
| 3 | `ProjectDeleted` bus event | Add `DomainEvent::ProjectDeleted`, emit from `delete_project`, replace Overseer's polling workaround. |
| 4 | `tail_per_session` | Internal-only `ContextSnapshot` field. No HTTP endpoint, no UI. Recall-quality improvement for Overseer. |
| 5 | Deferred-recs UI toggle | Status-filter segmented control in `RecModal`. |
| 6 | UTC token-budget reset | In-memory `last_reset_utc` on `RecommenderState`. Check on each `try_consume`. No persistence. |
| 7 | `/recommendations/explain` UI button | "Re-explain" button per rec in `RecModal`. Inline rationale textarea. |

---

## Architecture

No new subsystems. Each task is a small, focused change in code that already
exists:

- **DB schema** gains one new status (`archived`) and one new column
  (`status_changed_at`) — single migration `0006_rec_archived.sql`.
- **Bus events** gain one variant (`ProjectDeleted { project_id }`).
- **Recommender state** gains one field (`last_reset_utc`).
- **Overseer config flags** flip default; no code change beyond the
  `unwrap_or` defaults.
- **`ContextSnapshot`** gains one field (`tail_per_session: HashMap<Uuid, String>`).
- **Cron tick** in `main.rs:39` already exists for deferred-rec resurfacing;
  task 2 piggybacks on the same 5-minute interval.
- **`RecModal`** gains a status-filter and a per-rec re-explain button.

Critical-path files that get touched:

- `src/recommendations.rs` — `RecStatus::Archived`, `archive_old`,
  `set_status` writes `status_changed_at`.
- `src/api/recommendations.rs` — accept `?status=archived` in list query.
- `src/api/events.rs` — `ProjectDeleted` variant.
- `src/session_manager.rs` — emit `ProjectDeleted` from `delete_project`.
- `src/overseer/mod.rs` — subscribe to `ProjectDeleted`, drop the existing
  workaround loop noted at `mod.rs:19-28`.
- `src/overseer/recommender.rs` — `last_reset_utc` on `RecommenderState`.
- `src/overseer/config.rs` — flip default for `enabled`.
- `src/overseer/observer.rs` — populate `tail_per_session` in `ContextSnapshot`.
- `migrations/0006_rec_archived.sql` — recreate `recommendations` table to
  loosen CHECK constraint and add `status_changed_at`.
- `ui/src/components/RecModal.tsx` — status segmented filter, re-explain UI.
- `ui/src/api.ts` — already has `explainRec`; no change.
- `main.rs:39` — call `archive_old(db, 7_days)` inside the existing tick.

---

## Per-task design

### Task 1 — Defaults default-on flip

`OverseerConfig::from_env` and `PlanExecutor::from_env` currently treat
absence of the env var as "off". Invert: absence means on. Set env var to
`0` / `false` / `off` to disable.

Behavior change:
- Existing users with the env var set keep their behavior.
- Existing users with the env var unset (the majority) get Overseer + Plan
  Executor enabled on next start.

Tests: existing `tests/overseer_config.rs` cases for the parser need their
"unset" expectation flipped from `None` to `Some(default)`. Add a kill-switch
case.

### Task 2 — 7-day rec auto-archive

**Schema change** (`migrations/0006_rec_archived.sql`):

```sql
-- SQLite can't ALTER a CHECK constraint, so recreate the table.
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

INSERT INTO recommendations_new
    SELECT id, project_id, kind, severity, title, body, rationale,
           evidence_json, status, created_at, deferred_until, acted_at,
           COALESCE(acted_at, created_at) AS status_changed_at, content_hash
    FROM recommendations;

DROP TABLE recommendations;
ALTER TABLE recommendations_new RENAME TO recommendations;

-- Preserves the existing list-query index (orders by created_at DESC).
CREATE INDEX recommendations_project_status_idx
    ON recommendations(project_id, status, created_at);

-- New index for the archive sweep predicate.
CREATE INDEX recommendations_status_changed_idx
    ON recommendations(status, status_changed_at);
```

**Code changes:**

- `RecStatus::Archived` variant + `as_str` arm.
- `set_status` and `defer` write `status_changed_at = now()` on every
  status transition.
- New function `recommendations::archive_old(db, age: Duration) -> Result<usize>`
  that runs:
  ```sql
  UPDATE recommendations SET status='archived', status_changed_at=?1
  WHERE status IN ('acted','dismissed') AND status_changed_at < ?2
  ```
  Returns row count for logging.

**Cron wiring:** `main.rs:39`'s 5-minute tick gets a second call:
`recommendations::archive_old(&db, Duration::days(7))`. No new spawn.

**API surface:** `GET /api/projects/:id/recommendations?status=archived` works
because the parser at `src/api/recommendations.rs:43` already does
`Some("archived") => RecStatus::Archived` (after the enum gains the variant).
No new route.

**Tests:**
- `tests/recommendations_db.rs`: add `archive_old_only_acted_dismissed` —
  insert one rec per status with `status_changed_at` set 8 days ago,
  call `archive_old`, assert only acted+dismissed move.
- Same test asserts deferred + pending are untouched.
- `tests/recommendations_db.rs`: add `archive_old_respects_threshold` —
  rec aged 6 days stays, rec aged 8 days moves.

### Task 3 — `ProjectDeleted` bus event

**`src/api/events.rs`**: add variant
```rust
ProjectDeleted { project_id: Uuid },
```
to `DomainEvent`. Update the `is_session_event` matcher at line 157 to
include it (returns `false` — this is a project-level event, not session).

**`src/session_manager.rs:692` (`delete_project`)**: after
`db.delete_project_cascade` succeeds, call
`bus.send(DomainEvent::ProjectDeleted { project_id }).ok()`.

**`src/overseer/mod.rs`**: replace the existing pre-loop comment workaround
(lines 19-28) and add a `ProjectDeleted` arm to the existing event match at
line 128 that calls `self.drop_project(project_id)`. Add `drop_project` to
`Supervisor` — removes the per-project Observer/Recommender entries from
its in-memory map.

**Tests:**
- `tests/overseer_supervisor.rs`: add `drops_project_on_deletion` — spawn
  Overseer, add a project, send `ProjectDeleted`, assert the per-project
  task exits (no more Observer activity for that project).

### Task 4 — `tail_per_session` ContextSnapshot field

**`src/overseer/observer.rs`** (`ContextSnapshot`): add field
```rust
pub tail_per_session: std::collections::HashMap<Uuid, String>,
```

**Population:** the Observer already iterates child sessions when building
the snapshot. For each, call the existing `PtyHandleShared::snapshot()`
(`src/pty.rs:78`) — this returns the rendered terminal screen as bytes,
naturally bounded by `rows × cols`. Decode via `String::from_utf8_lossy`
and store in the map.

No size knob needed — the screen emulator already bounds output to one
viewport. If a user has a 200×80 terminal, that's ~16 KB max per session.

**Kimi prompt (`src/overseer/recommender.rs`):** include
`tail_per_session` under a new `## Recent PTY tails` section in the prompt
when non-empty. Skip when the map is empty.

**Tests:**
- `tests/overseer_e2e_mocked.rs`: extend the existing fixture with two
  child sessions producing PTY output; assert the prompt sent to the mock
  Kimi includes both tails.

### Task 5 — Deferred-recs UI toggle

**`ui/src/components/RecModal.tsx`**: currently fetches with hardcoded
`status=pending`. Replace with:
- Local state `[statusFilter, setStatusFilter]` defaulting to `pending`.
- Segmented control above the list: `Pending | Deferred | Acted | Dismissed | Archived`.
- Refetch on filter change.
- Empty-state copy per filter ("No deferred recommendations" etc.).

**API client:** existing `listRecs(projectId, status)` already supports the
`status` query param. No change.

**Tests:**
- `ui/src/components/RecModal.test.tsx`: add `switches_to_deferred_filter`
  — render with mocked recs in two statuses, click `Deferred`, assert only
  deferred render and the API was called with `status=deferred`.

### Task 6 — UTC token-budget reset

**`src/overseer/recommender.rs`**: add field to `RecommenderState`:
```rust
pub last_reset_utc: chrono::NaiveDate,
```
Initialize in `new()` to `Utc::now().date_naive()`.

In `try_consume` (the function around line 139 that checks budget),
**before** the budget check:
```rust
let today = chrono::Utc::now().date_naive();
if today > state.last_reset_utc {
    state.tokens_used = 0;
    state.last_reset_utc = today;
}
```

No persistence. A restart resets the counter naturally; this only fixes
the long-running-process case.

**Tests:**
- `tests/overseer_review.rs`: add `budget_resets_at_utc_midnight` —
  manually set `last_reset_utc` to yesterday in a test-mode constructor,
  exhaust budget, then trigger `try_consume` after time progression; assert
  reset happens.

### Task 7 — `/recommendations/explain` UI button

**`ui/src/components/RecModal.tsx`**: per-rec footer gets a "Re-explain"
button. Click toggles an inline `<textarea>` for the user's "what's unclear"
note. Submit posts to existing `explainRec(projectId, recId, rationale)`.

When the response arrives, the rec's body in the local list state is
replaced with the new explanation. The `RecommendationActed{action:"explain"}`
event already fires — no other backend wiring needed.

**Tests:**
- `ui/src/components/RecModal.test.tsx`: add `re_explain_updates_body` —
  click Re-explain, type rationale, submit, assert mocked
  `explainRec` is called and the rec body re-renders.

---

## Data flow

No flow changes for tasks 1, 5, 6, 7 (local UI / config / per-call state).

**Task 2 (archive):** `cron tick → archive_old(db, 7d) → UPDATE rows`. The
existing GET endpoint at `src/api/recommendations.rs:43` already accepts
`?status=archived` once the enum gains the variant.

**Task 3 (`ProjectDeleted`):** `DELETE /api/projects/:id` →
`session_manager::delete_project` → `db.delete_project_cascade` →
`bus.send(ProjectDeleted)` → Overseer subscriber → `drop_project(uuid)`.
Replaces today's polling workaround at `src/overseer/mod.rs:19-28`.

**Task 4 (`tail_per_session`):** Observer's existing snapshot path now reads
PTY tails per child session and includes them in the snapshot. Same prompt
flow into Kimi as today, just with extra context.

---

## Error handling

- **Task 2**: `archive_old` returning a DB error logs `tracing::warn!` and
  is swallowed at the cron tick site. Same pattern as the existing
  `resurface_deferred` call.
- **Task 3**: `bus.send` failure on `ProjectDeleted` — no subscribers means
  the send returns `Err` silently; same pattern as `ProjectAdded` today.
- **Task 4**: PTY ring read failure for a single session does not abort the
  snapshot; that session is omitted from the map. Logged at debug.
- **Task 6**: Date comparison can't fail (chrono `NaiveDate` is total).
- **Task 7**: `explainRec` HTTP failure surfaces in RecModal as an inline
  error banner with retry. No new error type.

---

## Testing

Per-task unit/integration tests listed above. Plus one end-to-end:

- `tests/overseer_e2e_mocked.rs`: extend existing 5-rec smoke to assert
  (a) one rec gets archived after a simulated 8-day clock advance, and
  (b) a `ProjectDeleted` event terminates that project's per-project
  Overseer task.

UI tests via existing Vitest setup. No new test infrastructure.

---

## Out of scope

These items deliberately are NOT in Plan 9 (left for later or rejected as
unneeded):

- **Token-budget UI** — current users see "budget exhausted" via the existing
  `Budget` review error; no separate UI requested.
- **Archive UI surface beyond the filter** — no bulk-restore button, no
  "permanently delete" beyond eventual `ProjectDeleted` cascade.
- **`tail_per_session` HTTP endpoint** — confirmed internal-only.
- **Configurable archive threshold** — hard-coded 7 days. If users complain,
  Plan 10+ can add a config knob.
- **`PlanExecutor` defaults change** — already covered in Task 1; no separate
  task.
