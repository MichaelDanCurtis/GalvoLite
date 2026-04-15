# Lucidity Orchestrator — Product Requirements Document

**Author:** Michael Curtis
**Version:** 0.3 — Revised
**Date:** April 2026
**Status:** Design Phase

---

## 1. Overview

The Lucidity Orchestrator is a GitHub-native development supervisor that manages the full lifecycle of multi-instance AI coding workflows: planning, task decomposition, parallel execution via Claude Code, PR creation, automated review via GitHub Copilot, and human-gated merge — all through a unified web frontend.

It authenticates as a registered GitHub OAuth App, giving it first-class API access to repositories, pull requests, reviews, and branches. It uses the GitHub Copilot SDK for lightweight coordination intelligence (collision detection, handoff validation, review triage) while reserving Claude Code for the heavy coding work.

The system operates in three phases:

- **Phase A — Planning**: A "superpowers" Claude Code session analyzes the PRD, estimates sessions needed, decomposes work into non-colliding teams, and produces a `teams.md` manifest.
- **Phase B — Execution**: The orchestrator spawns Claude Code instances per `teams.md`, each on its own branch with a CLAUDE.md behavioral contract. Instances work, signal, hand off on context pressure, and signal completion.
- **Phase C — Review & Merge**: The orchestrator creates PRs via the GitHub API, requests Copilot review, surfaces findings in the frontend, and gives the human options: fix (via Claude Code), merge, or view in GitHub.

### What Makes This Different

Unlike Gas Town, multiclaude, or Claude Code Agent Teams, the Lucidity Orchestrator is **not primarily a parallel agent framework**. It is a **project lifecycle manager** that:

- Starts with planning and task decomposition before any code is written
- Enforces file ownership boundaries so parallel work doesn't collide
- Uses GitHub's PR/review system as the quality gate, not just CI
- Provides a web frontend for managing the entire workflow, not just a tmux dashboard
- Uses the right AI for each job: Claude Code for coding, Copilot models for coordination

---

## 2. Problem Statement

Building non-trivial features with Claude Code today is a manual orchestration task. The developer is the context manager, the reviewer, the process scheduler, and the merge authority — all at once, all manually.

The Lucidity Orchestrator reduces the human role to three things:

- **Directing**: Provide a PRD and approve the task decomposition
- **Deciding**: Answer questions when instances are blocked, review findings
- **Merging**: Approve PRs when they're clean

Everything else — session management, context handoffs, branch management, PR creation, review surfacing, collision detection — is automated.

---

## 3. Architecture

### 3.1 System Diagram

```
┌──────────────────────────────────────────────────────────────────┐
│                     Lucidity Orchestrator                        │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │                    Web Frontend (React)                     │  │
│  │  ┌──────────┐ ┌──────────┐ ┌───────────┐ ┌─────────────┐ │  │
│  │  │ Project  │ │ Session  │ │    PR &   │ │  Attention  │ │  │
│  │  │ Manager  │ │ Dashboard│ │  Review   │ │    Queue    │ │  │
│  │  │          │ │          │ │  Manager  │ │             │ │  │
│  │  └──────────┘ └──────────┘ └───────────┘ └─────────────┘ │  │
│  └────────────────────────┬───────────────────────────────────┘  │
│                           │ REST/WebSocket                       │
│  ┌────────────────────────┴───────────────────────────────────┐  │
│  │                  Backend (Rust + tokio + axum)              │  │
│  │                                                             │  │
│  │  ┌─────────────────────────────────────────────────────┐   │  │
│  │  │           GitHub Integration Layer                   │   │  │
│  │  │  ┌──────────────┐  ┌─────────────────────────────┐  │   │  │
│  │  │  │ GitHub REST  │  │    Copilot SDK (Go/FFI)     │  │   │  │
│  │  │  │ + GraphQL    │  │                             │  │   │  │
│  │  │  │ (OAuth token)│  │  - Collision detection      │  │   │  │
│  │  │  │              │  │  - Handoff validation       │  │   │  │
│  │  │  │ - Repos      │  │  - Review triage            │  │   │  │
│  │  │  │ - Branches   │  │  - PR descriptions          │  │   │  │
│  │  │  │ - PRs        │  │  - Coordination queries     │  │   │  │
│  │  │  │ - Reviews    │  │                             │  │   │  │
│  │  │  │ - Merges     │  │                             │  │   │  │
│  │  │  └──────────────┘  └─────────────────────────────┘  │   │  │
│  │  └─────────────────────────────────────────────────────┘   │  │
│  │                                                             │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐                 │  │
│  │  │ Session  │  │ Session  │  │ Session  │  ...             │  │
│  │  │ Manager  │  │ Manager  │  │ Manager  │                 │  │
│  │  │ tmux:w1  │  │ tmux:w2  │  │ tmux:w3  │                 │  │
│  │  └────┬─────┘  └────┬─────┘  └────┬─────┘                 │  │
│  │       └──────────────┴──────────────┘                       │  │
│  │                Signal Router                                │  │
│  │           (stdout monitor + state machine)                  │  │
│  └─────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
```

### 3.2 GitHub Integration Layer

The orchestrator authenticates as a registered GitHub OAuth App. On first launch, the user completes the OAuth device flow (no browser redirect needed for a CLI/local app). The resulting `gho_xxx` token is stored securely and refreshed automatically.

**Required OAuth scopes:** `repo` (full repo access including private repos, branches, PRs, reviews, merges, and repo creation).

#### REST API Operations

All GitHub operations go through `api.github.com` directly — no `gh` CLI dependency.

| Operation | Endpoint | Method |
|-----------|----------|--------|
| Create repo | `POST /user/repos` or `POST /orgs/{org}/repos` | POST |
| Create branch | `POST /repos/{owner}/{repo}/git/refs` | POST |
| Create PR | `POST /repos/{owner}/{repo}/pulls` | POST |
| Request Copilot review | `POST /repos/{owner}/{repo}/pulls/{n}/requested_reviewers` with `{"reviewers":["copilot"]}` | POST |
| List reviews | `GET /repos/{owner}/{repo}/pulls/{n}/reviews` | GET |
| Get review comments | `GET /repos/{owner}/{repo}/pulls/{n}/comments` | GET |
| Merge PR | `PUT /repos/{owner}/{repo}/pulls/{n}/merge` | PUT |
| Get PR status | `GET /repos/{owner}/{repo}/pulls/{n}` | GET |
| List PRs | `GET /repos/{owner}/{repo}/pulls` | GET |
| Get repo | `GET /repos/{owner}/{repo}` | GET |
| List repos | `GET /user/repos` | GET |

#### GraphQL Operations

For review thread resolution status (not available in REST):

```graphql
{
  repository(owner: "{owner}", name: "{repo}") {
    pullRequest(number: {n}) {
      reviewThreads(first: 100) {
        nodes {
          isResolved
          comments(first: 100) {
            nodes {
              databaseId
              body
              path
              line
              author { login }
            }
          }
        }
      }
    }
  }
}
```

This single query returns all review threads, their resolution status, and every comment with file path and line number. Filter by `author.login` for Copilot-specific comments.

#### Copilot SDK Integration

The same OAuth token powers the Copilot SDK for coordination intelligence. The orchestrator uses Copilot models (via the Go SDK, called from Rust via FFI or as a sidecar process) for lightweight tasks:

| Task | What Copilot Does |
|------|-------------------|
| File ownership collision detection | Given two `teams.md` member definitions, do their file paths overlap? |
| Attention triage | An instance needs to modify an unowned file. Does this conflict with active work? |
| Handoff validation | Is this handoff entry complete? Is the continuation prompt self-contained? |
| PR description generation | Given a diff and task assignment, write a meaningful PR description. |
| Review comment triage | Classify Copilot review comments by severity. Which need code changes vs. informational? |
| `teams.md` validation | Given a PRD, does this task decomposition have gaps or overlaps? |

This keeps Claude Code focused on actual coding. Copilot handles coordination at subscription cost, not per-token cost.

### 3.3 Session Manager

Each Claude Code session is a state machine managed by the orchestrator:

**States:**

| State | Description |
|-------|-------------|
| `Spawning` | Branch created, tmux pane created, `claude` process starting, CLAUDE.md + prompt injected. |
| `Working` | Instance actively working. Heartbeat signals expected. |
| `NeedsAttention` | Instance emitted `[ATTENTION:NEEDED]`. Surfaced in frontend. |
| `ContextPressure` | Instance emitted `[ATTENTION:CONTEXT]`. Handoff initiated. |
| `HandingOff` | Handoff instruction injected. Waiting for `[HANDOFF:READY]`. |
| `HandedOff` | Handoff complete. Session will be cleared and respawned. |
| `Complete` | Task done, branch pushed. PR being created. |
| `PrCreated` | PR opened, Copilot review requested. Polling for results. |
| `PendingReview` | Review complete. Surfaced in frontend with action options. |
| `Fixing` | Fix instance spawned to address review findings. |
| `Merged` | PR merged. Terminal state. |
| `Dismissed` | PR left open. |
| `Stale` | No heartbeat for timeout period. Surfaced to human. |
| `Error` | Process crashed or unexpected state. |

### 3.4 Signal Protocol

Defined in CLAUDE.md. Machine-parseable signals on their own line:

```
[STATUS:WORKING]                        → Reset heartbeat timer
[STATUS:COMPLETE]                       → Verify branch pushed, begin PR creation
[ATTENTION:NEEDED reason="..."]         → Queue for human attention
[ATTENTION:CONTEXT]                     → Initiate handoff sequence
[HANDOFF:READY]                         → Clear and respawn
[HANDOFF:BLOCKED reason="..."]          → Surface to human
```

### 3.5 PR & Review Pipeline

On `[STATUS:COMPLETE]`:

1. Orchestrator verifies clean git state on the branch
2. Uses Copilot SDK to generate a PR description from the diff + task assignment
3. Creates PR via `POST /repos/{owner}/{repo}/pulls`
4. Requests Copilot review via `POST /repos/{owner}/{repo}/pulls/{n}/requested_reviewers`
5. Polls review status via GraphQL `reviewThreads` query (30-second interval, 5-minute timeout)
6. When review is ready, uses Copilot SDK to triage comments by severity
7. Surfaces findings in the frontend with action options

On "Fix all" or "Fix selective":
1. Spawn a new Claude Code instance on the branch with a prompt listing selected issues
2. On completion, re-request Copilot review
3. Cycle until clean or human merges

On "Merge":
1. `PUT /repos/{owner}/{repo}/pulls/{n}/merge` with configured strategy
2. If dependent tasks in `teams.md` are waiting, unblock them

---

## 4. Web Frontend

The frontend is a React application served by the Rust backend via axum. It communicates with the backend over REST for operations and WebSocket for real-time updates (session state changes, new review results, attention events).

### 4.1 Views

#### Project View

The landing page. Shows all projects (repos) the orchestrator is managing.

- **Create new project**: Name, description, visibility (public/private), org (optional). Calls `POST /user/repos` or `POST /orgs/{org}/repos`.
- **Import existing project**: Select from user's repos (listed via `GET /user/repos`).
- **Project cards**: Repo name, active sessions count, open PRs count, attention items count.

Clicking a project opens the Project Dashboard.

#### Project Dashboard

The main working view for a project. Three panels:

**Sessions Panel:**
```
┌─ Sessions ──────────────────────────────────────────────────────┐
│ Member                Team        State          Heartbeat  Cyc │
│ backend-auth          Backend     ● Working      8s ago     1/2 │
│ backend-db            Backend     ⚠ Needs Input  42s ago    1/1 │
│ frontend-dashboard    Frontend    ↻ Handing Off  1s ago     2/2 │
│ frontend-settings     Frontend    ✓ PR Created   —          1/1 │
│                                                                  │
│ [+ New Session]  [↗ Open teams.md]  [⟳ Refresh]                │
└──────────────────────────────────────────────────────────────────┘
```

**Pull Requests Panel:**
```
┌─ Pull Requests ─────────────────────────────────────────────────┐
│ PR #47  feature/backend-db → main                               │
│   Copilot: 3 issues (1 medium, 1 low, 1 style)                 │
│   [Fix All]  [Fix Selected]  [View in GitHub]  [Merge]  [Skip] │
│                                                                  │
│ PR #48  feature/frontend-settings → main                        │
│   Copilot: clean ✅                                              │
│   [View in GitHub]  [Merge]                                      │
│                                                                  │
│ PR #46  feature/backend-auth → main    ✓ Merged                 │
└──────────────────────────────────────────────────────────────────┘
```

Each PR expands to show Copilot's inline comments with file paths and line numbers. The user can check/uncheck individual issues for selective fixing.

**Attention Queue Panel:**
```
┌─ Attention Queue ───────────────────────────────────────────────┐
│ ⚠ backend-db (Session 2):                                       │
│   "Should the pool health check interval be configurable        │
│    or hardcoded?                                                 │
│    Option A: config (flexible, more code)                        │
│    Option B: hardcoded 30s (simple, usually fine)"              │
│   [Open Terminal]  [Reply Inline]                                │
│                                                                  │
│ ℹ frontend-dashboard (Session 3):                               │
│   Context pressure — handoff in progress                         │
└──────────────────────────────────────────────────────────────────┘
```

"Open Terminal" attaches to that session's tmux pane in a terminal emulator embedded in the frontend (via xterm.js) or opens a local terminal.

"Reply Inline" lets the user type a response directly in the frontend, which is injected into the session's stdin.

#### PR Detail View

Opened by clicking a PR. Shows:

- PR title, description, branch, diff stats
- Copilot review comments, grouped by file, with severity badges
- Checkboxes for selective fixing
- Action buttons: Fix All, Fix Selected, View in GitHub, Merge, Dismiss
- Fix history: previous fix attempts and their results

#### Settings View

- GitHub OAuth: connected account, token status, re-authenticate
- Claude Code: path to CLI, default flags
- Orchestrator: max sessions, heartbeat timeout, handoff timeout
- Git: merge strategy, base branch, auto-create PR setting

### 4.2 Real-Time Updates

The frontend connects to the backend via WebSocket. Events pushed to the client:

| Event | Trigger | UI Update |
|-------|---------|-----------|
| `session_state_changed` | Any session state transition | Update session row |
| `attention_needed` | `[ATTENTION:NEEDED]` signal | Add to attention queue, notification |
| `pr_created` | PR created via API | Add to PR panel |
| `review_complete` | Copilot review results ready | Update PR with findings |
| `pr_merged` | PR merged | Update PR status, check dependencies |
| `handoff_started` | Context pressure detected | Update session row |
| `handoff_complete` | New session spawned | Update session row |
| `session_error` | Process crash or timeout | Alert in attention queue |

### 4.3 Repo Management

The frontend provides basic GitHub repo management, enough to bootstrap a project without leaving the orchestrator:

- **Create repo**: Name, description, visibility, org, .gitignore template, license
- **View repo contents**: File tree browser (read-only, via Contents API)
- **Create/upload files**: Push initial PRD, CLAUDE.md, teams.md to the repo
- **Branch management**: List branches, create branches, view branch protection rules
- **PR management**: List open/closed PRs, view diffs, merge, close

This is not a full GitHub replacement — it's a focused subset for managing the orchestration workflow without context-switching to the browser.

---

## 5. Workflow Detail

### 5.1 Phase A — Planning

1. User creates or imports a project in the frontend.
2. User starts a "superpowers" Claude Code session (interactive brainstorming).
3. User and Claude define the architecture and scope.
4. User instructs Claude to generate `teams.md` with the standard format (see Appendix A).
5. Orchestrator validates `teams.md`:
   - Uses Copilot SDK to check for file ownership overlaps
   - Verifies branch names are unique
   - Checks dependency graph for cycles
6. User reviews and approves `teams.md` in the frontend.

### 5.2 Phase B — Execution

1. Orchestrator reads `teams.md` and resolves dependencies (topological sort).
2. For each ready team member:
   a. Create git branch via API (`POST /repos/{owner}/{repo}/git/refs`)
   b. Spawn Claude Code session in tmux pane on that branch
   c. Inject CLAUDE.md behavioral contract
   d. Inject member's initial prompt from `teams.md`
3. Monitor for signals:
   - `[STATUS:WORKING]` → reset heartbeat timer
   - `[ATTENTION:NEEDED]` → surface in frontend attention queue
   - `[ATTENTION:CONTEXT]` → trigger handoff sequence
   - `[STATUS:COMPLETE]` → begin PR creation
4. On handoff: validate handoff entry, clear session, respawn with continuation prompt.
5. On completion: verify branch is pushed, proceed to Phase C.

### 5.3 Phase C — Review & Merge

1. Generate PR description via Copilot SDK.
2. Create PR via `POST /repos/{owner}/{repo}/pulls`.
3. Request Copilot review via `POST /repos/{owner}/{repo}/pulls/{n}/requested_reviewers` with `copilot`.
4. Poll for review via GraphQL `reviewThreads` query.
5. Triage review comments via Copilot SDK (classify severity).
6. Surface in frontend with action options.
7. If "Fix all/selective": spawn Claude Code fix instance with issue list.
8. If "Merge": `PUT /repos/{owner}/{repo}/pulls/{n}/merge`.
9. On merge: check `teams.md` dependencies, unblock waiting tasks.
10. If re-review needed after fix: `POST .../requested_reviewers` again, or post `/review` comment as fallback.

---

## 6. CLAUDE.md Behavioral Contract

See companion document: `CLAUDE-ORCHESTRATOR-TEMPLATE-v2.md`

Defines signal protocol, handoff format, file ownership rules, git workflow, and error recovery procedures.

---

## 7. Plugin Specification

The orchestrator plugin extends `thepushkarp/handoff` for clear-based handoffs.

**Hooks:**

| Hook | Trigger | Behavior |
|------|---------|----------|
| `PreCompact` | Before compaction | Write handoff skeleton with git status, branch, recent commits. |
| `SessionStart` | Session start | Inject latest handoff Continuation Prompt + CLAUDE.md. |
| `SessionEnd` | Session end | Emergency handoff if none exists. Write pointer file. |
| `Stop` | Model tries to stop | Block if handoff TODOs unfilled. Max 3 retries. |
| `UserPromptSubmit` | Every prompt | Increment counter. Inject context warning at threshold. |
| `PostToolUse(Bash)` | After bash commands | Detect `git push`, notify orchestrator. |

---

## 8. Configuration

```toml
[orchestrator]
max_sessions = 8
heartbeat_timeout_secs = 300
handoff_timeout_secs = 120
signal_poll_interval_ms = 2000

[github]
# OAuth App credentials (registered at github.com/settings/developers)
oauth_client_id = "Iv1.xxxxxxxxxxxxxxxx"
oauth_client_secret = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
# Token is obtained via device flow at first launch, stored in keychain
# token_path = "~/.lucidity/github_token.json"

[github.defaults]
auto_create_pr = true
auto_request_copilot_review = true
pr_merge_strategy = "squash"         # squash | merge | rebase
base_branch = "main"
copilot_review_poll_interval_secs = 30
copilot_review_timeout_secs = 300

[copilot_sdk]
# Uses the same OAuth token as github section
model = "gpt-4.1"                    # Model for coordination tasks
# Alternatively, use BYOK for coordination:
# byok_provider = "anthropic"
# byok_api_key_env = "ANTHROPIC_API_KEY"
# byok_model = "claude-sonnet-4-6-20250514"

[claude]
command = "claude"
default_flags = []
claude_md_path = "CLAUDE.md"

[planning]
teams_file = "teams.md"
respect_dependencies = true

[handoff]
file_path = "docs/handoff/HANDOFF.md"
tool_call_warning_threshold = 100
auto_handoff_on_context_signal = true

[frontend]
port = 3000
host = "127.0.0.1"
```

---

## 9. Implementation Plan

### Phase 1: GitHub Integration Layer (2-3 days)

- [ ] Rust project scaffold (tokio, axum, reqwest, serde, toml)
- [ ] OAuth device flow implementation (RFC 8628)
- [ ] Token storage and refresh
- [ ] GitHub REST client: repos, branches, PRs, reviews, merges
- [ ] GitHub GraphQL client: review threads with resolution status
- [ ] Repo creation (`POST /user/repos`, `POST /orgs/{org}/repos`)

### Phase 2: Copilot SDK Integration (1-2 days)

- [ ] Go SDK sidecar process (or FFI bridge)
- [ ] Coordination tasks: collision detection, handoff validation, review triage
- [ ] PR description generation
- [ ] `teams.md` validation

### Phase 3: Session Management (2 days)

- [ ] tmux management: create/destroy panes, capture-pane, send-keys
- [ ] Process spawning: launch `claude` in tmux with initial prompt
- [ ] Signal router: poll capture-pane, regex match signal protocol
- [ ] Session state machine with all transitions
- [ ] Heartbeat monitoring and stale detection

### Phase 4: Planning & Spawn Engine (1-2 days)

- [ ] `teams.md` parser (teams, members, branches, prompts, dependencies)
- [ ] Dependency resolver (topological sort)
- [ ] Branch creation via API
- [ ] Spawn loop: for each ready member, create branch → spawn → inject
- [ ] File ownership overlap validation (via Copilot SDK)

### Phase 5: Handoff Pipeline (1-2 days)

- [ ] CLAUDE.md injection on session start
- [ ] Context pressure detection → handoff instruction injection
- [ ] Handoff file validation (required fields, no TODOs)
- [ ] Clear → respawn with Continuation Prompt
- [ ] Emergency handoff on unclean exit

### Phase 6: PR & Review Pipeline (2 days)

- [ ] PR creation on `[STATUS:COMPLETE]`
- [ ] Copilot review request and polling
- [ ] Review comment extraction and triage (via Copilot SDK)
- [ ] Fix instance spawning with issue list prompt
- [ ] Re-review after fixes (with `/review` comment fallback)
- [ ] Merge with configurable strategy
- [ ] Dependency unblocking after merge

### Phase 7: Web Frontend (3-4 days)

- [ ] React project scaffold (Vite, TailwindCSS)
- [ ] axum backend: REST endpoints + WebSocket for real-time
- [ ] Project view: list, create, import repos
- [ ] Project dashboard: sessions panel, PR panel, attention queue
- [ ] PR detail view: review comments, severity badges, fix controls
- [ ] Attention queue: inline reply, terminal attach (xterm.js)
- [ ] Repo management: file browser, branch list, basic PR management
- [ ] Settings view: OAuth status, config management
- [ ] Real-time updates via WebSocket

### Phase 8: Plugin (1 day)

- [ ] Fork `thepushkarp/handoff`, add clear-specific hooks
- [ ] Tool-call counter on UserPromptSubmit
- [ ] Emergency handoff on SessionEnd
- [ ] Git push detection on PostToolUse(Bash)
- [ ] End-to-end test: pressure → handoff → clear → restore

### Phase 9: Integration & Polish (2 days)

- [ ] Configuration file parsing and validation
- [ ] Error recovery: crashed sessions, failed handoffs, failed PRs
- [ ] Structured logging
- [ ] README, setup guide, example `teams.md`
- [ ] End-to-end smoke test: plan → spawn → handoff → complete → PR → review → merge

### Total estimate: 15-20 days for a working v1

---

## 10. Dependencies

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Backend language | Rust | Process management, async, safety, performance. |
| Async runtime | tokio | Process spawning, timers, channels, HTTP server. |
| HTTP framework | axum | Serves frontend, REST API, WebSocket. |
| HTTP client | reqwest | GitHub API calls. |
| Frontend | React + Vite + TailwindCSS | Fast, modern, component-based UI. |
| Terminal in browser | xterm.js | Embed tmux session in frontend for session attach. |
| Terminal multiplexer | tmux | Session isolation, pane management, capture-pane. |
| Claude Code | CLI | The managed coding agent. |
| Copilot SDK | Go (sidecar) | Coordination intelligence via Copilot models. |
| GitHub API | REST + GraphQL | Repo, branch, PR, review management. Direct HTTP, no `gh` CLI. |
| Config | toml | Configuration format. |
| Token storage | OS keychain (keyring crate) | Secure OAuth token persistence. |

---

## 11. Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Copilot review not available (no subscription, rate limited) | Review pipeline stalls | Configurable: skip review, use alternative reviewer, or manual review. Frontend shows "review unavailable" state. |
| Copilot SDK is in preview, API may change | Coordination tasks break | Abstract SDK calls behind a trait. Fallback to direct LLM API calls (BYOK mode). |
| OAuth token expires or is revoked | All GitHub operations fail | Auto-refresh via refresh token. Frontend shows auth status, one-click re-auth. |
| Instances modify files outside ownership | Merge conflicts | CLAUDE.md enforces boundaries. Copilot SDK validates `teams.md` for overlaps. Pre-merge conflict detection via API. |
| Signal detection unreliable | Missed state transitions | CLAUDE.md forbids signals in code blocks. Plugin enforces handoff. Heartbeat timeout catches silence. |
| `teams.md` decomposition is wrong | Wasted work, collisions | Copilot SDK validates before execution. Human reviews in frontend. |
| Frontend complexity grows | Scope creep | v1 is focused: project list, dashboard, PR review, attention queue. No code editor, no full GitHub clone. |
| Multiple Copilot premium request limits | Coordination tasks throttled | Rate-limit awareness in SDK calls. Batch coordination queries. Degrade gracefully (skip triage, use simpler descriptions). |

---

## 12. Future Considerations

- **Auto-decomposition**: Given a PRD, auto-generate `teams.md` via Copilot SDK without a superpowers session.
- **Git worktree management**: Use `git worktree` for true filesystem isolation.
- **Cross-instance messaging**: Instances signal dependency completion to each other.
- **Token cost tracking**: Per-session, per-task cost estimation.
- **Session recording**: Full transcript capture for debugging and training data.
- **CI integration**: Run project CI pipeline on PRs before/alongside Copilot review.
- **Multiple review providers**: CodeRabbit, SonarQube, custom review bots alongside Copilot.
- **Remote execution**: Run sessions on remote machines, frontend consolidates.
- **Sentinel integration**: Connect to Kubernetes operational workflows.
- **Celerex integration**: Use Celerex agent framework for sophisticated cross-instance coordination.
- **Coalesce Enterprise integration**: Route Copilot SDK calls through Coalesce for model routing, compliance, and billing.
- **Mobile-friendly frontend**: Responsive design for monitoring from phone.
- **Notification system**: Push notifications (email, Slack, mobile) for attention events and completed PRs.

---

## Appendix A: `teams.md` Format

```markdown
# Teams Manifest
## Generated: YYYY-MM-DD
## PRD: <path to PRD in repo>
## Estimated total sessions: <N> (across context handoffs)

---

### Team: <Team Name>
**Branch prefix:** `feature/<prefix>-`
**Dependencies:** <None | list of team names that must complete first>

#### Member: <member-id>
- **Task:** <One-paragraph description>
- **Files owned:** <comma-separated paths, globs OK>
- **Branch:** `feature/<branch-name>`
- **Estimated context cycles:** <N>
- **Initial prompt:** >
  <Multi-line prompt. Self-contained. Tells the instance exactly
  what to read, what to build, what to test, what not to touch.>

---

### Execution Order
1. <Team A> and <Team B> start in parallel
2. <Team C> starts after <Team A> PRs merge
```

**Constraints:**
- File ownership is exclusive across all members
- Branch names are unique
- Each member gets exactly one branch
- Initial prompts are self-contained
- Dependency graph must be acyclic

---

## Appendix B: GitHub OAuth Device Flow

For CLI/local applications, the orchestrator uses the [device flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow):

1. `POST https://github.com/login/device/code` with `client_id` and `scope=repo`
2. Display the `user_code` and `verification_uri` to the user
3. Poll `POST https://github.com/login/oauth/access_token` with `device_code` until authorized
4. Store the `access_token` and `refresh_token` securely

The frontend can also support the standard web redirect flow for browser-based auth.
