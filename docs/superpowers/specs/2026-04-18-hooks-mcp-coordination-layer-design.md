# Design: Hooks + MCP coordination layer

- **Date:** 2026-04-18
- **Status:** Design approved, pending implementation plan
- **Supersedes:** the stdout-signal pipeline introduced in Plan 1a (`src/signals.rs`, `SignalPump`), which is retired by this design.
- **Unblocks:** the three smoke findings from `docs/superpowers/plans/2026-04-18-plan-2-smoke-findings.md`, and the AI orchestrator persona planned post-Plan 4.

---

## 1. Summary

GalvoLite's current session→harness coordination channel is stdout-line parsing of bracketed tokens like `[STATUS:WORKING]`. The pipeline is structurally incompatible with Claude Code's TUI (which wraps every line with a `●` bullet and ANSI SGR escapes) and cannot express bidirectional queries, structured payloads, or lifecycle gating.

This design replaces stdout signals with two complementary channels:

- **MCP tools** — model-chosen, structured calls for active intent (`status_report`, `set_role`, `write_handoff`, `ask_operator`, `list_siblings`, …). Served by GalvoLite's existing Axum server at `/mcp` (SSE transport).
- **Claude Code hooks** — harness-configured, lifecycle-triggered callbacks for passive guarantees (`SessionStart`, `PreCompact`, `Stop`, `SessionEnd`, `UserPromptSubmit`, `PostToolUse(Bash)`, `Notification`). Served at `/hooks/ingest`.

All state transitions funnel through a single `state::apply_event()` function that atomically writes an append-only event log row *before* mutating session state. The event log is the authoritative "who did what when" record, durable across GalvoLite restarts.

The stdout signal parser is deleted.

---

## 2. Background and motivation

### 2.1 Smoke findings that drove this design

A real-claude smoke test against Plan 2's dashboard (documented in `docs/superpowers/plans/2026-04-18-plan-2-smoke-findings.md`) exposed three distinct breakages:

1. **SignalPump rejects every signal emitted by claude's TUI.** `src/signals.rs:48-63` requires the trimmed line to be exactly `[<KIND>]` or `[<KIND> key="val"]`. Claude's TUI prefixes every reply line with `● ` and wraps in ANSI SGR, so what lands in `LineBuffer` looks like `\x1b[32m● [STATUS:WORKING]\x1b[0m` — returns `None`. The entire signal protocol (STATUS, ATTENTION, HANDOFF, PLAN, SEED, ROLE) is unusable with real `claude`.
2. **Send-to-Session ships text without `\r`.** UI writes bytes to the PTY but doesn't submit. A one-line UI fix, orthogonal to this design but bundled in delivery.
3. **No AwaitingInput state.** Session state is binary (working/idle); a claude that's blocked on a y/n prompt looks "working" forever, defeating autopilot.

### 2.2 The PRD intent we never implemented

PRD §7 ("Plugin Specification") explicitly specified a fork of `thepushkarp/handoff` with six Claude Code hooks: `PreCompact`, `SessionStart`, `SessionEnd`, `Stop`, `UserPromptSubmit`, `PostToolUse(Bash)`. That was the intended transactional backbone for handoff-over-compaction, Stop-gating, and session continuity.

We built zero of it. Signals-in-stdout was Plan 1a-era scaffolding that was never meant to be the permanent coordination layer.

### 2.3 Why MCP instead of just fixing signals

- **Structured tool calls can't ANSI-mangle.** An MCP payload is a JSON object the client and server agree on. There's no TUI rendering layer between model intent and harness state.
- **Bidirectional by design.** Signals can only push. MCP tools can pull (`list_siblings`, `status_of`, `read_events`, `read_handoff`) — the substrate an AI orchestrator needs to reason about sibling state.
- **Claude Code natively supports MCP.** Tools appear in the model's tool menu, completions propose them, errors surface structurally.
- **Hooks handle what MCP can't.** Lifecycle events (pre-compact, stop-gate, session-end) fire independent of model cooperation. Hooks are the passive belt to MCP's active suspenders.

### 2.4 Why not Claude Code's Agent Teams feature

Agent Teams is an experimental CC feature (env-flag gated) where a lead session spawns managed teammates sharing a task list and mailbox. Evaluated, rejected at the outer orchestration layer:

- **Ephemeral, not persistent.** Teammates die with the lead. GalvoLite's premise is operator-visible, reattachable, restart-surviving sessions.
- **Opaque to the operator.** No hooks to render teammate PTYs in our dashboard, kill individual teammates, or inject messages from the GUI. Violates our dashboard parity rule.
- **No worktree isolation per teammate.** Plan 4 spawn/rejoin mandates `git worktree add` per child.
- **Experimental / flag-gated.** Depending on it for our production coordination substrate has the same fragility profile as the ANSI-signal trap we're fleeing.

Agent Teams remains acceptable *inside* a single GalvoLite session as a worker-layer optimization (threads inside a process). We do not build on top of it at the orchestrator layer.

---

## 3. Goals and non-goals

### 3.1 In scope

- GalvoLite-hosted MCP server at `/mcp` on the existing Axum process (SSE transport).
- Hook-ingest endpoint at `/hooks/ingest`.
- Bundled `galvolite-hook` binary + auto-generated per-session `.mcp.json` and `.claude/settings.json`, installed into the session's worktree at spawn time.
- Deletion of the stdout signal pipeline (`src/signals.rs` parser, SignalPump signal-consumer wiring).
- DB schema deltas to represent the richer state the new channels produce, plus the `events` append-only log.
- Dashboard WebSocket events and UI surfaces for new state, event timeline, live plan view, and operator-prompt modal.
- Optional integration with Docker MCP Gateway for a per-role MCP portfolio (no Docker requirement at baseline).

### 3.2 Non-goals (explicit)

- **AI orchestrator persona.** The CLAUDE.md profile that drives siblings via MCP is a later plan. This design delivers the substrate; reserved tool stubs keep the surface stable.
- **Spawn/rejoin implementation.** Worktree-per-child, `[WAIT:children]` semantics, and `[CHILD:COMPLETE]` notification are Plan 4's scope. MCP tool stubs (`spawn_child`, `list_siblings`, `wait_for_children`) exist but return `not_implemented` until that plan lands.
- **Programmatic context read/write.** Claude Code does not expose the session context to MCP tools. Reserved as stubs with documented capability dependency.
- **GitHub OAuth, PR pipeline, Copilot SDK.** PRD §9 roadmap material, unrelated to this coordination layer.
- **Chinese-for-compression pipeline.** Evaluated, rejected: modern BPE tokenizers are English-biased and typically cost *more* tokens for CJK per semantic unit. See §13.1.

---

## 4. Process topology

```
┌──────────────────────────────────────────────┐
│ GalvoLite (one Axum process, bound 127.0.0.1)│
│  ┌────────────┐  ┌──────────────┐            │
│  │ REST + WS  │  │ /mcp (SSE)   │            │
│  └────────────┘  └──────────────┘            │
│  ┌───────────────┐  ┌────────────────┐       │
│  │ /hooks/ingest │  │ state::apply_  │       │
│  └───────────────┘  │ event() funnel │       │
│             ▲       └────────────────┘       │
│             │              │                 │
│             │              ▼                 │
│             │       ┌──────────────┐         │
│             │       │ SQLite       │         │
│             │       │ (sessions,   │         │
│             │       │  events,     │         │
│             │       │  plan_steps) │         │
│             │       └──────────────┘         │
└─────────────┼────────────────────────────────┘
              │                 ▲
      ┌───────┴──────┐          │ SSE MCP
      │ galvolite-   │   ┌──────┴──────────┐
      │ hook binary  │   │ claude (child)  │
      │ in worktree  │◀──│ reads .mcp.json │
      └──────────────┘   │ & hook config   │
                         └─────────────────┘
```

All local. Server binds `127.0.0.1` only; tokens scoped per-session.

---

## 5. MCP tool catalog

Every tool accepts session identity implicitly via a bearer header set from `GALVOLITE_TOKEN` (§7). Tool names are stable once shipped — no renames without a migration.

### 5.1 Active coordination

**Status & lifecycle** (replaces `[STATUS:*]`, `[ATTENTION:*]`, `[PLAN:*]`, `[SEED:*]`)

- `status_report({ kind: "working"|"complete"|"blocked", note? })`
- `request_attention({ reason })`
- `set_plan({ markdown })` — parses `- [ ]` / `- [x]` checkboxes (§9.4) and fires `session.plan_replaced`.
- `plan_check_step({ step_hash | title, status, note? })` — granular update; fires `session.plan_step_changed`.
- `set_seed({ markdown })`

**Role** (replaces `[ROLE:*]`)

- `set_role({ role: "orchestrator"|"brainstorm"|"implementation"|"scout" })`

**Handoff** (replaces `[HANDOFF:*]`)

- `write_handoff({ path?, summary, next_steps })`
- `read_handoff({ path })`

**Operator comms** (new; dashboard-backed)

- `ask_operator({ question, choices?, timeout_secs? }) → { answer, timed_out }` — blocks via WS round-trip to dashboard.
- `notify_operator({ level: "info"|"warn"|"urgent", message })` — toast stack.

**Session introspection**

- `whoami() → { id, role, project_id, worktree, parent_id, depth, started_at }`
- `get_transcript({ session_id?, turns? })`
- `get_pty_log({ session_id?, since?, limit? })` — ANSI-stripped scrollback.

**Cross-session**

- `list_siblings({ project_id? }) → [{ id, role, state, last_event_ts, awaiting_input }]`
- `status_of({ session_id }) → full state snapshot`
- `read_events({ session_id?, since?, limit? }) → event log entries`
- `send_to_session({ session_id, text, submit?: bool })` — writes into a sibling PTY; `submit=true` appends `\r`.

**Self-control**

- `pause_self({ reason })` — voluntary idle, operator-dismissable.
- `request_end({ reason })` — graceful exit request, operator confirms.

### 5.2 Path ownership (PRD §4 parallelism primitive; tools reserved, stubs until Plan 4)

- `claim_paths({ globs })`
- `release_paths({ globs })`
- `list_claims() → [{ session_id, glob, claimed_at }]`
- `check_path({ path }) → { owner_session_id? }`

### 5.3 Spawn (reserved; stubs until Plan 4)

- `spawn_child({ role, task, depends_on?, worktree? }) → { session_id }`
- `wait_for_children({ ids: uuid[] })`

### 5.4 MCP portfolio management (reserved; implementation requires Docker MCP Gateway integration)

Session-level management is coherent only at spawn time — once tools are in the system prompt, runtime toggling cannot claw tokens back. Management tools therefore operate on *future* spawns.

- `list_available_mcps() → [{ name, running }]`
- `list_session_mcps({ session_id }) → [{ name }]`
- `configure_session_mcps({ session_id, servers: [...] })` — effective on next spawn of that profile.
- `gateway_toggle({ server, enable })`

### 5.5 GitHub surface (reserved; implementation requires PRD §9 Phase 1)

- `open_pr`, `list_prs`, `request_review`, `merge_pr`, `get_pr_comments` — stubs return `not_implemented`.

### 5.6 Context r/w (reserved; capability-blocked)

- `read_context()`, `write_context()` — not implementable under Claude Code today. Migration to Claude Agent SDK required for specific sessions if this becomes load-bearing. Reserved name only.

### 5.7 Rules applied

- **One tool does one thing.** `set_role` does not also status-report.
- **Idempotent or dedup-keyed.** Retries are safe. Every tool call includes an `event_id` used as UNIQUE key in the events table.
- **Every call writes one event-log row before mutating state.** No code path bypasses the log. This is the atomicity invariant.
- **Tool definitions are small.** Target: full catalog including reserved stubs fits in a few hundred tokens of system prompt.

---

## 6. Hook surface

Installed via `.claude/settings.json` in the session's worktree scope — never touches the user's global CC config. All seven hooks dispatch to a single `galvolite-hook` binary (installed to `~/.galvolite/bin/`) which curls `/hooks/ingest` with `$GALVOLITE_SESSION_ID` + `$GALVOLITE_TOKEN` and optionally writes `inject_text` from the server response to stdout.

### 6.1 Envelope (all hooks)

```
POST /hooks/ingest
Headers: X-Galvolite-Token: <token>
Body: {
  session_id: "<uuid>",
  hook:       "<name>",
  ts:         "<iso8601>",
  event_id:   "<uuid>",       // dedup key
  payload:    { ...hook-specific... }
}
Response: {
  action: "proceed"|"continue"|"block",
  inject_text?: "<string>",
  note?: "<string>"
}
```

Server writes the events row *before* computing the response. Response is derived from post-write state.

### 6.2 Per-hook behavior

| Hook | Payload | GalvoLite behavior | Purpose |
|------|---------|-------------------|---------|
| `SessionStart` | `{is_resume, cwd}` | Build seed text: profile CLAUDE.md keyed by `sessions.role`, last handoff summary (if any), current MCP tool pointer. Return as `inject_text`. | Load-bearing for handoff continuity. Replaces ad-hoc prompt injection in `session_manager.rs`. |
| `SessionEnd` | `{exit_code, reason}` | Transition row: clean → `ended`, unclean → `error`. If no `write_handoff` seen in last N events, auto-generate minimal handoff from event log + PTY scrollback; write `.galvolite/handoffs/{id}/pointer.json`. | Covers clean exits; orphan reconciler still covers crash case. |
| `PreCompact` | `{trigger_reason}` | If no `write_handoff` since last checkpoint, inject "write_handoff now" prompt so model writes before compaction compresses context. Else proceed. | Enforces handoff-over-compaction structurally, not by polite CLAUDE.md request. |
| `Stop` | `{turn_id}` | If `working` with no recent `status_report`, inject "report status or continue" prompt (max 3 retries). Else transition `working → idle`. **If no subsequent `UserPromptSubmit` within 3s of Stop, mark `awaiting_input`.** | Finding 3 dissolves: no PTY heuristics, no prompt-shape regex. |
| `UserPromptSubmit` | `{prompt_preview, est_tokens}` | Increment `turn_count`; accumulate `est_tokens`; at 80% of context limit inject warning prompt. Transitions `awaiting_input → working`. | Real counter, not the prior heuristic. |
| `PostToolUse(Bash)` | `{command, exit_code, stdout_preview}` | Pattern-match `git push`, `git commit`, `cargo test`, `npm test`. Log structured events. Raise attention on push. Future: trigger PR flow. | Operator visibility into what the session actually did to the repo. |
| `Notification` | `{level, message}` | Mirror to operator notify path; dashboard toast. | CC-native attention channel without requiring MCP call. |

### 6.3 Atomicity

Each hook response is computed from post-write state. A concurrent MCP call landing between hook fire and hook response is visible to the response computation. No read-modify-write races: all transitions happen inside one SQLite transaction that includes the events append.

---

## 7. Session identity and `.mcp.json` injection

### 7.1 Spawn sequence

`SessionManager::spawn` gains these steps (inside a transaction):

1. **Allocate session row.** Generate `session_id` (uuid v4), insert row with role/project/parent.
2. **Mint bearer token.** 32-byte random hex. Stored in `sessions.api_token`. Single-session scope.
3. **Resolve worktree.** Reuse today's worktree logic. Plan 4 will replace with `git worktree add`; identity plumbing here is unchanged.
4. **Write per-session CC config into worktree:**
   - `CLAUDE.md` — role-specific profile body from `claude_md::render(role, project_ctx)`. CC auto-loads. Rewritten on `set_role` MCP call for mid-session role change.
   - `.claude/settings.json` — hook table pointing all seven hooks at `~/.galvolite/bin/galvolite-hook`.
   - `.mcp.json` — coordination endpoint + optional portfolio gateway:
     ```json
     {
       "mcpServers": {
         "galvolite": {
           "type": "http",
           "url": "http://127.0.0.1:3001/mcp",
           "headers": { "Authorization": "Bearer <token>" }
         },
         "portfolio": {
           "type": "http",
           "url": "http://127.0.0.1:<gw-port>/mcp"
         }
       }
     }
     ```
     The `portfolio` entry is only written when a Docker MCP Gateway is configured for that role.
5. **Inject env.** Child process env extends OS env with:
   ```
   GALVOLITE_SESSION_ID=<uuid>
   GALVOLITE_API_BASE=http://127.0.0.1:3001
   GALVOLITE_TOKEN=<bearer>
   ```
6. **Spawn PTY.** `claude` launched with `cwd` = worktree, env as above. PID recorded.

### 7.2 Auth

Every `/mcp/*` and `/hooks/ingest` request presents `GALVOLITE_TOKEN` (MCP via `Authorization: Bearer`, hooks via `X-Galvolite-Token`). Server does constant-time compare against `sessions.api_token`, resolves to `session_id`. On `SessionEnd`, token is deleted — stale curl attempts get 401.

### 7.3 Reattach

If PTY dies but row remains, operator-initiated reattach spawns a new `claude` with the same `session_id` and a newly-minted token (old revoked). Row unchanged, event log continuous. `SessionStart` fires with `is_resume: true`; seed prompt includes "reattached from prior run — last known state: …".

### 7.4 Transport fallback

SSE is CC's HTTP-MCP path. If client-side quirks surface, the fallback is a stdio shim binary that proxies to our HTTP endpoint. Documented as a capability dependency, not a day-one requirement.

---

## 8. State model, schema, and event log

### 8.1 Session lifecycle

```
Starting ──▶ Idle ──▶ Working ──▶ AwaitingInput
   │         ▲ ▲  │       │             │
   │         │ │  └──▶ NeedsAttention   │
   │         │ └───────────┤            │
   │         └─────────────┴────────────┘
   └──▶ Error
        │
Ended ◀─┴── SessionEnd
```

Transitions are driven exclusively by MCP tool calls, hook payloads, or operator actions. Never by PTY-byte heuristics.

### 8.2 DB delta

```sql
-- Extend sessions
ALTER TABLE sessions ADD COLUMN api_token TEXT;
ALTER TABLE sessions ADD COLUMN state TEXT NOT NULL DEFAULT 'starting';
ALTER TABLE sessions ADD COLUMN turn_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN est_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN last_status_note TEXT;
ALTER TABLE sessions ADD COLUMN handoff_path TEXT;
ALTER TABLE sessions ADD COLUMN plan_markdown TEXT;
ALTER TABLE sessions ADD COLUMN seed_markdown TEXT;
ALTER TABLE sessions ADD COLUMN last_event_ts DATETIME;

-- Append-only event log
CREATE TABLE events (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  ts            DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  source        TEXT NOT NULL,            -- 'mcp'|'hook'|'operator'|'system'
  kind          TEXT NOT NULL,            -- tool or hook name
  payload_json  TEXT NOT NULL,
  result_json   TEXT,
  prev_state    TEXT,
  next_state    TEXT,
  dedup_key     TEXT UNIQUE
);
CREATE INDEX events_session_ts ON events(session_id, ts);
CREATE INDEX events_kind_ts    ON events(kind, ts);

-- Structured plan steps (derived from set_plan markdown)
CREATE TABLE plan_steps (
  id           INTEGER PRIMARY KEY,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  step_hash    TEXT NOT NULL,
  ordinal      INTEGER NOT NULL,
  title        TEXT NOT NULL,
  status       TEXT NOT NULL DEFAULT 'pending',  -- pending|in_progress|done|blocked
  note         TEXT,
  owner_id     TEXT REFERENCES sessions(id),
  updated_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(session_id, step_hash)
);

-- Path claims (reserved; populated when Plan 4 implements spawn/rejoin)
CREATE TABLE path_claims (
  id         INTEGER PRIMARY KEY,
  session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  glob       TEXT NOT NULL,
  claimed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(glob)
);
```

### 8.3 Atomicity invariant

Every state transition is one SQLite transaction containing `(event append, session update, optional plan_steps update)`. No code path mutates session state without writing the event first. This is enforced by making all mutations go through a single `state::apply_event()` function; `SessionManager`, MCP handlers, and hook handlers all call it instead of writing the DB directly.

### 8.4 Plan step extraction

`set_plan({markdown})`:

1. Tokenize markdown for list items matching `^\s*-\s*\[([ xX])\]\s+(.+)$`.
2. Compute `step_hash = sha256(normalize(title))[:16]` per step.
3. Upsert `plan_steps` rows keyed by `(session_id, step_hash)`; update `ordinal` to match source order.
4. Diff against prior state; emit `session.plan_step_changed` WS event for each row whose `status` changed.
5. Set `sessions.plan_markdown` to the raw markdown.

`plan_check_step` takes either `step_hash` or `title` (hashed server-side); updates a single row; emits the same WS event.

### 8.5 Event-log usage

- **Recovery on restart** — replay last 5 minutes of events for each `starting|working|awaiting_input|needs_attention` session; reconstruct last known intent.
- **Forensics** — `read_events` MCP tool exposes to future orchestrator persona; dashboard Events tab renders to operator.
- **Timeline UI** — reverse-chrono feed per session (§10.3).
- **Retention** — no auto-prune in this design. Acceptable for solo-dev single-machine usage. Add policy when pain surfaces.

### 8.6 Transition reference (abridged)

| From | Event | To | Notes |
|------|-------|-----|-------|
| Starting | `SessionStart` hook | Idle | — |
| Idle | `UserPromptSubmit` hook | Working | first turn |
| Working | `Stop` hook (clean) | Idle | normal end-of-turn |
| Working | `Stop` hook + no input 3s | AwaitingInput | Finding 3 fix |
| Working | `status_report(blocked)` | NeedsAttention | model-initiated |
| Working | `request_attention` | NeedsAttention | — |
| AwaitingInput | `UserPromptSubmit` hook | Working | operator answered |
| any | `request_end` | Ended | voluntary |
| any | PTY death without `SessionEnd` | Error | orphan reconciler |

### 8.7 Migration

ALTER TABLE additions are backward-compatible. `events`, `plan_steps`, `path_claims` are net-new. Bump `schema_version`. Existing sessions at upgrade time: state = `error`, note "pre-migration; state unknown." One-time operator reset acceptable for solo-dev use.

---

## 9. Deletions

### 9.1 Delete

- `src/signals.rs` — parser. Dead.
- `SignalPump` consumer wiring that applied parsed signals to DB state. All transition logic moves into `state::apply_event()`.
- Test fixtures exercising `parse_signal` against TUI captures.
- CLAUDE.md lines instructing `[STATUS:*]` / `[ROLE:*]` / etc. emission. Replace with MCP tool guidance.

### 9.2 Keep, unchanged

- PTY byte capture + `vt100::Parser` + scrollback ring — still required for terminal reattach.
- PTY WebSocket broadcast — unaffected.
- Orphan reconciler (`4e6ea7b`) — still correct for PTY-died-without-SessionEnd.

### 9.3 Keep, repurpose

- `LineBuffer` — lift from "signal consumer" to "transcript producer" for `get_pty_log` / `get_transcript` MCP tools. Minor rename + drop parser call.

### 9.4 Migration order

Two commits, deliberately not one:

1. **Add new surfaces alongside old.** Hooks + MCP endpoint + new state funnel land. Signal pipeline still runs in parallel. End-to-end verifiable against real `claude` without delete risk.
2. **Delete signal pipeline.** Follow-up commit once real-claude smoke confirms hooks+MCP carries the load. Revert path is one `git revert`.

---

## 10. Dashboard integration

### 10.1 New WebSocket events

- `session.state_changed { session_id, prev, next, last_event_ts }`
- `session.event_appended { session_id, event }`
- `session.role_changed { session_id, role }`
- `session.meta_changed { session_id, turn_count, est_tokens, plan_markdown?, seed_markdown?, handoff_path? }`
- `session.plan_replaced { session_id, steps: [...] }`
- `session.plan_step_changed { session_id, step_hash, prev_status, next_status, note? }`
- `operator.prompt { prompt_id, session_id, question, choices?, timeout_secs? }`
- `operator.toast { session_id, level, message }`

### 10.2 Component changes

- **`SessionRow`** — expanded state-color map (idle / working / awaiting_input / needs_attention / error / ended), role badge, handoff tick.
- **`ExpandedSessionView`** — tabbed layout: **Terminal** (existing) / **Events** (new) / **Plan** (new) / **Meta** (new).
- **`EventsTab`** — reverse-chrono feed filtered to session; row shows ts, source badge, kind, one-line summary, expandable payload JSON. Search + kind filter.
- **`PlanTab`** — live checklist rendered from `plan_steps`; status pills (○ pending / ◐ working pulse / ● done / ◆ blocked); current step highlighted; completed collapsed by default; future: owner_id attribution for orchestrator-driven flows.
- **`MetaTab`** — current plan_markdown, seed_markdown, handoff link, turn/token counters.
- **`OperatorPromptModal`** *(new)* — fires on `operator.prompt`; renders question + choices or freeform; POST `/operator/prompt/{id}/answer`; server returns answer to waiting `ask_operator` MCP call; timeout auto-closes with `{ answer: null, timed_out: true }`.
- **Toast stack** — top-right overlay, up to 5, auto-dismiss by level (info 4s, warn 8s, urgent sticky).
- **Rail per-session** — gains "Event log" button opening Events tab; existing Send/Handoff/End buttons unchanged (Send picks up Finding 2 `\r` fix).

### 10.3 Parity rule compliance

Every new MCP tool and every new state set by a hook is visible or actionable in the dashboard. No DB state that the operator can't see or override. (Per memory `feedback_gui_parity`.)

---

## 11. Failure modes & recovery

### 11.1 Failure catalog

| Failure | Detection | Response |
|---------|-----------|----------|
| GalvoLite crashes/restarts | Startup scan finds non-terminal sessions | Recovery protocol (§11.2) |
| PTY dies without `SessionEnd` | Orphan reconciler (existing) | Mark `error`; last event log points at cause |
| Hook script can't reach server | curl non-zero | Script writes to `~/.galvolite/hook-fallback.log` and exits 0 so CC doesn't crash; server replays on next online request |
| MCP tool handler error | Handler returns `{error}` | Logged as event; state unchanged; model sees error in tool response |
| Malformed MCP payload | Schema validation | 400 with structured error; event logged with `source: system` |
| Invalid/revoked token | Constant-time compare fails | 401, no info leak; event logged with remote |
| `ask_operator` timeout | Server-side timer | Tool resolves `{answer: null, timed_out: true}`; state returns to pre-prompt |
| Hook response too slow | 5s hard timeout in binary | Empty response (no inject), CC continues; server still logs on late completion |
| Docker MCP Gateway unavailable | `.mcp.json` gateway entry connect fails | Session runs on `galvolite` MCP only; operator sees toast; no hard failure |
| SQLite write failure | TX error | Bubbled as 500; atomic update means no half-states |
| Concurrent MCP + hook writes | SQLite serialization | One wins by TX ordering; both events logged; state convergent |

### 11.2 Restart recovery protocol

On GalvoLite startup:

1. Load all sessions with state ∈ {starting, idle, working, awaiting_input, needs_attention}.
2. For each, verify PTY PID alive on OS.
3. **Alive PTY:**
   - Reattach (rebind WS broadcast to PTY FD or re-establish read loop).
   - Regenerate `api_token`; rewrite `.mcp.json` if stale.
   - Replay last 20 events via WS `session.resumed`.
   - Append synthetic `system.recovered` event marking restart boundary.
   - State preserved.
4. **Dead PTY:**
   - Transition to `error` with synthetic `system.orphaned` event.
   - Attempt auto-generated handoff from last event + cached PTY scrollback.
   - Row badge turns red; operator offered "retry (new PTY, resume from handoff)" or "end."
5. **Hook-fallback replay:** scan `~/.galvolite/hook-fallback.log`; ingest in order; delete on success.

### 11.3 Token compromise

Tokens are per-session + localhost-only. A leaked token grants access only to that one session's coordination surface from the same host. Rotate by killing session. If we ever expose cross-host (Tailscale), bump to mutual-TLS between hook scripts and server.

### 11.4 Invariants

- Event log row written *before* state row mutated.
- Every tool call is idempotent or has a unique `event_id` dedup key.
- No code path silently blocks on operator — every wait has a timeout.

---

## 12. Testing strategy

### 12.1 Unit (in-process, fast)

- `state::apply_event` — table-driven transition tests; atomicity verified via in-memory SQLite.
- MCP tool handlers — happy path + malformed payload + auth failure per tool.
- Hook payload parsing — one representative CC-shape payload per hook kind; property-test dedup idempotency.
- Markdown plan parser — checkbox extraction, stable hashing, ordinal preservation, diff computation.

### 12.2 Integration (real HTTP + temp-file SQLite)

- `/hooks/ingest` end-to-end per hook — POST → DB write → WS emission.
- `/mcp/*` SSE session — open stream, call each tool, assert envelope + event log rows.
- Concurrent hook + MCP writes — SQLite serialization produces convergent state.
- Token lifecycle — mint, use, revoke on SessionEnd, 401 after revoke.

### 12.3 Recovery (process-level)

- Crash-restart harness: spawn mock session, inject events, SIGKILL GalvoLite, restart, assert replay protocol for both alive-PTY and dead-PTY paths.
- Hook-fallback replay: write to fallback log with server down, start server, verify ingestion and cleanup.

### 12.4 Real-claude smoke (mandatory before merge)

- Recorded claude-TUI byte streams as fixtures — specifically the ANSI-wrapped output that broke `signals.rs`. New pipeline must ignore stdout entirely. Fixture-based regression proof Finding 1 cannot reoccur.
- Scripted 15-minute session against real `claude` with known task. Expected artifacts: correct state transitions, event log populated, handoff written on PreCompact, no stdout-signal code path executed.

### 12.5 UI (Vitest + JSDOM)

- `EventsTab`, `PlanTab`, `OperatorPromptModal` — prop/event-driven renders, live WS-driven transitions.
- Existing 37 UI + 96 Rust tests must remain green.
- Send modal picks up `\r`-on-submit fix (Finding 2) with new test.

### 12.6 Out of scope for testing

- Docker MCP Gateway internals (treat as third-party, mock port).
- CC host behavior (we trust the CC hook / MCP client; catch breakage in smoke).

### 12.7 Fixture hygiene

- New fixtures live under `src/state/tests/fixtures/`.
- Real-claude captures checked in as raw bytes with short README describing capture conditions for future recreation.

---

## 13. Rejected alternatives

### 13.1 Chinese-for-compression pipeline

**Proposed:** sessions communicate and comment code in Chinese to reduce token cost; a cheap model translates to English at PR time.

**Rejected because:**
- Modern BPE tokenizers (Anthropic, OpenAI) are English-biased. A single Chinese character typically splits into 2–3 BPE tokens; per *token*, Chinese is usually more expensive per semantic unit than English, not cheaper.
- Round-trip translation introduces semantic drift precisely where precision matters (comments encoding intent, symbol names, commit messages).
- Mixed-language codebase is ungreppable for a solo dev.
- Operational cost of maintaining a translation pipeline likely exceeds any tokenization savings, if savings are even real.

**Alternatives that achieve the same spirit:** caveman-speak CLAUDE.md mode (structural compression, legit wins); terse MCP payload conventions; dashboard token meter for visibility; handoff-driven compaction (far bigger savings than encoding tricks).

### 13.2 Agent Teams as outer orchestration layer

See §2.4. Fundamentally ephemeral/opaque to operator; rejected. Acceptable inside a single session as worker-layer optimization.

### 13.3 Approach B: MCP sidecar binary + Unix socket

Separate `galvolite-mcp` binary with UDS IPC. Cleaner separation but zero user-visible benefit today. YAGNI until/unless we want to ship MCP as a standalone artifact.

### 13.4 Approach C: stdio MCP per child + plugin fork

PRD-faithful in letter (fork `thepushkarp/handoff`) but wrong in spirit for our cross-session use case. Each stdio server knows only its own session; cross-session tools (`list_siblings`, orchestrator queries) would need an upstream IPC path anyway, doubling the plumbing.

### 13.5 Runtime MCP toggle from inside a session

Self-cancelling: you can't call a tool to curate the toolset before the session that has the tool exists, and toggling tools mid-session cannot claw back tokens already in the system prompt. Moved to spawn-time profile curation (§7.1) + future Docker MCP Gateway portfolio management.

### 13.6 SSE-only MCP transport (the older spec revision)

**Proposed:** Implement only the legacy two-endpoint SSE transport (`GET /mcp/sse` + `POST /mcp/messages`) since CC's reference clients shipped with that shape first.

**Rejected because:**
- The 2025-03-26 MCP spec revision marks SSE-only as deprecated and adds Streamable HTTP as the recommended transport.
- Streamable HTTP collapses two endpoints into one, removes the requirement for clients to maintain a separate event-stream connection for trivial request/response tools (which is most of our catalog), and preserves SSE as an optional per-request upgrade for handlers that genuinely stream.
- Implementing the deprecated path first means a forced migration when CC bumps client versions; implementing Streamable HTTP first means the legacy path becomes a backward-compat fallback only if §15 dep #1 lands negative.

**Alternatives considered:** dual-host both transports on different routes (`/mcp` vs `/mcp/sse`) — wait until §15 dep #1 has data before paying the maintenance cost.

---

## 14. Future work

- **AI orchestrator persona** — CLAUDE.md profile that drives sibling sessions via MCP. Substrate (this design) is ready; persona is a separate plan.
- **Plan 4 — spawn/rejoin** — implements the reserved `spawn_child` / `wait_for_children` / `claim_paths` tool surface, adds worktree-per-child, builds on event-log + hook plumbing.
- **GalvoLite-native MCP portfolio manager** — eventual replacement for Docker MCP Gateway dependency if/when we want lighter install story.
- **Caveman mode** — opt-in CLAUDE.md addendum for token-lean in-session reasoning. Reserved as `sessions.compact_speech` flag.
- **Agent SDK migration path** — for sessions requiring programmatic context read/write (`read_context` / `write_context`), consider spawning those specific session types via Claude Agent SDK instead of PTY-driven `claude`. Changes nothing for baseline sessions.
- **Cross-host deployment** — Tailscale exposure requires TLS on MCP + hooks endpoints and mutual-auth between `galvolite-hook` binary and server.
- **Plan-step ownership attribution** — when orchestrator spawns children, `plan_steps.owner_id` shows "step N being executed by child-session-X" in the live Plan tab.

---

## 15. Dependencies to verify before implementation

These are open capability questions whose answers shape implementation but not the design shape:

1. **CC Streamable HTTP MCP transport stability.** Does Claude Code's MCP client implement the 2025-03-26 Streamable HTTP transport (single POST endpoint, optional SSE upgrade)? If not, fall back to the legacy SSE-only path documented at §7.4, which keeps two endpoints (`POST /mcp/messages` + `GET /mcp/sse`) bridged through the same dispatcher. Stdio shim is the secondary fallback.
2. **CC `tools/list_changed` round-trip.** If/when we implement dynamic MCP toggling, does CC re-render the system prompt on `tools/list_changed` notifications? If not, runtime toggling can only *add* tools, never remove tokens.
3. **PreCompact hook `inject_text` semantics.** Does CC give the model a turn to respond to injected text before compaction proceeds? Needs verification; falls back to blocking compact via `action: "block"` if necessary.
4. **Stop hook max-retries.** CC documents a retry limit on Stop-hook continue actions; confirm current value and align our "max 3 retries" accordingly.
5. **Docker MCP Gateway API stability.** Recent product; API surface moving. Wrap integration behind a `McpPortfolio` trait so the backend is swappable.

Each dependency becomes a checkpoint in the implementation plan.

---

## 16. Appendix: what this design does NOT change

- Dashboard project/session management CRUD (Plan 2 scope) — untouched.
- Terminal-reattach pipeline (PTY byte capture, `vt100::Parser`, scrollback ring) — untouched.
- Interactive terminal input (Plan 3 scope) — untouched, picks up one bug fix for `\r`-on-submit.
- Role-as-state (Plan 3 scope) — still lives on `sessions.role`; `set_role` MCP tool replaces the `[ROLE:*]` signal as the write path.
- Orphan reconciliation logic (`4e6ea7b`) — still correct.
- GitHub OAuth, PR pipeline, Copilot SDK — untouched; reserved MCP tools stubbed only.
