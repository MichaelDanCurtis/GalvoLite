# Orchestrator Discipline + Memory Augmentation

**Status:** design memo (pre-plan)
**Date:** 2026-05-01
**Supersedes:** nothing yet — extends `2026-04-15-galvolite-design.md` §6, §10
**Author intent:** make the orchestrator a dumb program manager and the
executor a state-machine-advancer. Stop primary-session drift without
adding a graph DB.

---

## 1. Problem

Subagents (dispatched via Superpowers patterns) follow plans cleanly because
they get: tight scoped prompt, explicit return contract, fresh bounded
context. The **primary session of an executor** has none of those — context
grows, stop condition is fuzzy ("when done"), prompts are open-ended ("work
on Part 5"). Result: drift, repeated human-in-loop interventions, no rhyme
or reason to when the agent stalls.

Memory retrieval (graph DB or otherwise) does not fix any of those root
causes. It gives a drifting agent more rope.

What does fix it: **make every turn either advance a state machine or
escalate.** Remove the executor's discretion about "what to do next" — it
queries the next plan step or asks the operator. Same for orchestrator —
remove its discretion about how to route work; the directive already said.

## 2. Proposed primitives

### 2.1 `.galvolite/directive.md` — orchestration contract

Brainstorm produces two artifacts now, not one:

- `.galvolite/plan.md` — *what* (existing; spec §6.2). Source of truth for
  scope. Immutable except by explicit human edit.
- `.galvolite/directive.md` — *how to route*. Phase DAG, autonomy flags,
  escalation rules. Same immutability rule.

Format — markdown with structured tables, parseable:

```markdown
# Directive: <plan name>

Plan: ./plan.md

## Phases

| ID | Parts (from plan.md) | Autonomy | Depends on |
|----|----------------------|----------|------------|
| P1 | 1                    | full     | —          |
| P2 | 2, 3                 | review   | P1         |
| P3 | 5, 8, 9              | full     | —          |
| P4 | 4, 6, 7              | manual   | P2         |

Autonomy levels:
- `full`     — orchestrator may dispatch + auto-merge on green CI
- `review`   — dispatch ok, but human must approve PR before merge
- `manual`   — human must explicitly authorize each dispatch

## Escalation rules

- On schema/migration change → `ask_operator`
- On new dependency added → `ask_operator`
- On executor stuck >3 cycles → `notify_operator(level=warn)`
- On any decision not encoded above → `ask_operator`

## Notes

Free-form context the orchestrator should know — e.g. "P3 is independent of
the rest, can run in parallel from the start."
```

Backend: extend `.galvolite/` watcher (notify crate) to detect directive
changes. Surface in dashboard as a sibling tab to the plan view. Editable
via the existing plan-editor pattern.

### 2.2 "Advance state machine" rule for executor

New CLAUDE.md clause for `implementation` and `fix` profiles:

> **Every turn must end with one of:**
> - `plan_check_step({...})` — you completed a plan step
> - `status_report({kind: 'working', note: '<what next>'})` — heartbeat with
>   the *next* step you're about to take
> - `ask_operator({...})` — the directive doesn't cover this; escalate
> - `request_attention({...})` — blocked, need human
> - `write_handoff({...})` + `status_report({kind: 'complete'})` — done
>
> **Never end a turn with prose only.** If you find yourself about to write
> "Now I'll think about X..." — call `status_report` with X as the note and
> stop. The next turn picks it up cleanly.

This is mechanical. It removes the "drift between two thoughts in the same
turn" failure mode. The harness already has every tool needed; the rule just
forces use.

### 2.3 Orchestrator-as-router rule

New CLAUDE.md clause for `orchestrator` profile:

> **You do not decide. You route.** The directive (`.galvolite/directive.md`)
> tells you which parts run, in what order, with what autonomy. Your job:
>
> 1. Read directive + plan.
> 2. For each phase whose deps are satisfied, `spawn_session` per part.
> 3. `wait_children`. On rejoin, mark phase complete in the plan, advance.
> 4. **On any decision not encoded in the directive: `ask_operator`.**
>    Examples: "should P5 wait for P4's findings?", "executor surfaced an
>    architectural concern, expand scope?", "two executors want the same
>    file."
>
> **You are explicitly forbidden from:**
> - Choosing scope ("I'll add Y while we're here")
> - Resolving conflicts between executors yourself
> - Editing plan.md or directive.md (only humans do that)
> - Skipping `ask_operator` because "the answer seems obvious"

### 2.4 Postmortem memo type

When an executor finishes (terminal: `complete` or `error`), the harness
asks it to write `.galvolite/postmortems/<plan>-<part>-<session-id>.md`:

```markdown
---
plan_part: 5
session_id: 7f3a...
outcome: shipped | stuck | escalated
cycles: 3
duration: 47min
---

## What worked
- ...

## Gotchas
- DB migration locked when CI ran in parallel with P3
- `notify` crate fires duplicate events on macOS APFS

## Follow-ups
- (link to seed if filed)
```

Orchestrator reads existing postmortems at spawn time (small, < 10 per
project typically) and includes "known gotchas for similar parts" in the
spawn prompt. Cross-session learning without a graph DB.

### 2.5 Decision log memo type

When `ask_operator` returns an answer, the harness writes
`.galvolite/decisions/<session-id>-<ts>.md`:

```markdown
---
asked_by: 7f3a... (orchestrator)
asked_at: 2026-05-01T15:23Z
question: "Should P5 wait for P4's findings before dispatch?"
answer: "No, P5 is fully independent per directive."
---
```

Orchestrator (and future executors in same project) read recent decisions on
restore. Stops re-asking the same question after a handoff.

## 3. CLAUDE.md profile diffs

### 3.1 `IMPL_PROFILE` — add after "## Scope Rules"

```markdown
## Turn Discipline

Every turn ends by calling exactly ONE of:

- `plan_check_step({...})` — finished a plan step
- `status_report({kind: 'working', note: '<what next>'})` — heartbeat,
  state the next step explicitly
- `ask_operator({...})` — directive does not cover this decision
- `request_attention({...})` — blocked
- `write_handoff(...)` + `status_report({kind: 'complete'})` — done

Never end a turn with prose only. If you're about to write "Now let me
think about...", call `status_report` instead with that thought as the
note. The next turn resumes cleanly.
```

### 3.2 `ORCH_PROFILE` — replace `## Pattern` section

```markdown
## Inputs (you read both, never edit either)

- `.galvolite/plan.md` — what to build (immutable; humans edit)
- `.galvolite/directive.md` — how to route (immutable; humans edit)

## Pattern

1. Read directive. Identify phases whose deps are satisfied.
2. Read `.galvolite/postmortems/` and `.galvolite/decisions/` for any
   prior context on these phases.
3. For each ready phase, `spawn_session` per part with autonomy from
   directive.
4. `wait_children`. On rejoin, `plan_check_step` the completed parts,
   advance.
5. Repeat until directive's terminal phase is reached.

## You Do Not Decide

The directive decided. On any decision not in the directive, call
`ask_operator`. Examples that REQUIRE ask_operator:

- An executor surfaces an architectural concern not in plan
- Two executors need the same file
- A phase looks parallelizable but directive says sequential
- Scope creep request from any executor

You are explicitly forbidden from:

- Adding work not in plan.md
- Resolving inter-executor conflicts yourself
- Editing plan.md or directive.md
- Skipping `ask_operator` because "the answer seems obvious"
```

### 3.3 `BRAINSTORM_PROFILE` — add to "## Pattern"

```markdown
6. When the option matrix collapses to a chosen path, write BOTH:
   - `.galvolite/plan.md` — scope (existing)
   - `.galvolite/directive.md` — phase DAG, autonomy flags, escalation rules
   Then `set_plan({markdown})` and `request_attention` to hand off to the
   operator for spawn approval.
```

## 4. Implementation tasks (becomes a sub-plan)

Rough scope — not a full plan, just enough to estimate:

1. **Schema + watcher** (~ ½ day)
   - `.galvolite/directive.md` parser (markdown table → struct)
   - notify-crate watcher: emit `DirectiveChanged` on edit
   - dashboard event + sibling tab for directive view

2. **Postmortem + decision-log writers** (~ ½ day)
   - On executor terminal: harness prompts `write_postmortem` (new MCP tool
     or extend `write_handoff` with postmortem flag)
   - On `ask_operator` return: harness writes
     `.galvolite/decisions/<sid>-<ts>.md` automatically (no Claude action
     needed)

3. **Profile updates** (~ 1 hour)
   - `src/claude_md.rs` — diffs from §3 above
   - Tests: assert profiles contain the rule strings (regression-proof)

4. **Orchestrator spawn-time context injection** (~ ½ day)
   - When orchestrator spawns child, harness includes recent postmortems +
     decisions for the same plan part in the injection prompt
   - Cap: most recent 3 of each, < 2KB

5. **Brainstorm produces directive** (~ 1 hour, mostly profile)
   - Profile update from §3.3
   - Dashboard: spawn-button disabled until both files exist (currently
     only checks plan.md)

**Estimate:** 2-3 days, fits one sub-plan. Ships orchestrator discipline
+ memory augmentation in one cut.

## 5. What we're explicitly NOT doing

- **Neo4j / graph DB.** No current query needs traversal. The flat-file
  + index pattern handles cross-session lookup at the volume we have.
  Revisit when we have a real similarity-retrieval query that flat files
  can't answer.
- **Auto-mutation of plan.md or directive.md.** Both are human-only.
  Executor or orchestrator that wants to change them must `ask_operator`.
- **Cross-project memory.** Postmortems and decisions are per-project.
  Cross-project pattern recognition is a future problem; over-investing
  now picks the wrong shape.

## 6. Open questions

- Should `directive.md` be YAML/TOML instead of markdown-with-tables?
  Markdown is friendlier to humans and consistent with `plan.md`; tradeoff
  is parser complexity. Recommend markdown.
- Do we need a `directive.md` linter (warns on circular deps, missing
  parts)? Probably yes, but can ship as a follow-up.
- Postmortem writer: extend `write_handoff` or new tool? Extending is
  cheaper but conflates two concerns. Recommend new `write_postmortem`
  tool, takes same fields plus `outcome` enum.
