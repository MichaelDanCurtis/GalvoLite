# Plan 10b — PR Creation Pipeline Design

**Date:** 2026-05-09
**Status:** Draft (awaiting user review before writing-plans phase)
**Scope:** Single PLAN.md. Second sub-plan of the 10a-10e GitHub-native effort decomposed during the 2026-05-07 brainstorm.

---

## Goal

Land the operator-driven PR creation pipeline on top of Plan 10a's auth +
read-only foundation. From the dashboard, an operator clicks "Open PR"
on an active or completed child session; GalvoLite pushes the session's
worktree branch to GitHub and opens a PR via `octocrab`. The PR URL is
persisted on the session row and surfaced in the UI as a "View PR"
link. Closes the gap between Plan 4 (worktree-per-child) and the PRD's
"Phase C — Review & Merge" vision; unblocks Plan 10c (PR review
surfacing + fix-spawning).

---

## Position in the GitHub-native decomposition

| Sub-plan | Status | Scope |
|---|---|---|
| **10a** Auth + read-only browsing | DONE 2026-05-08 (a0e0056) | Foundation: PAT auth, per-project linkage, PR/issue browse |
| **10b** PR creation pipeline | THIS spec | Push child-session branch + open PR via dashboard button |
| **10c** PR review surfacing + fix-spawning | Not started | Read review comments → spawn Claude with comments injected |
| **10d** Issue → session ingestion | Not started | Issues become first-class task sources |
| **10e** Webhook ingest → DomainEvents | Not started | Translate pr_created/pr_merged/issue_opened to bus events |

10b depends on 10a (Client wrapper + per-project repo linkage) AND
Plan 4 (each child session has a `worktree_path` and a `branch`).

---

## Architecture

One new migration, extensions to existing modules (no new top-level
subsystem), one new HTTP endpoint pair, one new modal. Touches:

| File | Role |
|---|---|
| `migrations/0008_sessions_pr_url.sql` | `ALTER TABLE sessions ADD COLUMN pr_url TEXT NULL` |
| `src/db.rs` | `Session.pr_url: Option<String>` field; `set_session_pr_url(id, &str)` setter; SELECT updates in `get_session` / `list_sessions` |
| `src/github/push.rs` (new) | Shell-out helper: `push_branch(worktree_path, branch, owner, name, token)` — runs `git -C <wt> push https://x-access-token:<token>@github.com/<owner>/<name>.git <branch>:<branch>`, redacts token from any error stderr before propagating |
| `src/github/client.rs` | New `Client::create_pull_request(owner, name, head, base, title, body, draft) -> Result<PrInfo, GithubError>` (uses `octocrab.pulls(owner, name).create(...)`); new `Client::default_branch(owner, name) -> Result<String, GithubError>` for base default |
| `src/main.rs::cmd_serve` | Token-scope warning enhancement: parse `x-oauth-scopes` from `/user` response; log `tracing::warn!` if `repo` write scope missing (non-fatal) |
| `src/api/github.rs` | `POST /api/sessions/:id/pr-draft` (returns `{title, body, base, commits_ahead, commits_behind}`); `POST /api/sessions/:id/pr` (push + create PR + write `pr_url` + emit event) |
| `src/api/mod.rs` | Two new routes |
| `src/api/events.rs` | New `DomainEvent::SessionPrCreated { session_id, project_id, pr_url, pr_number }` (variant + `applies_to` arm: `true` because session-scoped) |
| `ui/src/components/modals/OpenPrModal.tsx` (new) | Modal-with-preview using shared `Modal` primitive: editable title input, multiline body textarea (pre-populated), base-branch text input, "N commits ahead" indicator, Submit button |
| `ui/src/components/ExpandedSessionView.tsx` (modify) | "Open PR" button (opens modal) when eligible; "View PR" link when `pr_url` set |
| `ui/src/types.ts` | `Session.pr_url?: string \| null`; `OpenPrDraft`, `OpenPrReq`, `OpenPrResp` types |
| `ui/src/api.ts` | `getPrDraft(sessionId)`, `openPr(sessionId, {title, body, base})` |
| `ui/src/hooks/useSessionEvents.ts` (modify) | Handle `session.pr_created` event → patch local session state |

**Locked architectural choices:**

- **Operator-driven trigger** — button in `ExpandedSessionView`, no signal-based or state-based auto-PR. Smallest blast radius.
- **HTTPS push via PAT** (`x-access-token` pattern) — same `GITHUB_TOKEN` from Plan 10a; user may need to bump scopes to include `repo` write. GitHub's officially supported PAT-push pattern; single rotation point.
- **Modal-with-preview** — uses shared `Modal` primitive (LinkRepoModal pattern from Plan 10a). Operator sees and can edit title/body/base before submission.
- **Smart-default body content** branched on `session.role`:
  - **Orchestrator** (`role == "orchestrator"`): `## Task` + `## Commits` + `## Plan`
  - **Other roles** (implementation, brainstorm, scout, etc.): `## Task` + `## Commits`
- **One PR per session (1:1)** — tracked via `sessions.pr_url`; click-twice safe via 422 from `POST /pr` if `pr_url` already set
- **`SessionPrCreated` bus event** — fans out via existing WebSocket so dashboard auto-updates without refetch; matches `RecommendationActed` precedent
- **Token redaction** — push helper masks the PAT from any error string before propagating (security-critical; covered by a unit test)

---

## Data flow

### Draft-fetch flow (`GET /api/sessions/:id/pr-draft`)

1. Operator clicks "Open PR" in `ExpandedSessionView`. Button is gated on:
   - `project.github_repo != null` (project has linked repo)
   - `session.pr_url == null` (no existing PR for this session)
   - `session.branch != null` (session has a worktree branch)
2. UI opens `OpenPrModal` and immediately fetches the draft.
3. Backend handler:
   - Loads session via `Db::get_session(session_id)`. 404 if missing.
   - Loads parent project via `Db::get_project(session.project_id)`. 404 if missing or `github_repo` is None.
   - Parses `owner/name` from `project.github_repo` via the existing `parse_repo` helper (reused from Plan 10a `src/api/github.rs`).
   - Constructs `Client::from_env()`. 503 if no token.
   - Computes default base via `client.default_branch(owner, name).await`. Falls back to `"main"` on error (logs `tracing::warn!`).
   - Computes commit counts and log via `git -C <worktree_path>` shell-outs:
     - `commits_ahead` / `commits_behind`: `git rev-list --left-right --count <base>...HEAD` (handles missing base ref by falling back to `(0, 0)`).
     - Commits log: `git log --oneline --no-decorate <base>..HEAD` (caps at first 50 lines).
   - Fetches optional plan content if orchestrator: read `session.worktree_path/.galvolite/plan.md` (best-effort; missing = skip the section).
   - Generates title from `session.task_summary` (truncated to 72 chars at the last word boundary).
   - Generates body based on role (see "Body templates" below).
   - Returns:
     ```json
     {
       "title": "Add token-budget reset",
       "body": "## Task\n...\n\n## Commits\n- abc123 ...\n",
       "base": "main",
       "commits_ahead": 12,
       "commits_behind": 0
     }
     ```
4. Modal renders editable title input, multiline body textarea (pre-populated), base-branch text input (default-filled), and a small "12 commits ahead of main" indicator. Submit button disabled until `commits_ahead > 0` AND title.trim() non-empty.

### Open-PR flow (`POST /api/sessions/:id/pr`)

1. UI calls `POST /api/sessions/:id/pr` with body `{ title, body, base }` (operator's edited values).
2. Backend handler:
   - Re-loads session + project (race-safety; another client could have linked/unlinked the repo since draft fetch).
   - 422 if `session.pr_url` already set: `{"error": "PR already exists for this session", "pr_url": "<existing>"}`.
   - 404 chain identical to draft endpoint (session missing / no linked repo / no branch).
   - Constructs `Client::from_env()`. 503 if no token.
   - Calls `github::push::push_branch(&session.worktree_path, &branch, owner, name, &token)`:
     - Runs `git -C <worktree_path> push https://x-access-token:<token>@github.com/<owner>/<name>.git <branch>:<branch>` via `std::process::Command`.
     - On non-zero exit, captures stderr, runs `replace(&token, "***")` to redact, returns `Err(PushError::GitFailed(redacted_stderr))`.
     - Returns 502 with the redacted error to the client.
   - Calls `client.create_pull_request(owner, name, head=&branch, base=&req.base, title=&req.title, body=&req.body, draft=false).await`.
     - On octocrab error (e.g. branch deleted, base missing, PR already exists), returns 502 with the error message.
   - Writes `db.set_session_pr_url(session_id, &pr_info.html_url)`.
   - Publishes `DomainEvent::SessionPrCreated { session_id, project_id: session.project_id, pr_url: pr_info.html_url.clone(), pr_number: pr_info.number }`.
   - Returns 200 with `{ pr_url, pr_number }`.
3. UI's `useSessionEvents` subscriber receives the event; updates `session.pr_url` in local state. Modal closes. Button flips to "View PR" link in the next render.

### Body templates

**Orchestrator** (`session.role == "orchestrator"`):

```
## Task
<session.task_summary>

## Commits
- <oneline 1>
- <oneline 2>
- ...

## Plan
<contents of <worktree>/.galvolite/plan.md, or "(no plan.md)" if missing>
```

**Other roles** (implementation, brainstorm, scout, default):

```
## Task
<session.task_summary>

## Commits
- <oneline 1>
- <oneline 2>
- ...
```

Plan section is intentionally omitted for non-orchestrators to keep PR
bodies short for the typical case (focused implementation child).

### Token-scope check at startup (additive to Plan 10a)

After `client.validate()` returns Ok in `cmd_serve`, also fetch the
response headers (octocrab's `current().user_with_response()` or via the
inner `reqwest::Client`) and parse `x-oauth-scopes`. If `repo` is missing,
log `tracing::warn!("GITHUB_TOKEN lacks 'repo' write scope — Plan 10b PR creation will fail; rotate the PAT to add 'repo' or 'public_repo' write scope")`. **Do not fail startup** — Plan 10a features still work read-only.

The startup-time warning is purely advisory; the Plan 10b push path also surfaces the underlying GitHub error verbatim if push fails, so operators get a second signal at use time.

---

## Error handling

| Trigger | Backend response | UI surfacing |
|---|---|---|
| `GITHUB_TOKEN` unset | `pr-draft`/`pr` → `503 {"error": "GITHUB_TOKEN not set"}` | Button hidden when `GithubStatusBadge` says not-connected; modal shows error banner if race causes 503 |
| Token lacks `repo` write scope | `pr` push step → `502` with redacted git stderr (e.g. `"remote: Permission to .../*.git denied to <user>"`) | Modal error banner with hint: "PAT may need 'repo' write scope" |
| Session has no linked repo project | `404 {"error": "no linked repo"}` | Button hidden; defense-in-depth |
| Session has no branch | `404 {"error": "session has no branch"}` | Button hidden when `session.branch == null` |
| Branch has 0 commits ahead of base | `pr-draft` returns `commits_ahead: 0`; modal disables Submit | "0 commits ahead — nothing to PR" message |
| `pr_url` already set | `422 {"error": "PR already exists for this session", "pr_url": "..."}` | Modal closes; dashboard already shows existing pr_url via "View PR" link |
| Push fails (network, auth, rejected, no upstream) | `502` with token-redacted stderr | Modal error banner with stderr |
| `create_pull_request` fails (base deleted, PR already exists for branch on GitHub side, etc.) | `502` with octocrab error message | Modal error banner |
| `default_branch` lookup fails | Logs `tracing::warn!`; falls back to `"main"`; draft returns base=`"main"` | Modal pre-fills `"main"`; operator can edit |
| `SessionPrCreated` bus publish fails (no subscribers) | Silently ignored (matches existing pattern) | UI re-fetches session on next render |

**Token redaction (security-critical):** the push helper builds the URL once with the token interpolated, then before propagating any error string, runs `replace(&token, "***")` to strip every occurrence from captured stderr. Test `tests/github_push.rs::error_message_redacts_token` deliberately triggers a push failure and asserts the token value never appears in the returned error.

**Logging:** all non-success responses log via `tracing::warn!` with `session_id`, `project_id`, and `repo` for triage. The token value is never logged. The `Client::create_pull_request` error path logs the GitHub-side error verbatim (no token in those errors).

---

## Testing

Mirrors Plan 10a's mockito-based pattern. Estimated counts: ~12 backend tests, ~6 UI tests.

### Backend

- `tests/github_push.rs` (new) — push helper tests using a temp git repo:
  - `push_succeeds_to_local_bare_remote` — set up `git init --bare` as a remote, call `push_branch`, assert remote has the branch
  - `push_fails_with_redacted_token_when_remote_unreachable` — push to a bogus remote URL (deliberately 127.0.0.1:1 or similar), assert error string does NOT contain the token literal
  - `push_fails_clearly_when_branch_missing_locally` — try to push a non-existent branch, assert clear error
  - `push_fails_clearly_when_worktree_path_invalid` — invalid path, assert clear error

- `tests/github_client.rs` — extend with `create_pull_request` mockito tests:
  - `create_pull_request_returns_pr_info_on_201` — mocked POST `/repos/{owner}/{name}/pulls` returns 201 with html_url + number
  - `create_pull_request_err_on_422` — PR already exists for branch, octocrab surfaces 422
  - `create_pull_request_err_on_404` — base missing, mocked 404
  - `default_branch_returns_main_or_master` — mocked `/repos/{owner}/{name}` with `default_branch` field

- `tests/api_github.rs` — extend with PR endpoint tests:
  - `pr_draft_returns_orchestrator_body_shape` — set up a session with role=orchestrator and a fake plan.md in its worktree, mock the git log shell-out via a test git repo, assert body contains `## Plan`
  - `pr_draft_returns_child_body_shape` — role=implementation, assert body has `## Commits` but NOT `## Plan`
  - `pr_draft_404s_on_missing_session` — bogus uuid
  - `pr_draft_404s_on_no_linked_repo` — session exists but project.github_repo is None
  - `pr_draft_404s_on_session_with_no_branch` — orchestrator-only session
  - `pr_endpoint_422s_when_pr_url_already_set` — pre-populate `sessions.pr_url`, attempt POST, assert 422 with the existing pr_url echoed back
  - `pr_endpoint_503_when_token_unset`
  - GitHub-network happy path stays at the `tests/github_client.rs::create_pull_request` level (same env-injection limitation as Plan 10a)

- `tests/db_crud.rs` — extend with:
  - `session_pr_url_round_trip` — insert session → assert pr_url None → set → assert Some → set None → assert None

- `tests/api_github.rs` (also): one event-fanout test that exercises the bus publish on success (using the existing `EventBroadcaster` test pattern from `tests/api_event_broadcast.rs`).

### Frontend

- `ui/src/components/modals/OpenPrModal.test.tsx` (new):
  - `renders draft on mount` — mock getPrDraft returns title/body/base/counts, modal renders pre-populated
  - `submit calls openPr with edited values` — edit body, click Submit, assert openPr called with edited body
  - `submit disabled when commits_ahead is 0` — mock draft with `commits_ahead: 0`, assert Submit disabled
  - `submit disabled when title is empty` — clear title, assert Submit disabled
  - `error banner renders on 502` — mock openPr rejection, assert error visible

- `ui/src/components/ExpandedSessionView.test.tsx` — extend or co-locate:
  - `open_pr_button_hidden_when_no_linked_repo` — project.github_repo null
  - `open_pr_button_hidden_when_session_has_no_branch` — session.branch null
  - `view_pr_link_renders_when_pr_url_set` — pr_url non-null, assert anchor with href

---

## Out of scope

These items deliberately are NOT in 10b (deferred or rejected):

- **Signal-driven auto-PR** (`[STATUS:READY_FOR_PR]` SignalPump path) — defer to 10c when fix-spawning needs it
- **PR template rendering** (`.github/PULL_REQUEST_TEMPLATE.md`) — we use our own body templates; revisit when operators ask
- **Draft PR support** — all PRs default to `draft: false`; add a checkbox in a follow-up if needed
- **Multiple PRs per session** — 1:1 model is fixed in 10b; revisit if close/reopen-with-new-number becomes common
- **Per-project base-branch override** — base default comes from GitHub `default_branch`; operator can edit per-PR in modal; no project-level config
- **Reviewer assignment** — defer to 10c
- **Branch deletion after merge** — defer to 10c or 10e (webhook ingest)
- **Status checks / required-CI gating** — defer to 10c
- **PR update / re-push** — once `pr_url` is set, the only action is "View PR"; force-push or amend flow is out of scope (operator does this from the terminal if needed; the PR auto-updates because we're tracking the same branch)
- **OAuth flow / Keychain token storage** — still env-var-only PAT per Plan 10a's choice; revisit when GalvoLite ships beyond solo-dev
- **Webhook-driven `pr_merged` / `pr_closed` event handling** — defer to 10e; `pr_url` stays "stale" (linking to a closed PR) until manually cleared
