# Plan 8 — Plan Executor + Overseer

**Date:** 2026-05-02
**Author:** Michael Curtis (with Claude)
**Status:** Design approved, ready for implementation planning
**Builds on:** Plans 1a–7 (continuity, fan-out, role profiles, directive contract, postmortems, decisions)

---

## 1. What This Plan Adds

Two intertwined subsystems shipping together:

1. **Plan Executor** — closes original GalvoLite PRD §2 promise: "automated multi-session spawning from that plan." Operator clicks a single "Execute plan" button; the server reads `.galvolite/plan.md` + `.galvolite/directive.md`, derives a spawn graph (one part per checkbox step, phases per directive), and spawns one impl-role child per part in dependency order with worktree, `plan_part` injection, and Plan 7 prior-context hydration. Phase autonomy tiers (`full` / `review` / `manual`) gate advancement.

2. **Overseer** — long-lived per-project background process that watches the executor (and any other running sessions) using **Kimi K2** as a deliberately different model from the executor's Claude. Reads bus events, plan + directive, recent postmortems + decisions, sliding PTY tail per active session, and per-worktree git state. Calls Kimi on debounced triggers, emits human-readable recommendations into a `recommendations` table. Dashboard surfaces a top-bar bell button → modal with rec list + per-rec Make / Defer / Better-explanation actions, with multi-select support. "Make" routes through the orchestrator session via a new `pending_overseer_recs` field that mirrors the existing `pending_rejoin` lane.

Why both together: the Plan Executor produces the canvas the Overseer watches. Shipping in isolation either leaves Overseer reviewing only manual fan-out (limited demo value) or leaves the executor unmonitored (no cross-perspective check that decomposition was right). The full feature is "click execute, walk to your desk, see Overseer recs, dispatch them."

## 2. Scope

### In Plan 8

- `POST /api/projects/:id/execute-plan` endpoint + dashboard button.
- `src/plan_executor.rs` — pure spawn-graph derivation from plan.md + directive.md.
- Phase autonomy enforcement: `full` (auto-advance on rejoin), `review` (block on operator approval), `manual` (operator dispatches each phase).
- `OverseerSupervisor` + per-project `OverseerTask` background runtime.
- Kimi K2 HTTP adapter (OpenAI-compat) with env-config endpoint + key.
- `recommendations` DB table + REST CRUD.
- `sessions.pending_overseer_recs` column mirroring `pending_rejoin` semantics.
- Dashboard rec-bell + rec-modal (multi-select, severity, Make/Defer/Explain).
- Orchestrator profile updated with rec-routing rule (`>3 recs → fan out via spawn_child`).
- Feature flags: `GALVOLITE_OVERSEER` and `GALVOLITE_PLAN_EXECUTOR` (default off during stabilization).
- Integration + UI tests covering happy paths and documented edge cases.

### Explicitly deferred

- Cross-project Overseer (one global watcher seeing all projects) — Plan 8 is per-project.
- Overseer self-retraction of stale recs (auto-staleness was rejected in favor of explicit human action).
- Overseer with file-edit / spawn arms (Plan 8 keeps Overseer purely advisory; orchestrator dispatches).
- Adapter for non-Kimi models (Anthropic / OpenAI / local) — single-model in Plan 8.
- Dashboard support for *non-Claude* executor sessions (multi-CLI direction stays out of Plan 8).
- GitHub PR pipeline / Copilot review surface (still pending in PRD; separate plan).

### Out of scope (possibly ever)

- Overseer-driven autonomous code changes without operator click.
- Per-rec voice / SMS / mobile-push delivery.

## 3. Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        PROJECT (per-project)                        │
│                                                                     │
│  ┌─────────────┐   ┌──────────────┐   ┌──────────────────────────┐  │
│  │ plan.md +   │──▶│ PlanExecutor │──▶│ SessionManager.spawn_*   │  │
│  │ directive.md│   │   (new)      │   │   (existing)             │  │
│  └─────────────┘   └──────────────┘   └──────────────────────────┘  │
│         ▲                                       │                   │
│         │ "Execute plan" button                 │                   │
│         │                                       ▼                   │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │               EventBus (existing, broadened)                │    │
│  └─────────────────────────────────────────────────────────────┘    │
│         │                                       │                   │
│         ▼                                       ▼                   │
│  ┌──────────────┐                  ┌────────────────────────────┐   │
│  │  Overseer    │                  │  pending_overseer_recs     │   │
│  │  (new bg     │─writes recs─────▶│  (sessions row column,     │   │
│  │   process)   │                  │   mirrors pending_rejoin)  │   │
│  │              │                  └────────────────────────────┘   │
│  │  • bus sub   │                              │                    │
│  │  • PTY tail  │                              ▼                    │
│  │  • git state │                  ┌─────────────────────────────┐  │
│  │  • Kimi API  │                  │  Orchestrator session       │  │
│  └──────────────┘                  │  (additionalContext hook    │  │
│         │                          │   fetches + clears)         │  │
│         ▼                          └─────────────────────────────┘  │
│  ┌──────────────────────────┐                                       │
│  │ recommendations table    │ ◀── Dashboard rec-modal reads         │
│  └──────────────────────────┘                                       │
└─────────────────────────────────────────────────────────────────────┘
```

### Module map

| Module | Purpose |
|--------|---------|
| `src/plan_executor.rs` | Pure logic: parse plan + directive, derive spawn graph, walk phases. Calls into `SessionManager`. |
| `src/api/execute_plan.rs` | `POST /api/projects/:id/execute-plan` endpoint. |
| `src/overseer/mod.rs` | `OverseerSupervisor` — owns one task per active project; lifecycle on project create/delete. |
| `src/overseer/observer.rs` | Bus subscriber + rolling `ContextSnapshot { events, tail_per_session, git_state, plan, directive, postmortems, decisions }`. |
| `src/overseer/kimi.rs` | Kimi K2 HTTP client (OpenAI-compat). Env: `KIMI_API_KEY`, `KIMI_BASE_URL`. |
| `src/overseer/recommender.rs` | Debounced trigger: gather snapshot, prompt Kimi, parse rec JSON, persist. |
| `src/api/recommendations.rs` | REST: `GET /api/projects/:id/recommendations`, `POST .../act`, `POST .../defer`, `POST .../explain`. |
| `ui/src/components/RecBell.tsx` | Top-bar bell with count badge + degraded-state styling. |
| `ui/src/components/RecModal.tsx` | Modal with rec list, severity badges, multi-select, action buttons. |
| `ui/src/components/ExecutePlanButton.tsx` | Replaces / sits beside SpawnControls; same gate logic (plan + directive present). |

### DB schema additions

```sql
CREATE TABLE recommendations (
    id              TEXT PRIMARY KEY,         -- uuid
    project_id      TEXT NOT NULL,
    kind            TEXT NOT NULL,            -- e.g. "stuck_session", "plan_drift", "missing_test"
    severity        TEXT NOT NULL CHECK (severity IN ('info','warn','urgent')),
    title           TEXT NOT NULL,
    body            TEXT NOT NULL,            -- markdown
    rationale       TEXT NOT NULL,            -- markdown; expanded on "Better explanation"
    evidence_json   TEXT NOT NULL,            -- JSON refs: signal IDs, postmortem paths, sids
    status          TEXT NOT NULL CHECK (status IN ('pending','acted','dismissed','deferred')),
    created_at      TEXT NOT NULL,
    deferred_until  TEXT NULL,
    acted_at        TEXT NULL,
    content_hash    TEXT NOT NULL UNIQUE      -- dedupe on identical rec body+title+kind
);
CREATE INDEX recommendations_project_status_idx ON recommendations(project_id, status, created_at);

ALTER TABLE sessions ADD COLUMN pending_overseer_recs TEXT NULL;
```

### New bus events

- `RecommendationCreated { rec_id, project_id, severity }`
- `RecommendationActed { rec_id, action: "make"|"defer"|"dismiss"|"explain", route: Option<sid> }`
- `OverseerDegraded { project_id, reason }`
- `OverseerRecovered { project_id }`
- `PhaseReviewNeeded { project_id, phase_id }` (Plan Executor)
- `PhaseAdvanced { project_id, phase_id }` (Plan Executor)

### Boundaries (load-bearing invariants)

- **Overseer is read-only on the bus + write-only to `recommendations` and `sessions.pending_overseer_recs`.** Never edits files. Never spawns sessions. Never writes plan / directive / postmortems / decisions.
- **Plan Executor is stateless logic + REST handler.** Does not subscribe to the bus; reacts to operator click. State lives in the spawn graph it produces; once spawned, ordinary fan-out / rejoin machinery owns it.
- **Orchestrator remains router-only (Plan 7 invariant).** Overseer recs are advisory inputs, equivalent to operator messages. Routing rule (`>3 → spawn_child`) preserves "orchestrator does not decide *what* to do, it routes *who* does it."

## 4. Data Flow

### Flow A — Plan execution

1. Operator clicks "Execute plan" in dashboard.
2. UI → `POST /api/projects/:id/execute-plan`.
3. Server reads `<project>/.galvolite/plan.md` + `directive.md`. Parses steps via `plan_steps::parse_plan_markdown`. Parses phases via `directive::parse`.
4. Validates: every part referenced in directive resolves to a plan step ordinal; no cyclic phase deps; autonomy values valid; orchestrator session exists for this project.
5. For each phase in topological order:
   a. For each part in the phase: `mgr.spawn_child(orch.id, "implementation", &step.title, Some(part_num))`. Existing prior-context injection (Plan 7) fires per child.
   b. `set_waiting_on(orch.id, &child_ids)` — orchestrator naturally waits via existing rejoin pattern.
   c. Phase autonomy `full` → on rejoin, executor enqueues next phase automatically (writes `[NEXT_PHASE p_id]` to orch's `pending_rejoin`).
   d. Phase autonomy `review` → emit `PhaseReviewNeeded`; UI shows "Approve phase N" button. Operator click → executor advances.
   e. Phase autonomy `manual` → executor stops at phase boundary; operator must click "Spawn next phase" each time.
6. Each spawn emits existing `SessionSpawned` event; dashboard fan-out tree updates live.

### Flow B — Overseer continuous loop

1. On project create / app start: `OverseerSupervisor` spawns one `OverseerTask` per project where `recommendations_enabled=1`.
2. Task subscribes to project's filtered bus + opens Kimi HTTP client.
3. Maintains in-memory `ContextSnapshot { events: ring(200), tail_per_session: ring(50 lines), git_state: HashMap<sid, GitDelta>, plan_md, directive_md, recent_postmortems(5), recent_decisions(10) }`.
4. Trigger conditions (debounced 30s after last event):
   - Any signal event (`STATUS:STUCK`, `STATUS:COMPLETE`, `ATTENTION:*`, `HANDOFF:*`).
   - `PostmortemWritten` / `DecisionRecorded` / `PlanChanged` / `DirectiveChanged`.
   - Periodic floor: every 10 min even if no events (proactive review).
5. Trigger fires → `recommender::review(snapshot)` → packs prompt → Kimi K2 API call → expects JSON `{recommendations: [{kind, severity, title, body, rationale, evidence_refs}]}`.
6. Each rec deduped via `content_hash = sha256(kind || title || body)`; new ones inserted as `pending`. Bus emits `RecommendationCreated`. Dashboard bell increments via WS.
7. Auto-archive sweep every 1h: `pending` recs older than 7 days → `dismissed`.
8. Rate cap: max 1 Kimi call per project per 30s; daily token budget per project (env-configurable, default 100k tokens).

### Flow C — Rec action

1. Operator opens rec-modal, multi-selects N recs, clicks "Make."
2. UI → `POST /api/projects/:id/recommendations/act { rec_ids: [...], action: "make" }`.
3. Server marks each rec `acted_at=now, status=acted`. Concatenates rendered bodies into a single block:
   ```
   ## Apply these N recommendations:
   ### Rec 1: <title>
   <body>
   _Rationale: <rationale>_
   ---
   ### Rec 2: ...
   ```
4. Atomically appends block to `sessions.pending_overseer_recs` for project's orchestrator session (existing block, if any, gets the new block appended after `\n\n---\n\n`). Append-only — never silently overwrite.
5. Emits `RecommendationActed { rec_ids, action: "make" }`.
6. Orchestrator session's `additionalContext` hook on next turn fetches + clears `pending_overseer_recs`. Orchestrator profile (updated) applies routing rule: rec count ≤3 → handle inline; >3 → fan out via `spawn_child` per rec.
7. Defer: server sets `status=deferred, deferred_until=now+24h` (default; operator can override). Cron sweep promotes back to `pending` when `deferred_until` passes.
8. Better explanation: server sets a hint on the next Overseer trigger (`?explain=rec_id`); Kimi call returns expanded `rationale` field; rec stays `pending` with new rationale stored. Bus emits `RecommendationActed { action: "explain" }`.

## 5. Error Handling & Edge Cases

### Plan Executor

- Missing `plan.md` or `directive.md` → 400, error surfaced in dashboard.
- Cyclic phase deps → 400 with cycle node IDs.
- Part referenced in directive but absent from plan.md → 400 listing the orphan parts.
- Spawn failure mid-phase → mark partial children as `orphaned` (existing path), abort executor for the run, surface in dashboard. Operator can retry.
- Worktree branch collision → existing `spawn_child` already handles via unique branch naming.
- "Execute plan" called twice for same project → idempotency: server checks no children currently in `running` state for this project's orchestrator before spawning. Otherwise 409.

### Overseer

- Kimi API down / timeout → log via `tracing::warn`, skip the trigger, bus continues. No retries on transient errors; next bus event will trigger. 5 consecutive failures → emit `OverseerDegraded`; dashboard shows yellow status on bell icon. Recovery on next success → `OverseerRecovered`.
- Kimi malformed JSON response → log, skip rec emission for that call. Don't crash task.
- Daily token budget exceeded → mark Overseer suspended for project; dashboard shows red bell with tooltip "budget exhausted, raise limit or wait for daily reset (UTC midnight)."
- Bus event lag (channel backpressure) → tracing::warn; recover via next snapshot. Recommendations are advisory; dropped events won't crash anything.
- PTY tail buffer overflow → ring drops oldest. Snapshot always captures last N lines, never blocks.
- Git state read fails (worktree GC'd mid-read) → field defaults to `unknown`; recommendation may be less precise but doesn't crash.
- Project deleted while Overseer running → supervisor drops the task on `ProjectDeleted` event.

### Rec routing (Overseer → Orchestrator)

- Operator clicks "Make" but project has no orchestrator session → 409, modal shows "spawn an orchestrator first."
- Operator clicks "Make" while orchestrator is mid-turn → idempotent **append** to `pending_overseer_recs` (concat with `\n\n---\n\n`). Avoids losing prior unfetched recs.
- Orchestrator fails to consume (hook 500) → recs stay in `pending_overseer_recs`; existing pending_rejoin recovery semantics apply.
- "Defer" with no `deferred_until` value supplied → default 24h.
- Multi-rec batch where some recs already acted (race) → server filters to only `pending` recs in the request; returns `200` with body `{results: [{rec_id, outcome}]}` listing per-rec outcomes.

### Security

- `KIMI_API_KEY` from env; never logged. Existing tracing-secret-scrubber covers it.
- Rec body rendered as markdown → plain text in modal (no inline HTML execution). Reuse whichever markdown renderer the dashboard already uses; verify the chosen renderer escapes raw HTML by default.
- All `recommendations` queries scoped by `project_id`; no global rec endpoint.
- "Better explanation" requires session API token — gated through existing auth middleware.
- Rec content does NOT pass user-controlled markdown directly into orchestrator profile rendering — it lands in `pending_overseer_recs` as opaque text, fetched by hook, presented to orchestrator session as additionalContext (which already handles untrusted text).

## 6. Testing Strategy

### Pure-function tests (fast, deterministic)
- `plan_executor::derive_spawn_graph(plan_md, directive_md)` — happy path, missing parts, cyclic deps, autonomy parsing, single-phase, 3-phase.
- `overseer::recommender::pack_prompt(snapshot)` — context budget cap, redaction of API keys, deterministic ordering.
- `overseer::recommender::parse_kimi_response(json)` — well-formed, malformed, empty list.
- `recommendations::concat_for_injection(recs)` — multi-rec batching produces documented header + separator.

### Integration tests (in-process axum + SessionManager)
- `tests/execute_plan_e2e.rs` — write plan.md + directive.md, POST execute-plan, assert N children spawned with correct `plan_part`; autonomy=`full` advances on rejoin; autonomy=`review` blocks until approval endpoint hit.
- `tests/execute_plan_validation.rs` — orphan part rejected; cycle rejected; missing artifacts rejected; double-execute rejected.
- `tests/recommendations_lifecycle.rs` — insert pending; act → status acted, pending_overseer_recs populated, RecommendationActed emitted; defer → status deferred + deferred_until set; defer expires → resurfaces; auto-archive 7d sweep.
- `tests/recommendations_act_routing.rs` — single rec → injects; 5 recs batched → single concat block; orchestrator `additionalContext` GET fetches + clears.
- `tests/overseer_observer.rs` — fake bus: feed events → snapshot reflects them; PTY tail rings; git state captured.
- `tests/overseer_kimi_mocked.rs` — Kimi client points at mock HTTP server returning canned JSON; verify rec inserted, dedupe on identical content_hash, malformed response logged + skipped.
- `tests/overseer_degraded.rs` — 5 consecutive Kimi failures → `OverseerDegraded` emitted; recovery emits `OverseerRecovered`.

### UI tests (vitest)
- `RecModal.test.tsx` — renders pending list, severity badges, multi-select, Make/Defer/Explain buttons fire correct API calls, empty state, deferred-recs hidden by default toggle.
- `RecBell.test.tsx` — count badge updates on `RecommendationCreated` WS event; degraded state styling.
- `ExecutePlanButton.test.tsx` — disabled when plan/directive missing (reuses SpawnControls gate logic), enabled + fires POST when both present.

### Smoke (manual)
- On real project with `KIMI_API_KEY` set: write plan + directive, click Execute, watch fan-out tree, watch Overseer drop a rec, multi-select 3, click Make, observe orchestrator wake with injected block.

### Coverage targets
- 90%+ on `plan_executor` and `overseer::recommender` (pure logic).
- All REST endpoints have at least one happy-path + one error-path test.

## 7. Decisions Locked During Brainstorm (2026-05-02)

| # | Decision | Choice |
|---|----------|--------|
| 1 | Direction (vs GitHub PR / multi-CLI / hardening) | Plan executor + Overseer (E + 1 hybrid) |
| 2 | Overseer trigger model | Continuous (A) — long-lived bg task |
| 3 | Overseer's powers | Read-only / advisory (A) — orchestrator dispatches |
| 4 | Overseer scope | Per-project (A) |
| 5 | Plan 8 packaging | Both ship together (A) |
| 6 | Overseer observation surface | Maximum (D) — bus + artifacts + PTY tail + git |
| 7 | Overseer → orchestrator dispatch | Pending-injection lane (B) — mirror pending_rejoin |
| 8 | Recommendation lifecycle | Pending/acted/dismissed/deferred (A) — no Overseer self-retraction |
| 9 | Multi-rec batch action | Hybrid (C) — orchestrator profile rule: >3 fan out via spawn_child |
| Model | Overseer LLM | Kimi K2 (cheap, different perspective vs Claude executor) |
