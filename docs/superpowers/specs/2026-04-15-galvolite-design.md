# GalvoLite — Design Spec

**Date:** 2026-04-15
**Author:** Michael Curtis (with Claude)
**Status:** Design approved, ready for implementation planning
**Supersedes:** `LUCIDITY-ORCHESTRATOR-PRD-v3.md` (for the Lite scope)

---

## 1. What GalvoLite Is

GalvoLite is a **solo continuity engine with optional multi-worktree decomposition and PR/Copilot review integration** for Claude Code. It automates the save/clear/restore ritual that a solo developer runs by hand today when juggling many Claude Code sessions — both across independent projects and within one project split into parallel sub-threads — and surfaces the whole fleet in a single dashboard.

It is a sibling to **Galvo** (Lucidity's enterprise agent-networking platform) but intentionally smaller in scope:

- **Solo user**, not teams
- **Free** to use; Galvo remains the enterprise product
- **Single-machine** with optional Tailscale access for remote browser monitoring
- **No A2A / marketplace / trust tier** — those are Galvo's moat
- May interoperate with Galvo APIs later; for v1, standalone

### The pain it solves

The user currently runs four-plus concurrent tmux windows, each a different project or sub-thread. Each window cycles through:

```
brainstorm → work → save to memory → /clear → restore → continue
```

The manual ritual is the single biggest friction point. GalvoLite's v1 north star: **the harness runs this loop for the user, losslessly, cheaply.**

### Priorities (from brainstorm ranking)

1. **Automate the ritual itself** (eliminate manual labor across N windows)
2. **Lossless save at the context boundary** (never lose work to overflow)
3. **Cost discipline** (terse machine-to-machine comms; save output tokens)
4. **Memory continuity** (good-enough; don't over-engineer a semantic layer)

---

## 2. Scope

### In v1

- Project discovery and dashboard management
- Brainstorm sessions that produce a decomposition plan
- Automated multi-session spawning from that plan (one Claude process per planned parallel part, each in its own git worktree on its own branch)
- Autonomous save/clear/restore ritual per session (per-session autopilot toggle)
- Ground-truth context-pressure detection via a Claude Code plugin
- PR auto-creation on session completion
- Copilot review findings surfaced inline with per-issue actions (Fix with Copilot / Spawn fix session)
- Seeds subsystem with Scout sessions
- Dashboard: row-per-instance, mini live terminal, hover-expand, click-to-interact
- Tauri desktop app + same web frontend served over network for Tailscale remote access
- GitHub OAuth App device flow

### Explicitly deferred (not in v1)

- Copilot SDK–based coordination intelligence (triage, validation, PR descriptions)
- Team/member collision detection (solo user + worktrees makes it moot)
- Cross-instance messaging
- Token cost tracking
- Session recording / replay
- CI integration
- Alternative review providers (CodeRabbit, SonarQube, etc.)
- Mobile-native client
- Team / multi-user
- Enterprise compliance / audit trail

### Out of scope (possibly ever)

- Full GitHub UI replacement
- Code editor
- A2A inter-org protocol (Galvo's job)
- Trust-tier marketplace (Galvo's job)

---

## 3. Core Concepts

| Concept | Definition |
|---|---|
| **Master folder** | User-configured parent directory (e.g., `~/code/`). Contains project folders. GalvoLite scans it for git repos. |
| **Project** | One git repo inside the master folder. Has its own `.galvolite/` subdirectory holding plan, worktrees, handoffs, seeds. |
| **Session** | A long-lived Claude Code process GalvoLite manages. Has exactly one pty, one worktree, one branch (except brainstorm), one task, one role. |
| **Role** | One of `brainstorm`, `implementation`, `scout`, `fix`. Determines which CLAUDE.md profile is injected. (`fix` uses the Implementation profile with a specialized initial prompt.) |
| **Cycle** | One `/clear` boundary within a session's life. A session has N cycles; each cycle ends when context pressure triggers the ritual. |
| **Ritual** | The automated sequence: save-memory → wait for the session's `write_handoff` MCP call → send `/clear` → send restore prompt. The process is never killed during normal operation. |
| **Plan** | `.galvolite/plan.md` — structured decomposition Claude writes during brainstorm. Harness parses to spawn sessions. |
| **Handoff** | Per-cycle durable record at `.galvolite/handoffs/<session-id>/<timestamp>.md`. Contains continuation prompt consumed by the next cycle. |
| **Seed** | Out-of-scope idea captured mid-work at `.galvolite/seeds/SEED-NNN-<slug>.md`. Promotable to a task. |

---

## 4. Architecture

```
┌───────────────── GalvoLite ─────────────────┐
│                                             │
│  ┌─────────────────────────────────────┐   │
│  │  Tauri app (native shell)           │   │
│  │  └─ React + xterm.js frontend       │   │
│  └──────────────┬──────────────────────┘   │
│                 │ HTTP + WebSocket          │
│  ┌──────────────┴──────────────────────┐   │
│  │  Rust core (tokio + axum)           │   │
│  │  ┌──────────┐  ┌───────────────┐   │   │
│  │  │ Session  │  │ GitHub layer  │   │   │
│  │  │ Manager  │  │ (OAuth,       │   │   │
│  │  │ (pty,    │  │  REST,        │   │   │
│  │  │  signal  │  │  GraphQL)     │   │   │
│  │  │  router, │  └───────────────┘   │   │
│  │  │  ritual  │  ┌───────────────┐   │   │
│  │  │  driver) │  │ Git (worktree │   │   │
│  │  └──────────┘  │  manager)     │   │   │
│  │  ┌──────────┐  └───────────────┘   │   │
│  │  │ SQLite   │  ┌───────────────┐   │   │
│  │  │ (ops     │  │ File watcher  │   │   │
│  │  │  state)  │  │ (plan.md,     │   │   │
│  │  └──────────┘  │  seeds)       │   │   │
│  │                └───────────────┘   │   │
│  └──────────────┬──────────────────────┘   │
│                 │ pty                       │
│                 ▼                           │
│  ┌─────────────────────────────────────┐   │
│  │ N long-lived Claude Code processes  │   │
│  │ (one per active session)            │   │
│  └─────────────────────────────────────┘   │
│                                             │
└─────────────────────────────────────────────┘

Serves frontend two ways:
  1. Locally via Tauri webview (native app feel)
  2. Over HTTP on configurable interface → Tailscale remote access

Also installs:
  - Claude Code plugin (PreCompact + tool-call counter) for context-pressure ground truth
```

### Why Tauri, not a web daemon alone

- Native app feel matches "I open GalvoLite like I open Slack"
- Single binary install lowers barrier for a free tool
- Core Rust daemon still binds HTTP on a configurable interface so Tailscale remote-browser access works with zero extra product

### Why no Copilot SDK in v1

The per-issue UI (Fix with Copilot button + Spawn fix session button) replaces the need for severity triage. The user is the judge. Simpler code, faster ship.

---

## 5. The Continuity Loop (Save / Clear / Restore)

The heartbeat of the product. Drives the E and A priority goals.

### 5.1 Process model

- One session = one long-lived `claude` CLI process controlled via pty
- The process is **never killed** during normal operation. It is only torn down on: (a) explicit user "Kill session" click, (b) fatal pty/process error, or (c) successful `status_report({kind: 'complete'})` + PR merge.
- `/clear` is sent as a keystroke; the conversation resets but the process stays alive, authentication is preserved, startup cost is paid once per session
- Cycles are delineated by `/clear` events

### 5.2 Triggers (layered)

| Layer | Mechanism | Reliability | Role |
|---|---|---|---|
| Primary | **Claude Code plugin**: hooks `PreCompact` + `UserPromptSubmit` counter | Ground truth | Fires when real compaction is imminent or tool-call threshold hit |
| Fallback | **"Handoff now" button** in dashboard | Manual, always works | User override |
| Safety net | **CLAUDE.md instructs Claude to call `request_attention({reason: 'context'})`** when it self-senses pressure | Unreliable | Extra belt-and-suspenders |

### 5.3 The ritual sequence

```
1. Trigger fires (plugin signal, button click, or Claude self-signal)
2. Harness injects (via stdin): "Handoff time. Write your handoff per CLAUDE.md."
3. Claude writes .galvolite/handoffs/<session-id>/<timestamp>.md
   Includes: state summary, decisions made, files touched, open questions,
   next steps, and a VERBOSE, SELF-CONTAINED continuation prompt for the
   next cycle.
4. Claude calls `write_handoff({summary, next_steps})`; harness validates and persists the file.
5. Harness sends /clear
6. Harness sends restore prompt:
   "Read .galvolite/handoffs/<session-id>/<latest>.md. You are the next
    cycle. Resume from the Continuation Prompt section."
7. New cycle begins. MEMORY.md is auto-re-injected by Claude Code on /clear.
```

### 5.4 Autopilot toggle

- Per-session boolean stored in SQLite
- Default: **OFF** for new sessions (includes fix sessions)
- Exception: **ON** for handoff-restored cycles of a session that was previously ON (so autopilot survives its own cycles)
- UI: toggle in row + in expand panel
- OFF behavior: trigger fires → harness prepares handoff preview → waits for user "Handoff now" click → executes sequence
- ON behavior: trigger fires → sequence runs immediately

### 5.5 Memory vs handoff split

| Layer | Lives at | Written by | Read by | Purpose |
|---|---|---|---|---|
| **Cognitive memory** | Claude Code's native `~/.claude/projects/<hash>/memory/MEMORY.md` + per-memory files | Claude Code itself (via its memory skill) | Auto-injected on every `/clear` | Durable facts, preferences, decisions that persist across all cycles |
| **Operational state** | SQLite in GalvoLite's app data directory | Rust core | Rust core | Session status, heartbeats, PR state, seed queue, autopilot flags — machine state |
| **Handoff** | `.galvolite/handoffs/<session-id>/<timestamp>.md` (in the project repo) | Outgoing Claude session | Incoming cycle of same session | Cycle-specific "what was I just doing" + verbose continuation prompt |

GalvoLite does not duplicate cognitive memory. Claude Code already does that well.

---

## 6. The Decomposition Loop (Brainstorm → Spawn)

### 6.1 Flow

1. User clicks **+ New session → Brainstorm** on a project
2. Harness spawns Claude in the project's main worktree (no branch, no filesystem isolation — brainstorm is discussion, not code changes)
3. Brainstorm CLAUDE.md is injected — encourages opinionated exploration and eventual plan emission
4. User and Claude iterate. When decomposition is ready, Claude writes `.galvolite/plan.md` and emits `[PLAN:READY]`
5. Harness detects the file change + signal, shows plan preview in dashboard
6. User can **edit** `.galvolite/plan.md` directly before spawning
7. User clicks **Spawn sessions** → harness creates worktree + branch + session per plan entry
8. Brainstorm session stays alive (dismissible by user) in case follow-up discussion is needed

### 6.2 `.galvolite/plan.md` format

```markdown
# Plan
## Generated: 2026-04-15T14:32:00Z
## Project: Galvo
## Source brainstorm: session-abc123

---

## Session: phase-b-trust
- **Branch:** feature/phase-b-trust
- **Worktree:** .galvolite/worktrees/phase-b-trust
- **Depends on:** (none)
- **Task:** Implement the trust engine policy evaluator per GALVO_ARCHITECTURE.md §6.2
- **Files:** engine/trust/**, sdk/trust/**, docs/trust/**
- **Initial prompt:**
  >
  > Read GALVO_ARCHITECTURE.md §6.2 and engine/trust/README.md.
  > Build the policy evaluator as specified. Tests in engine/trust/tests/.
  > Commit incrementally. Call `status_report({kind: 'complete'})` when branch is mergeable.

## Session: phase-d-a2a
- **Branch:** feature/phase-d-a2a
- **Worktree:** .galvolite/worktrees/phase-d-a2a
- **Depends on:** phase-b-trust
- **Task:** A2A routing fabric — inter-org message routing with mediation hooks
- **Files:** engine/a2a/**, sdk/a2a/**
- **Initial prompt:**
  > ...

---

## Execution
1. phase-b-trust starts immediately
2. phase-d-a2a waits for phase-b-trust merge
```

Dependencies are explicit. Harness topologically sorts and spawns ready sessions.

### 6.3 Why always gate before spawn

Claude's decomposition is often wrong on first pass. Human review + the ability to edit `plan.md` before spawning is strictly required for v1. No auto-spawn, ever.

---

## 7. The Review Loop (PR → Copilot → Merge/Fix)

### 7.1 Flow

1. Implementation session calls `status_report({kind: 'complete'})`
2. Harness verifies: clean working tree, tests pass (optional check), branch pushed
3. Harness auto-creates PR via `POST /repos/{owner}/{repo}/pulls`
4. GitHub's existing Copilot auto-review (already configured by user) runs
5. Harness polls via GraphQL `reviewThreads` every 30s
6. Findings surfaced in dashboard (raw, no triage)
7. **If no issues:** one big `Merge` button
8. **If issues:** `Show issues` → list, each with:
    - **Fix with Copilot** — POST to GitHub API to trigger Copilot's suggested fix commit
    - **Spawn fix session** — creates a new Implementation session on the branch with an initial prompt containing the specific comment(s)
9. `Merge` → `PUT /repos/{owner}/{repo}/pulls/{n}/merge` with configured strategy
10. On merge: check dependencies in `plan.md`, unblock and spawn any downstream sessions

### 7.2 Why no per-issue triage or severity

User is the judge. UI is a flat list of Copilot's raw comments. Avoids building a triage subsystem in v1.

### 7.3 Fix session lifecycle

A fix session is just an Implementation session with:
- `role = fix`
- `parent_session_id = <original>`
- Branch = the PR's branch. Reuses the original session's worktree if still present on disk; otherwise harness recreates the worktree from the branch.
- Initial prompt = "Address these Copilot review comments on PR #N: [comments...]"

After completion, harness pushes the new commit(s) and requests re-review. The PR is the same PR — no new PR is created for fixes.

---

## 8. Data Model

### 8.1 SQLite (operational state — GalvoLite owns)

Tables (approximate, final schema TBD in implementation):

- `projects` (id, name, path, remote_url, master_folder, created_at)
- `sessions` (id, project_id, role, branch, worktree_path, pty_pid, state, autopilot, parent_session_id, task_summary, created_at)
- `cycles` (id, session_id, started_at, ended_at, handoff_path)
- `signals` (id, session_id, cycle_id, type, payload, at)
- `pull_requests` (id, session_id, number, state, copilot_review_state, merged_at)
- `copilot_findings` (id, pr_id, path, line, body, author, resolved)
- `seeds` (id, project_id, source_session_id, title, priority, state, created_at)
- `scout_reports` (id, seed_id, session_id, summary, recommendation, created_at)

### 8.2 Session state machine

```
spawning → working → [context_pressure] → handing_off → working (next cycle)
                  ↓
           attention_needed → working (after reply)
                  ↓
              complete → pr_creating → pr_open → review_pending
                                              ↓
                                          merged | fixing | dismissed
```

Terminal states: `merged`, `dismissed`, `error`, `killed`.

### 8.3 Coordination protocol (MCP tools + hooks)

> **History:** v0.x parsed bracketed signals (`[STATUS:WORKING]`, `[HANDOFF:READY]`,
> etc.) from pty stdout. That pipeline (`SignalPump`, `src/signals.rs`,
> `signals` table) was deleted in commit `e32ad8d` (Sub-plan 5 task 12). The
> stdout reader now only feeds the scrollback buffer; coordination flows
> through MCP tool calls (authoritative) plus Claude Code hooks (lifecycle
> markers like turn boundaries).

Sessions coordinate with GalvoLite by calling MCP tools on the per-session
`/mcp` endpoint (bearer-token auth via the session's `api_token`). The full
tool surface is documented in each role's `CLAUDE.md` profile
(`src/claude_md.rs`); the headline tools and their replaced-signals are:

| MCP tool | Replaces signal | Notes |
|---|---|---|
| `status_report({kind: 'working'\|'complete'\|'blocked', note?})` | `[STATUS:*]` | Heartbeat + state transitions; drives `SessionState` machine. |
| `request_attention({reason})` | `[ATTENTION:NEEDED]` / `[ATTENTION:CONTEXT]` | Operator notification + dashboard pin. |
| `write_handoff({summary, next_steps})` | `[HANDOFF:READY]` / `[HANDOFF:BLOCKED]` | Persists `.galvolite/handoffs/<sid>/<ts>.md`; status reported via `kind`. |
| `set_plan({markdown})` + `plan_check_step({...})` | `[PLAN:READY]` | Populates `plan_steps` table; emits `SessionPlanReplaced` / `SessionPlanStepChanged` to dashboard. |
| `set_role({role})` | `[ROLE:*]` | Mid-session role transition; updates `sessions.role` + reinjects profile if applicable. |
| `spawn_session({role, task})` + `wait_children({children})` | (new) | Sub-plan 6 fan-out + idle-rejoin; not in v0.x signal protocol. |
| `notify_operator({level, message})` / `ask_operator({question, choices?})` | (new) | Operator-loop primitives. |

Lifecycle markers (turn boundaries, prompt submit, etc.) come from Claude
Code hooks via `POST /hooks/ingest` rather than from stdout parsing.

---

## 9. Filesystem Layout

```
<master-folder>/                         # user-configured, e.g. ~/code
├── Galvo/                               # 1 folder = 1 project = 1 git repo
│   ├── .git/
│   ├── .galvolite/
│   │   ├── plan.md                      # decomposition manifest
│   │   ├── worktrees/
│   │   │   ├── phase-b-trust/           # git worktree
│   │   │   └── phase-d-a2a/
│   │   ├── handoffs/
│   │   │   └── <session-id>/
│   │   │       └── 2026-04-15T14-32-00.md
│   │   └── seeds/
│   │       ├── SEED-001-connection-drain.md
│   │       └── SEED-002-rotate-tokens.md
│   └── ... (project source)
├── GalvoLite/
│   └── ... (same structure)
└── Prizm/
    └── ...
```

GalvoLite's own app data (SQLite, config, OAuth tokens) lives in OS-standard app data dir (e.g., `~/Library/Application Support/GalvoLite/` on macOS), not in a project folder.

`.galvolite/` should be added to `.gitignore` or committed selectively per user preference — plan.md and seeds/ are useful to commit; worktrees/ and handoffs/ probably not.

---

## 10. CLAUDE.md Profiles

Three profiles, all sharing the core signal protocol, file ownership rules, and handoff protocol. Differences summarized:

### 10.1 Brainstorm (doubles as Planner)

- Verbose, opinionated, exploratory tone with user
- Terse to harness
- Can write `.galvolite/plan.md` when decomposition is ready
- Can write seed files when out-of-scope ideas arise
- Emits `[PLAN:READY]` when plan is complete and self-consistent
- Does not write code changes

### 10.2 Implementation

- Terse mode default for everything except `[ATTENTION:NEEDED]` messages
- Exclusive file ownership (defined in plan.md)
- Works on assigned branch in assigned worktree — never switches
- Writes verbose, self-contained continuation prompt on handoff (this is the single most important property of the profile — the next cycle's success depends entirely on this prompt's quality)
- Emits `[STATUS:COMPLETE]` only when: clean tree, tests pass, branch pushed
- Can write seed files; does NOT investigate seeds beyond writing them

### 10.3 Scout

- Short-lived, single-response typically
- Reads a seed, investigates via file reads + grep
- Writes report to `.galvolite/seeds/REPORT-SEED-NNN.md`
- Emits `[STATUS:COMPLETE]` when report written
- Does not modify source code, does not create branches

### 10.4 Shared: the "verbose handoff" rule

CLAUDE.md for every profile includes explicit instructions:

> When writing the Continuation Prompt section of your handoff, be **reasonably verbose**. The next cycle reads only the handoff — it has no memory of this conversation. Include: exact file paths and line numbers of in-progress changes, decisions made and why, rejected alternatives and why, the specific next step to take, and anything non-obvious about the task. Err on the side of more context. The cost of verbosity on handoff is trivial compared to the cost of the next cycle being underbriefed.

---

## 11. Seeds Subsystem

### 11.1 Lifecycle

```
Written → Unassessed → (Scouted) → (Accepted | Declined)
                                  → Promoted to brainstorm → Tasked → Done
```

### 11.2 Seed file format

```markdown
# Seed: <title>
## ID: SEED-NNN
## Created: 2026-04-15
## Source: implementation/phase-b-trust (session-abc123)
## Priority: medium

### Context
<what was I doing when this came up>

### Observation
<what did I notice>

### Proposed Action
<what should be done>

### Scope Impact
- Affects: <parts of system>
- Blocking: no
- Urgency: next cycle

### Related Files
- engine/trust/policy.rs — where the evaluator currently hardcodes X
```

### 11.3 UI actions

- **Scout it** — spawns a Scout session. Writes REPORT-SEED-NNN.md next to the seed.
- **Promote to brainstorm** — pastes the seed + optional report into a new Brainstorm session
- **Archive** — state → declined; stays on disk for history

### 11.4 UI panels

- Per-project Seeds panel (filter of all seeds in that project)
- Global Seeds panel (all projects, useful when Michael notices cross-project patterns)

---

## 12. Dashboard UX

### 12.1 Primary view

One row per running Claude instance. Grouped by project. Filterable.

Per-row fields (left to right):
- Status dot (green=working, yellow=attention, blue=handoff, purple=PR, gray=idle, red=error)
- Task summary + branch name
- Cycle counter (e.g., "cycle 2/3")
- Last signal badge
- Heartbeat age
- Autopilot on/off pill
- Miniature live xterm.js preview (read-only, ~last N lines, scale ~0.4)

### 12.2 Miniature preview interaction

- `onmouseenter` + 3-second dwell → CSS transform expand to 80vw × 80vh (centered overlay)
- `onclick` on expanded preview → enable stdin, same terminal becomes interactive; user can type directly
- `onmouseleave` (with no click) → collapse back to miniature
- Avoids accidental expansions; preserves click intent for interactivity

### 12.3 Expand panel (hover-only, non-interactive preview)

When hovering a row without clicking, a separate expand panel shows (not just the mini terminal):
- Task header + latest signal
- Tail of recent output
- Worktree path, last commit, tool-call count, threshold
- Action buttons: Reply inline, Open terminal, Handoff now, Toggle autopilot, View plan.md, Kill session

### 12.4 Other views

- **Global view:** all sessions across all projects (unfiltered)
- **Project view:** filter to one project
- **PRs panel:** per-project list of open/recent PRs with Copilot findings and Merge/Show issues actions
- **Seeds panel:** per-project + global
- **Settings:** OAuth status, master folder picker, merge strategy, heartbeat timeout, etc.

### 12.5 Attention surfacing

- Row signal badge turns yellow with icon
- Top-bar count increments
- OS notification via Tauri
- No separate attention queue — just sort/filter the row list by state

---

## 13. Tech Stack

| Layer | Tech | Rationale |
|---|---|---|
| Desktop shell | Tauri | Native app feel, single binary install, Rust-native |
| Core daemon | Rust + tokio + axum | Process mgmt, async, pty control, GitHub API |
| HTTP client | reqwest | GitHub REST |
| GraphQL | reqwest + manual queries (or cynic/graphql-client) | Review thread resolution state |
| Pty control | portable-pty or pty-process | Cross-platform pty for claude processes |
| Database | SQLite (rusqlite or sqlx) | Operational state persistence |
| Config | TOML | User config file |
| Secrets | OS keychain (keyring crate) | OAuth tokens, not in plaintext |
| File watching | notify crate | Detect plan.md / seed file changes |
| Frontend | React + Vite + TailwindCSS | Fast iteration, matches galvo-dashboard style |
| Terminal in browser | xterm.js | Live pty streaming for mini + expanded preview |
| Packaging | Tauri bundler | DMG for macOS; cross-platform later |
| Claude Code plugin | JS (or whatever Claude Code plugin SDK requires) | PreCompact + tool-call counter |

---

## 14. GitHub Integration

### 14.1 Auth

- GitHub OAuth App (device flow, RFC 8628)
- Scopes: `repo`, `read:user`
- Token stored in OS keychain
- First-launch onboarding shows device code + `github.com/login/device` link

### 14.2 Remote UI auth

- Local Tauri webview: implicit trust (same machine)
- Remote browser over Tailscale: on first connection from an unknown Tailscale IP, the harness generates a one-time device code shown only in the local Tauri window. User enters the code in the remote browser to establish a long-lived session cookie for that IP. Cookie cleared or IP changed → new device-code handshake required.
- Non-Tailscale network interfaces: HTTP bind refuses by default; must be explicitly enabled in config.

### 14.3 APIs used (v1)

- `POST /user/repos` / `GET /user/repos` (project discovery / import)
- `POST /repos/{owner}/{repo}/git/refs` (create branch)
- `POST /repos/{owner}/{repo}/pulls` (create PR)
- `GET /repos/{owner}/{repo}/pulls/{n}` (PR state)
- `GraphQL reviewThreads` (Copilot findings with resolution state)
- `PUT /repos/{owner}/{repo}/pulls/{n}/merge` (merge)
- Copilot fix-suggestion API (TBD exact endpoint — research in plan phase)

---

## 15. Setup / Onboarding

1. Install GalvoLite.app
2. First launch:
   - OAuth device flow → paste code at github.com/login/device
   - Master folder picker (default: `~/code` or current working dir)
3. Main view: dashboard with auto-discovered projects (any `.git/` directory in the master folder)
4. Per-project first-time: GalvoLite initializes `.galvolite/` subdir on first session creation
5. Settings view: change master folder, edit config, re-auth, view plugin install status
6. Plugin install: one-click "Install Claude Code plugin" action in settings; writes to Claude Code's config + installs the hook

---

## 16. Implementation Phasing (high-level)

Detailed plan will be produced by the `writing-plans` skill. Anticipated phase breakdown:

1. **Core skeleton** — Rust project + Tauri shell + SQLite schema + basic frontend
2. **Session manager** — pty control, process spawning, signal router, state machine
3. **Claude Code plugin** — PreCompact + tool-call counter, emits context-pressure signal
4. **Ritual driver** — handoff file write/read, /clear keystroke, restore prompt
5. **Brainstorm → plan.md flow** — profile injection, file watcher, preview UI
6. **Spawn engine** — worktree creation, branch creation, session spawn from plan
7. **GitHub integration** — OAuth device flow, REST + GraphQL clients
8. **PR pipeline** — auto-create, review polling, findings surfacing
9. **Per-issue actions** — Fix with Copilot API, Spawn fix session
10. **Seeds subsystem** — file watcher, UI panel, Scout profile + sessions
11. **Dashboard UX polish** — miniature xterm.js, hover-expand, interactivity, notifications
12. **Remote UI auth** — Tailscale-compatible token cookie flow
13. **Installer / distribution** — Tauri bundler, code signing

---

## 17. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Claude Code plugin API not suitable for reliable context-pressure detection | Primary trigger unreliable | Fallback to tool-call counter via stdout parse; user can always click "Handoff now" |
| Handoff continuation prompt quality is bad → drift | User has to re-brief manually (B priority degraded) | Iterate on CLAUDE.md wording; possibly add a quality check that reads the continuation prompt and warns if too short |
| xterm.js at 0.4 scale unreadable | Tiny preview useless | Fall back to last-3-lines text summary in the row; full xterm only in expanded view |
| Tailscale auth UX painful | Remote monitoring friction | Keep local-only mode fully functional; remote access is a nice-to-have |
| GitHub Copilot API changes | Review pipeline breaks | Abstract Copilot calls behind a trait; alternative reviewers can replace later |
| Git worktree edge cases (uncommitted changes, branch conflicts) | Spawn fails | Clear error surfacing; retry guidance; worst case user resolves manually |
| Users install without the Claude Code plugin | Only manual + Claude self-signal triggers | Plugin install prompt on first session; warning banner if plugin missing |

---

## 18. Success Criteria (v1)

- User can open GalvoLite, OAuth once, see auto-discovered projects
- User can start a brainstorm session, iterate with Claude, get `.galvolite/plan.md` written
- User can review + edit the plan and click "Spawn sessions" — N Claude sessions start in N worktrees on N branches
- Save/clear/restore ritual runs losslessly — session can run through 3+ cycles with continuity; restored cycle starts work *inside* the task, not at re-onboarding
- On completion, PR is auto-created, Copilot review fires (via user's existing GitHub config), findings surface in dashboard
- User can click Fix-with-Copilot per issue or Spawn-fix-session; merge when clean
- Autopilot-ON sessions run the ritual with zero user input for basic context-pressure triggers
- Seeds written mid-session appear in the seeds panel; Scout sessions evaluate them; promote-to-brainstorm works end-to-end
- Remote browser monitoring over Tailscale works
- A typical day: user opens dashboard, sees the fleet, replies to attention signals, merges clean PRs, spawns fixes for dirty ones — all without running a single manual `/clear` or save-to-memory command

---

## Appendix A: Signal Protocol Quick Reference

```
[STATUS:WORKING]                         # heartbeat
[STATUS:COMPLETE]                        # done, tests pass, branch pushed
[ATTENTION:NEEDED reason="..."]          # needs human; provide context on subsequent lines
[ATTENTION:CONTEXT]                      # self-sensed pressure (safety net)
[HANDOFF:READY]                          # handoff file written and validated
[HANDOFF:BLOCKED reason="..."]           # cannot complete handoff
[PLAN:READY]                             # .galvolite/plan.md written (brainstorm only)
[SEED:NEW id="SEED-NNN"]                 # seed file created
```

Signals must appear on their own line. Never inside code blocks, comments, or quoted text.

---

## Appendix B: Relationship to the v3 PRD

This spec supersedes `LUCIDITY-ORCHESTRATOR-PRD-v3.md` for the Lite product scope.

Preserved from v3:
- Signal protocol (with additions)
- Handoff-over-compaction philosophy
- CLAUDE.md-as-behavioral-contract pattern
- Seeds concept and lifecycle
- tmux/pty + worktree combo for filesystem isolation

Simplified for Lite:
- Four profiles → three (Orchestrator merged into Brainstorm)
- Two AIs → one (Claude only; Copilot lives on GitHub, not in the harness)
- Team/member structure → flat per-project sessions
- Triage + severity + selective-fix UX → raw-comment list with per-issue buttons
- Rust+React+web-daemon → Tauri (UI unified) + Rust core (still serves HTTP for remote access)
- OAuth App + PAT choice → OAuth App only

Inherited unchanged:
- GitHub as quality gate
- Handoff file format (nearly identical)
- Plan manifest concept (renamed from teams.md to plan.md, scoped to `.galvolite/`)

---

*End of spec.*
