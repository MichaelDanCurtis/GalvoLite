# Session Spawn / Fan-out / Rejoin — Design

**Status:** approved 2026-04-30. Supersedes the 2026-04-17 spawn/rejoin sketch (memory `project_spawn_rejoin.md`) which assumed the bracket-token signal pipeline retired in Sub-plan 5.

**Goal.** A running GalvoLite session can spawn child sessions to perform parallel or sequenced subtasks, idle-wait until those children reach terminal status, and resume on a single synthesized rejoin prompt that summarises each child's outcome. Each child runs in its own `git worktree` on its own branch.

**Non-goals (v1).** GitHub PR-per-child auto-creation; depends_on DAG between siblings; per-child timeout; auto-merge of child branches; custom branch names; runtime-configurable concurrency caps.

---

## 1. Architecture

Three pillars work together:

1. **MCP tools (model-driven, in-turn).** Two new MCP tools the model itself calls during a turn: `spawn_session(role, task)` and `wait_children([ids])`. `spawn_session` returns immediately with the new child's `{session_id, worktree_path, branch}`. `wait_children` records the wait list in the database and returns immediately. Parent's turn ends naturally after the model emits final text.

2. **Harness state machine (orchestrator side).** New `sessions.parent_session_id`, `sessions.waiting_on`, and `sessions.pending_rejoin` columns. SessionManager observes terminal events on every child (`status_report` with kind `complete`/`blocked`, or `system.session_ended`). When the last child a parent is waiting on becomes terminal, the harness builds the rejoin markdown payload, writes it to the parent's `pending_rejoin` column, and writes a short marker token to the parent's PTY.

3. **Hook-driven payload delivery.** The parent's `UserPromptSubmit` hook (the existing `galvolite-hook` binary from Sub-plan 2) recognises the marker pattern, calls `GET /api/sessions/:id/pending-rejoin`, returns the authoritative markdown payload as Claude Code's `additionalContext`, and suppresses the original marker text via `decision: "block"`. The endpoint atomically clears `pending_rejoin` on read.

**Why this shape.** The DB is the source of truth for the rejoin payload, so the design survives PTY scrollback drops, terminal resizes, mid-stream output races, and recovery restarts. The PTY write is a wake marker only; structured content flows over HTTP. The hook surface and per-session bearer auth from Sub-plans 2 and 3 are reused. The `feedback_idle_sessions_no_cost.md` rule is preserved — parent's turn ends after `wait_children`, no further bytes are sent until the wake fires.

**No DAG sequencing in v1.** Parent does multiple `wait_children` waves to sequence work: emit a scout, `wait_children([scout])`, parse result, then emit three implementations and `wait_children([impl1, impl2, impl3])`.

---

## 2. Data Model

Migration `0003_session_spawn.sql` (next free number after `0002_hooks_mcp_foundations.sql`):

```sql
-- parent_session_id and worktree_path already present from 0001_initial.sql.
ALTER TABLE sessions ADD COLUMN waiting_on      TEXT NULL;  -- JSON array of child session UUIDs, NULL when not waiting
ALTER TABLE sessions ADD COLUMN pending_rejoin  TEXT NULL;  -- markdown payload set by harness; cleared atomically by /pending-rejoin GET
ALTER TABLE sessions ADD COLUMN terminal_status TEXT NULL;  -- 'complete'|'blocked'|'error'|'ended'; set by on_child_terminal; NULL for live or never-terminal sessions
CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
```

No new tables. JSON-in-TEXT for `waiting_on` matches the existing `plan_markdown` and `events.payload` conventions. `path_claims` (reserved by 0002) is unused in v1; left for a future write-coordination feature.

**Column semantics:**

- `parent_session_id` (existing): set on insert when `spawn_session` mints the row. NULL for top-level sessions.
- `waiting_on`: JSON array `["uuid", ...]`. Set by `wait_children` MCP tool. Mutated atomically by `on_child_terminal` reactor (remove finished child id). Cleared to NULL when payload is built.
- `pending_rejoin`: markdown text. Written when `waiting_on` becomes empty. Cleared inside the same transaction as the API endpoint serving it.
- `terminal_status`: set by `on_child_terminal` from the triggering event. `complete` / `blocked` come from `mcp.status_report.kind`; `error` / `ended` come from `system.session_ended` reason. Used by the rejoin markdown builder and the worktree GC pass; orthogonal to `SessionState` (which never has a `complete` variant — `mcp.status_report(complete)` transitions state to `Idle`).

---

## 3. Components

### 3.1 New modules

- **`src/mcp/tools/spawn.rs`** — registers two MCP tools.
  - `spawn_session(role: String, task: String) -> { session_id, worktree_path, branch }`. Validates role is one of the four known roles. Calls `SessionManager::spawn_child`. Emits soft-cap warning toasts (see §5).
  - `wait_children(children: Vec<Uuid>) -> { ok: bool, immediate_rejoin: bool }`. Validates each id belongs to a session whose `parent_session_id == caller`. Short-circuits if all listed children already terminal — builds rejoin and writes marker before returning. Otherwise updates `waiting_on` and returns `{ok: true, immediate_rejoin: false}`.

- **`src/worktree.rs`** — git plumbing.
  - `add_worktree(repo_root: &Path, session_id: Uuid, base_ref: &str) -> Result<(PathBuf, String)>`. Runs `git -C <repo> worktree add <repo>/.galvolite/worktrees/<id>/ -b galvolite/<short> <base_ref>`. Returns `(worktree_path, branch_name)`. Branch collision retry: `<short>`, `<short>-2`, `<short>-3`, then error.
  - `remove_worktree(worktree_path: &Path) -> Result<()>`. Runs `git -C <parent> worktree remove --force <path>`.
  - `derive_branch_name(session_id: Uuid) -> String`: `galvolite/<first-8-of-uuid>`.

- **`src/api/spawn.rs`** — one HTTP handler.
  - `GET /api/sessions/:id/pending-rejoin` — bearer-auth via existing per-session token. Returns `{markdown: String}` and atomically clears `pending_rejoin` in the same SQL transaction (`UPDATE sessions SET pending_rejoin = NULL WHERE id = ? AND pending_rejoin IS NOT NULL RETURNING pending_rejoin`). 404 when nothing pending.

### 3.2 Extended modules

- **`src/session_manager.rs`** — four new methods.
  - `spawn_child(parent_id, role, task) -> Result<SessionRow>`: resolves parent's project + worktree base ref; calls `worktree::add_worktree`; inserts session row with `parent_session_id` set; spawns PTY in the child worktree (existing flow); applies `mcp.spawn_session` event on parent and `system.session_started` event on child.
  - `set_waiting_on(parent_id, child_ids) -> Result<()>`: validates each child is owned by parent, updates `waiting_on` column, applies `mcp.wait_children` event.
  - `build_rejoin_markdown(parent_id) -> Result<String>`: pure read of child rows, formats per §4.2.
  - `on_child_terminal(child_id, status: "complete"|"blocked"|"error"|"ended") -> Result<()>`: writes `terminal_status` on the child row; looks up parent observers (`SELECT id, waiting_on FROM sessions WHERE waiting_on LIKE '%<child-id>%'`) inside `BEGIN IMMEDIATE`; mutates JSON array; if empty, builds markdown, writes `pending_rejoin`, applies `session.children_resolved` event, queues PTY-write of marker token. The reactor is invoked from three call sites: `mcp.status_report` handler when `kind ∈ {complete, blocked}`; PTY exit path on `system.session_ended` (status from reason: clean exit → `ended`, crash → `error`); recovery startup pass.

- **`src/hooks/handlers/user_prompt_submit.rs`** — new branch.
  - Detects `^\[GALVOLITE:REJOIN ([0-9a-f-]{36})\]\s*$` in the submitted prompt text. On match: calls `GET /api/sessions/<id>/pending-rejoin` with the in-process bearer token; on 200 returns `{decision: "block", additionalContext: <markdown>}`; on 404 returns no additionalContext and lets the marker fall through (visible debug breadcrumb); on network error logs to stderr and falls through.

- **`src/claude_md.rs`** — extend the MCP-tool guide string for the orchestrator and brainstorm role profiles to mention `spawn_session` and `wait_children`. Implementation and scout profiles do not get them — those roles are leaves.

- **`src/state.rs`** — register new event kinds in the `apply_event` exhaustive match. New kinds:
  - `mcp.spawn_session` — logging only, no state mutation.
  - `mcp.wait_children` — logging only.
  - `session.children_resolved` — logging only; emits a DomainEvent variant for live UI update.

### 3.3 Frontend

- **`ui/src/types.ts`**: extend `Session` with `parent_session_id?: string` and `waiting_on?: string[]`. Add `DomainEvent` variant:

  ```ts
  | {
      type: "session.children_resolved";
      session_id: string;
      children: Array<{ id: string; role: string; status: string; handoff: string | null }>;
    }
  ```

- **`ui/src/components/Dashboard.tsx`**: render sessions as a nested tree using `parent_session_id`. Children indented under parent. Computes the tree once per `sessions[]` update via `useMemo`.

- **`ui/src/components/SessionRow.tsx`**: new optional `depth?: number` prop drives a left padding step. New "waiting on N" badge when `session.waiting_on?.length > 0`.

- **`ui/src/hooks/useSessions.ts`**: handle `session.children_resolved` by clearing `waiting_on` on the parent row.

---

## 4. Data Flow

### 4.1 Happy path

```
T0  parent.spawn_session(role="implementation", task="Auth refactor")
       MCP → SessionManager::spawn_child(parent_id, ...)
         → worktree::add_worktree(...)  → ".../child1/", "galvolite/abc1"
         → db.insert_session(NewSession{ parent_session_id: Some(parent_id), worktree_path, branch, role, ... })
         → soft-cap warn (project active count, depth) — see §5
         → spawn PTY (existing flow)
         → apply_event_with_bus(mcp.spawn_session, parent_id, payload={child_id, role})
         → apply_event_with_bus(system.session_started, child1)
       MCP returns { session_id: <child1>, worktree_path: ".../child1/", branch: "galvolite/abc1" }

T1  parent.spawn_session(...) → child2 (same flow)

T2  parent.wait_children([child1, child2])
       MCP → SessionManager::set_waiting_on(parent_id, [child1, child2])
         → UPDATE sessions SET waiting_on = '["<child1>","<child2>"]' WHERE id = parent_id
         → apply_event_with_bus(mcp.wait_children, parent_id, payload={children})
       MCP returns { ok: true, immediate_rejoin: false }

T3  parent's turn ends (model emits final text). Harness checks waiting_on; non-empty → no new prompt sent. PTY parked. Zero context cost.

T4  child1.status_report({kind: "complete"})  [+ write_handoff prior]
       Existing MCP path → apply_event_with_bus(mcp.status_report, child1, ...)
       After commit: SessionManager::on_child_terminal(child1):
         BEGIN IMMEDIATE
           SELECT id, waiting_on FROM sessions WHERE waiting_on LIKE '%<child1>%'
           For each parent: remove child1 from JSON; UPDATE waiting_on
           If new waiting_on non-empty: COMMIT; done.
         COMMIT

T5  child2.status_report({kind: "complete"})
       on_child_terminal(child2): parent's waiting_on now empty.
       BEGIN IMMEDIATE
         build_rejoin_markdown(parent_id) reads child rows: role, terminal status, handoff_path
         UPDATE sessions SET pending_rejoin = <markdown>, waiting_on = NULL WHERE id = parent_id
       COMMIT
       apply_event_with_bus(session.children_resolved, parent_id, payload={children: [...]})
       PTY-write to parent: b"[GALVOLITE:REJOIN <parent-id>]\n"
       Harness resumes normal prompt cycle for parent.

T6  Claude Code on parent processes the marker as a user prompt.
       galvolite-hook UserPromptSubmit fires.
         pattern match → GET /api/sessions/<id>/pending-rejoin
           Authorization: Bearer <session-token>
         server: SELECT pending_rejoin; UPDATE pending_rejoin = NULL (atomic); return {markdown}
         hook outputs JSON: {decision: "block", additionalContext: <markdown>}
       Claude Code suppresses the marker, prepends additionalContext.
       Parent's next model turn reads the rejoin payload and resumes.
```

### 4.2 Rejoin markdown payload (Q10-B format)

```
Children resolved. Resume.

- {child_id_short} role={role} status={complete|blocked|error} handoff={path-or-none}
- {child_id_short} role={role} status={complete|blocked|error} handoff={path-or-none}
```

Built by `SessionManager::build_rejoin_markdown(parent_id)` from a single `SELECT id, role, terminal_status, handoff_path FROM sessions WHERE parent_session_id = ? AND id IN (...)` query.

### 4.3 Recovery on restart

Sub-plan 5's `recover_non_terminal_sessions` runs at startup. Extension:

- For every session row with `waiting_on IS NOT NULL`: re-run `on_child_terminal(child_id)` for each child id in the list whose terminal event already exists in the events log. May immediately produce a rejoin write.
- For every session with `pending_rejoin IS NOT NULL` AND alive PTY: re-write the marker token to PTY. The hook re-reads the payload from the DB.
- For every orphaned session with `parent_session_id`: nothing special. The orphan flow already records `had_active_children` in the parent's auto-handoff.

---

## 5. Soft Caps

Per Q7-B: warnings, not rejection.

- **Per-project active concurrency** — default threshold 4. On every `spawn_session`, after insert, count `sessions WHERE project_id = ? AND ended_at IS NULL`. If `> 4`, publish `operator.toast{level: warn, text: "Project <name> exceeds 4 concurrent sessions (now <N>)"}`. Spawn proceeds.
- **Spawn depth** — default threshold 3. Walk the `parent_session_id` chain for the new child; if depth > 3, publish `operator.toast{level: warn, text: "Spawn depth <D> exceeds recommended max 3"}`. Spawn proceeds.

Both thresholds are hardcoded constants in `src/mcp/tools/spawn.rs` for v1 (cohesive with the tool that emits the toasts). No DB column. UI's `ToastStack` already renders warn-level (orange, 8s) per Sub-plan 4 Task 17.

---

## 6. Error Handling

| Failure | Handling |
|---|---|
| `git worktree add` fails (locked branch, dirty tree, disk full) | `spawn_session` MCP returns error before DB row insert. No half-state. |
| Branch name collision | `worktree::add_worktree` retries `<short>-2`, `<short>-3`, then errors. |
| Claude binary missing on child spawn | Existing orphan flow (Sub-plan 5) fires; child has `parent_session_id` set; `on_child_terminal` triggers immediately for parent's wait list. |
| Hook gets 404 from `/pending-rejoin` | Hook returns no `additionalContext`. Marker text falls through. Visible debug breadcrumb. |
| Hook can't reach GalvoLite (port closed during restart) | Hook logs to stderr (visible to operator), returns no `additionalContext`. Same fall-through. |
| `wait_children` listed children already terminal | Tool short-circuits: builds rejoin markdown, writes `pending_rejoin`, writes marker, returns `{ok: true, immediate_rejoin: true}`. |
| `wait_children` with unknown child IDs (not parent's children, or non-existent) | Tool returns 422 in MCP error shape: `{error: "unknown_child", id: ...}`. Parent must handle. |
| Concurrent terminal events on multiple children of one parent | `on_child_terminal` runs inside `BEGIN IMMEDIATE`. Read-modify-write of `waiting_on` is serialized. |
| Parent's PTY dies mid-wait | Per Q9-A: children continue independently. Existing orphan flow writes auto-handoff for parent listing `had_active_children: [ids]`. Children's `on_child_terminal` for the dead parent row writes `pending_rejoin` for inspection but not the marker (no live PTY to write to). |
| Operator kills parent | Existing kill cascade walks `parent_session_id` chain and `kill_session`s every descendant. Worktrees preserved per §7 because kill = error terminal. |

---

## 7. Worktree Lifecycle (Q4 + Q5)

- **Location:** `<project>/.galvolite/worktrees/<session-id>/`. `.galvolite/` is added to project `.gitignore` if not already present (handled by existing project-add flow; verify on plan).
- **Base ref:** parent's current `HEAD` (resolved at `spawn_session` time). Child inherits parent's WIP.
- **Branch:** `galvolite/<short-uuid>` on first attempt; collision retry up to 3 attempts.
- **Cleanup on terminal:**
  - `complete` → auto-`worktree remove --force` is **deferred** until the parent's `pending_rejoin` is consumed (the `/api/sessions/:id/pending-rejoin` GET, which is the moment the parent reads the handoff path). Reason: handoff files live inside the worktree, and the parent must be able to open them after the rejoin lands. Implementation: no extra column needed — after `/pending-rejoin` returns the markdown, the handler runs `SELECT id FROM sessions WHERE parent_session_id = ? AND terminal_status = 'complete'`, calls `worktree::remove_worktree` for each, and is idempotent (already-removed paths are no-ops). On recovery the same query catches any worktrees orphaned by a crash.
  - `blocked` / `error` / killed → preserve worktree dir for operator inspection. `worktree remove` deferred to a future "Delete session" dashboard action.
- **Branch:** never auto-deleted. Commits are work product; operator merges or deletes manually.
- **Project delete cascade:** existing cascade extends to call `worktree::remove_worktree` for every session being deleted in that project.
- **Parent kill cascade:** descendants enter `error` terminal state → worktrees preserved per the same rule.

---

## 8. UI

- Dashboard `Dashboard.tsx` renders the session list as a nested tree built from `parent_session_id`. Computed once per render via `useMemo`. Top-level rows render as before; children appear indented.
- `SessionRow.tsx` accepts a new optional `depth?: number` prop driving left padding (Tailwind `pl-{depth*4}`). When `session.waiting_on?.length > 0`, renders an inline "waiting on N children" badge next to the role badge.
- `useSessions` adds a reducer arm for `session.children_resolved` that clears `waiting_on` on the parent row in-place. The new event also fires `meta_changed`-like updates so `pending_rejoin` etc. propagate live.
- No tab changes inside `ExpandedSessionView`. Parent/child relationships are a list-level concern.

---

## 9. Testing Strategy

### Rust unit

- `worktree::add_worktree` / `remove_worktree` against a real git tempdir
- `worktree::derive_branch_name` + collision retry
- `build_rejoin_markdown` golden output for 1/2/3 children with mixed statuses
- Hook marker pattern regex + payload shape

### Rust integration (`tests/`)

- `spawn_session_creates_worktree_and_db_row`
- `wait_children_parks_parent` — assert no prompt sent until terminal
- `last_child_terminal_writes_marker_and_pending_rejoin`
- `pending_rejoin_endpoint_atomic_clear` — two concurrent GETs; only one returns markdown
- `wait_children_short_circuits_on_already_terminal`
- `parent_kill_cascades_to_descendants` — 3-deep tree
- `recovery_restores_waiting_state` — restart mid-wait; harness re-fires marker
- `concurrent_child_terminals_serialize`
- `worktree_preserved_on_blocked_removed_on_complete`

### Hook handler (Rust)

- Pattern match positive/negative cases
- HTTP 200 → `additionalContext` returned with `decision: "block"`
- HTTP 404 → no `additionalContext`, fall through
- Network error → log + fall through

### E2E

- `tests/spawn_e2e.rs` style mirror of `tests/dashboard_smoke_e2e.rs` — `bash` mock-claude as parent + 2 mock-claude children running `cat` of fixture files; assert nested events flow through WS, `/pending-rejoin` returns markdown, marker delivered to parent's PTY.

### Frontend (Vitest)

- Nested rendering with synthetic `parent_session_id`
- "Waiting on N" badge logic
- `useSessions` reducer arm for `session.children_resolved`

### Regression

- `tests/spawn_marker_recognized_by_hook.rs` — confirms the marker string round-trips through the hook correctly. Sibling of Sub-plan 5's `tests/real_claude_tui_fixture.rs`.

---

## 10. Open Questions / Deferred

These are intentionally out of v1. Listed so future plans can pick them up.

- **GitHub PR-per-child.** Auto `gh pr create` on child `complete`. Fits later GitHub-native scope (memory `project_github_scope.md`).
- **`depends_on` DAG.** Sibling sequencing inside the harness. Workaround in v1: parent does multiple `wait_children` waves.
- **Per-child timeout.** v1: no auto-timeout; operator kills via dashboard.
- **Custom branch names.** v1: auto-derived; operator can rename via `git`.
- **"Resend rejoin" dashboard action.** For when the hook fall-through fires due to network error.
- **Soft-cap thresholds in DB.** v1 hardcodes 4/3; promote to `projects.max_concurrent_sessions` if needed.
- **Auto-merge of child branches.** Future once GitHub-native scope lands.

---

## 11. Locked Decisions Summary (Q1-Q10)

For traceability:

| # | Topic | Decision |
|---|---|---|
| 1 | Rejoin wake | Hook-driven: PTY marker → UserPromptSubmit hook → /api/sessions/:id/pending-rejoin → additionalContext |
| 2 | Wait declaration | Dedicated `wait_children` MCP tool |
| 3 | Child-done semantic | Any terminal status (complete / blocked / system.session_ended) |
| 4 | Worktree location | `<project>/.galvolite/worktrees/<id>/`, branched from parent HEAD |
| 5 | Worktree cleanup | Auto-remove on `complete`; preserve on blocked/error/killed; never auto-delete branches |
| 6 | `spawn_session` signature | Minimal: `(role, task)` only. No depends_on, no custom branch, claude_bin inherited |
| 7 | Caps | Soft (warning toasts), defaults 4 concurrent / depth 3, hardcoded |
| 8 | Dashboard | Nested rows, "waiting on N" badge |
| 9 | Parent orphan with active children | Children continue; auto-handoff records had_active_children |
| 10 | Rejoin format | Markdown bullets per child |
