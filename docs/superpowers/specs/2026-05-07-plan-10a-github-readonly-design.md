# Plan 10a — GitHub Read-Only Layer Design

**Date:** 2026-05-07
**Status:** Draft (awaiting user review before writing-plans phase)
**Scope:** Single PLAN.md. First sub-plan of the multi-plan GitHub-native effort decomposed during the Plan 10 brainstorm.

---

## Goal

Land the foundational GitHub integration layer for GalvoLite: PAT-based
auth via env var, per-project repo linkage, and read-only browsing of
the linked repo's open PRs and issues from the dashboard. No mutations,
no OAuth, no webhooks. Closes the largest PRD gap (zero GitHub code
today, despite the PRD calling GalvoLite "GitHub-native") with the
smallest possible blast radius and unblocks the rest of the GitHub-layer
sub-plans (10b PR creation, 10c PR review surfacing, 10d issue → session
ingestion, 10e webhook ingest).

---

## Context: full GitHub layer decomposition

For reference — Plan 10 was decomposed into five independent sub-plans:

| Sub-plan | Scope | Depends on |
|---|---|---|
| **10a** Auth + read-only browsing | THIS spec | — |
| **10b** PR creation pipeline | Push branch + open PR from a child-session worktree | 10a + Plan 4 worktree-per-child |
| **10c** PR review surfacing + fix-spawning | Read review comments; "Fix selected" spawns a Claude session with comments injected | 10b |
| **10d** Issue → session ingestion | Issues become first-class task sources; comment back on state changes | 10a (and arguably 10b for PR-back path) |
| **10e** Webhook ingest → DomainEvents | Translate `pr_created` / `pr_merged` / `issue_opened` into bus events | 10a |

10a is the foundation: every other sub-plan needs auth and the per-project
repo linkage to exist.

---

## Architecture

One new Rust subsystem under `src/github/`, one new migration, two new
HTTP endpoint groups, three new React components. No new background tasks.
No mutations to GitHub.

| File | Role |
|---|---|
| `src/github/mod.rs` | Public API: `Client::from_env() -> Option<Client>`, `validate() -> Option<User>`, `list_open_prs(repo)`, `list_open_issues(repo)` |
| `src/github/client.rs` | Thin `octocrab::Octocrab` wrapper with PAT auth + connected-user cache |
| `src/api/github.rs` | Axum handlers: `GET /api/github/status`, `GET /api/projects/:id/github/{prs,issues}`, `PUT /api/projects/:id/github/repo` |
| `migrations/0007_projects_github_repo.sql` | `ALTER TABLE projects ADD COLUMN github_repo TEXT NULL` |
| `ui/src/components/GithubPanel.tsx` | Per-project panel: PRs + Issues lists for the linked repo. Single Refresh button refetches both lists in parallel. |
| `ui/src/components/LinkRepoModal.tsx` | Per-project settings modal: enter `owner/name`, validate via API, save |
| `ui/src/components/GithubStatusBadge.tsx` | Global dashboard-header badge showing connected user (`@octocat`) or "GitHub: not connected". Polls `GET /api/github/status` once on mount; no live refresh. |

**Locked architectural choices:**

- **`octocrab`** crate (not shell-out to `gh`). Native Rust, mockable via
  `mockito` for tests, no runtime dependency on a separate CLI binary.
- **PAT via `GITHUB_TOKEN` env var only.** No DB-stored secrets, no
  Keychain integration, no OAuth device flow. Solo-dev v1; OAuth and
  Keychain swap in a later sub-plan if/when GalvoLite ships beyond solo
  use.
- **All GitHub calls through the Rust backend.** UI never sees the
  token. No CORS to GitHub, no token-in-browser exposure.
- **One linked repo per GalvoLite project** (`owner/name` string). If
  unset, the panel hides and the project card shows "Link a repo to
  enable GitHub features."
- **No background polling, no application-layer cache.** All fetches
  triggered by panel-open or manual Refresh. Consistent with
  `feedback_idle_sessions_no_cost.md`.

---

## Data flow

### Backend startup wiring (`src/main.rs::cmd_serve`)

After constructing `SessionManager`, attempt `github::Client::from_env()`.
If `GITHUB_TOKEN` is set, hit `GET /user` once to validate; cache the
resulting `User { login, avatar_url }` on an
`Arc<RwLock<Option<User>>>` shared with the API state. If unset or
invalid, the cache stays `None` and `GET /api/github/status` returns
`{ connected: false, reason: "no token" | "token invalid" }`.

No retries; no background re-validation. The connected-user cache
refreshes only on server restart. Operators rotate tokens by setting
the new value and restarting.

### Per-project linkage flow

1. User opens `LinkRepoModal` from a project card → enters `owner/name` → clicks Save
2. UI calls `PUT /api/projects/:id/github/repo { repo: "owner/name" }`
3. Backend hits `GET /repos/{owner}/{name}` to validate (catches typos and missing-permissions cases)
4. On success: write `github_repo` to `projects` row, return `200 OK`
5. On failure: return `422 Unprocessable Entity` with the GitHub error message; UI shows inline error in the modal
6. After success, the project card now exposes the GithubPanel

### Panel-open data flow

1. UI calls `GET /api/projects/:id/github/prs` and `GET /api/projects/:id/github/issues` in parallel
2. Backend reads the project row; if `github_repo` is null, returns `404 No linked repo`
3. Otherwise calls
   `octocrab.pulls(owner, name).list().state(Open).per_page(50).send()`
   and likewise for issues
4. Maps each result to a small JSON shape, dropping fields the panel
   doesn't render (full details stay one click away on github.com):

```rust
struct PrSummary {
    number: u64,
    title: String,
    author: String,
    draft: bool,
    updated_at: String, // RFC 3339
    url: String,        // html_url to github.com
}

struct IssueSummary {
    number: u64,
    title: String,
    author: String,
    labels: Vec<String>,
    updated_at: String,
    url: String,
}
```

5. Returns JSON; on API error (rate limit, 5xx, network), returns
   `502 Bad Gateway` with the underlying error message as a string

### Refresh model

- **Manual only.** Refresh button on the panel re-runs both fetches.
- **Modal-open trigger:** opening a project's GithubPanel triggers one
  fetch automatically.
- **No polling, no background fetches** — consistent with
  `feedback_idle_sessions_no_cost.md`. Two consecutive panel opens
  within seconds will hit GitHub twice; this is a feature (always
  fresh on open) not a bug.

octocrab honors GitHub's `ETag` headers internally, so back-to-back
identical requests are cheap on GitHub's end. If the user spams Refresh
and we hit the rate limit, we surface the error rather than queuing.

---

## Error handling

| Trigger | Backend response | UI surfacing |
|---|---|---|
| `GITHUB_TOKEN` unset | `GET /api/github/status` → `{ connected: false, reason: "no token" }` | Status badge in dashboard header: "GitHub: not connected" |
| Token invalid (401 from `/user`) | Same shape with `reason: "token invalid"` | Status: "GitHub: token invalid" |
| Repo link to nonexistent repo | `PUT /repo` → `422 { error: "<msg>" }` | Inline error in `LinkRepoModal` |
| Repo link to private repo PAT can't see | `PUT /repo` → `422` | Same inline error: "not found or no access" |
| Panel fetch hits rate limit | `GET /prs` → `502 { error: "rate limited, resets at <ts>" }` | Panel error banner with reset time; Refresh button disabled until reset |
| Network / transient | `502 { error: "<message>" }` | Panel error banner; Refresh stays enabled |
| Project row exists but `github_repo` is null | `GET /prs` → `404 { error: "no linked repo" }` | Panel hidden in UI; this case is defense-in-depth — UI shouldn't issue the call when unlinked |

**Logging:** all non-success responses log via `tracing::warn!` with
`project_id` + `repo` for triage. The token value is never logged.

---

## Testing

Mirrors the Plan 8/9 patterns. Estimated counts: ~10 backend tests, ~6
UI tests.

**Backend (mockito-based):**

- `tests/github_client.rs`
  - `validate_returns_user_on_200` — mockito serves a `/user` payload, asserts `Client::validate()` returns `Some(User { login: "...", ... })`
  - `validate_returns_none_on_401` — mockito returns 401, asserts `None`
  - `list_open_prs_returns_summaries` — mockito serves a `/repos/{owner}/{name}/pulls?state=open&per_page=50` payload, asserts the right `Vec<PrSummary>`
  - `list_open_issues_returns_summaries` — same shape for issues
  - `rate_limit_response_surfaces_error` — mockito returns 403 with `X-RateLimit-Remaining: 0`, asserts our error variant carries the reset time

- `tests/api_github.rs`
  - `status_unconfigured_returns_not_connected` — no env var, hits status endpoint
  - `status_invalid_token_returns_token_invalid` — env var set, mock GitHub returns 401
  - `link_repo_validates_via_github_then_writes_db` — happy path
  - `link_repo_unknown_returns_422` — mock GitHub 404, no DB write
  - `prs_endpoint_404s_when_no_linked_repo` — defense-in-depth case
  - `prs_endpoint_returns_summaries_for_linked_repo` — happy path

**UI (Vitest):**

- `ui/src/components/GithubPanel.test.tsx`
  - `renders_empty_state_when_no_prs` — mocked API returns `[]`
  - `renders_pr_and_issue_lists` — mocked API returns one of each
  - `refresh_button_triggers_refetch` — clicking Refresh calls listGithubPrs/Issues again
  - `error_banner_renders_on_api_error` — mocked rejection

- `ui/src/components/LinkRepoModal.test.tsx`
  - `save_success_closes_modal` — happy path
  - `save_422_displays_inline_error` — error path

---

## Out of scope

These items deliberately are NOT in 10a (deferred to later sub-plans
or rejected as unneeded for v1):

- **OAuth device flow** — see 10a.5 / 10b
- **Webhook ingest** — see 10e
- **Any mutations to GitHub** — create PR, comment, label, assign,
  merge, close, react. All deferred to 10b/10c/10d.
- **Pagination beyond the first 50** — for solo-dev volumes this is
  enough; if a user has a repo with 100+ open PRs, they can use
  github.com directly. Add when 10e webhook events make it easier to
  reason about deltas.
- **Application-layer caching** — defer until rate-limit complaints
  emerge.
- **GitHub Actions / Copilot integration** — see PRD §3.2; much later.
- **Multi-repo linkage per project** — defer until a real use case
  surfaces (likely never for solo-continuity).
- **Token rotation UI** — operators restart the server with a new env
  var. Add Settings UI in a Keychain-storage sub-plan.
- **DomainEvent for `github.linked` / `github.unlinked`** — the bus
  has no subscriber that needs these yet. Add when 10e or 10c needs
  reactivity.
