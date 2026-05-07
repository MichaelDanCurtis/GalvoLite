# Plan 9 Brainstorm — Handoff

**Date:** 2026-05-06
**From:** Claude session `ee27c18e-31be-4fe9-b1f0-d012a87ab679` (resilient-snuggling-sonnet)
**To:** Whichever editor session picks this up next
**Status:** Mid-brainstorm. Awaiting user pick of Plan 9 scope.

---

## Where We Are

User invoked `superpowers:brainstorming` skill with arg `Plan 9 for GalvoLite`.
Skill checklist progress:

- [x] 1. Explore project context (memory + git log + plans/specs dirs read)
- [ ] 2. Visual companion — not yet offered (current question is conceptual; no visual needed yet)
- [ ] 3. Clarifying questions — **first question pending answer**
- [ ] 4. Propose 2–3 approaches
- [ ] 5. Present design sections
- [ ] 6. Write design doc → `docs/superpowers/specs/2026-05-06-plan-9-<topic>-design.md`
- [ ] 7. Spec self-review
- [ ] 8. User reviews spec
- [ ] 9. Invoke `superpowers:writing-plans`

**Pending question I asked the user:**

> What's Plan 9 aimed at?
>
> 1. **Multi-CLI adapters** — Gemini CLI + Copilot CLI as agent backends (per
>    `project_multi_cli_direction.md` memory)
> 2. **GitHub-native layer** — issues/PRs/comments as session sources (PRD goal,
>    zero code today)
> 3. **Plan 8 polish bundle** — defaults default-on, archive sweeps,
>    ProjectDeleted, tail_per_session, deferred-recs UI
> 4. **Something else** — user describes

User has NOT yet picked. Resume by re-asking or letting them pick.

---

## Codebase State

**Branch:** `main` (clean working tree as of compact)
**Latest commits (newest first):**

```
bb42a46 fix(overseer): bump Kimi HTTP timeout from 30s to 120s
3bcc6ec feat(overseer): default to Kimi Code endpoint + report as Kimi CLI
d705564 feat(overseer): make Kimi model env-driven via KIMI_MODEL
910e06b feat(overseer): auto-pickup new projects via ProjectAdded bus event
55c0530 fix(overseer): log review errors instead of dropping them
bffe629 fix(ui): refresh sessions on parent's mcp.spawn_session event
5c30a7d test(overseer): e2e mock Kimi → rec → make → orch append
```

**Plans 1a–8 ALL shipped** (see `docs/superpowers/plans/` listing below).
Plan 8 (Plan Executor + Overseer) live-smoke-confirmed 2026-05-05/06:
- Kimi Code namespace working (`api.kimi.com/coding/v1` + `kimi-for-coding`)
- 120s timeout sufficient for review-sized prompts with reasoning_content
- 5 recommendations generated from 10s smoke session — see
  `project_kimi_code_namespace.md` memory

**Existing plans dir:**

```
2026-04-15-plan-1a-ritual-engine.md
2026-04-15-plan-1b-tauri-dashboard.md
2026-04-17-plan-2-dashboard-crud-parity.md
2026-04-17-plan-3-fixes-and-roles.md
2026-04-18-foundations-hooks-mcp.md
2026-04-18-plan-2-hook-surface.md
2026-04-18-plan-2-smoke-findings.md
2026-04-18-plan-3-mcp-surface.md
2026-04-19-plan-4-dashboard.md
2026-04-29-plan-5-recovery-signal-deletion.md
2026-04-30-plan-6-spawn-fanout-rejoin.md
2026-05-01-plan-7-orchestrator-discipline.md
2026-05-02-plan-8-executor-overseer.md
```

**Existing specs dir:**

```
2026-04-15-galvolite-design.md
2026-04-17-plan-2-dashboard-crud-parity-design.md
2026-04-18-hooks-mcp-coordination-layer-design.md
2026-04-30-session-spawn-fanout-rejoin-design.md
2026-05-01-orchestrator-discipline.md
2026-05-02-plan-8-executor-overseer-design.md
```

---

## Plan 8 Deferred Items (still open after Plan 8 ship)

These are candidate scope for Plan 9 option 3 above:

- `GALVOLITE_OVERSEER` / `GALVOLITE_PLAN_EXECUTOR` flags default-on flip
- 7-day rec auto-archive sweep
- `ProjectDeleted` bus event (currently only `ProjectAdded` exists)
- `tail_per_session` API endpoint
- Deferred-recs UI toggle in RecModal
- UTC token-budget reset (daily rollover for `KIMI_DAILY_TOKEN_BUDGET`)
- `/recommendations/explain` re-prompt UI button (endpoint exists, no UI)

---

## Other Direction Hints (from memory)

- `project_multi_cli_direction.md` — future scope: Gemini CLI + GitHub Copilot
  CLI as additional agent adapters
- `project_github_scope.md` — PRD calls GalvoLite GitHub-native; zero GitHub
  code as of 2026-04-17 (still true)
- `feedback_gui_parity.md` — dashboard parity with API non-negotiable; CLI is
  a debugging escape hatch
- `feedback_idle_sessions_no_cost.md` — Claude Code context is per-turn, not
  wall-clock; don't invent polling loops
- `project_role_as_state.md` — orchestrator/brainstorm/implementation/scout
  transitions mid-session via `[ROLE:*]` signal
- `project_terminology.md` — project = logical grouping of sessions pointing at
  a directory GalvoLite doesn't own; `remove` never touches disk

---

## Resume Instructions for Next Session

1. **Read this file first.**
2. Optionally re-read the relevant memory files listed above for fuller
   context.
3. Re-invoke the brainstorming skill OR continue from where we left off:
   ask the user to pick from the 4 options above.
4. Once user picks, the brainstorming skill proceeds:
   one-question-at-a-time refinement → 2–3 approaches with recommendation →
   present design in sections (architecture, components, data flow, error
   handling, testing) → write spec to
   `docs/superpowers/specs/2026-05-06-plan-9-<topic>-design.md` → user
   reviews → invoke `superpowers:writing-plans` to produce the implementation
   plan.

**Critical constraints (from session memory) for whatever Plan 9 ends up being:**

- GUI must cover every API endpoint (no API-only features)
- Idle sessions consume zero context (no polling loops)
- `remove` operations never touch disk
- Roles transition via `[ROLE:*]` signal mid-session, not new sessions
- Sub-plans must produce working, testable software on their own (decompose
  if too big)
