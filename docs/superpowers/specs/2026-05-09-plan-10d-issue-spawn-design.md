# Plan 10d — Issue → Session Spawn Design

**Date:** 2026-05-09
**Status:** Auto-approved (operator delegated autonomous execution)
**Scope:** Single PLAN.md. Fourth sub-plan of 10a-10e.

## Goal

Per-issue "Start session" button in the existing `GithubPanel`. Clicking spawns
a child session under the project's orchestrator with the issue title in
`task_summary` and the issue body written to `<worktree>/.galvolite/issue.md`
for Claude to read. The new session row tracks `github_issue_url` so a future
sub-plan can comment back when state changes.

## Architecture

| File | Role |
|---|---|
| `migrations/0009_sessions_github_issue_url.sql` | `ALTER TABLE sessions ADD COLUMN github_issue_url TEXT NULL` |
| `src/db.rs` | `Session.github_issue_url: Option<String>` + `Db::set_session_github_issue_url` setter + SELECT updates |
| `src/github/client.rs` | New `Client::get_issue(owner, name, number) -> IssueDetail { title, body, html_url }` |
| `src/github/mod.rs` | Re-export `IssueDetail` |
| `src/api/github.rs` | New `spawn_from_issue` handler at `POST /api/projects/:id/github/issues/:number/spawn` — finds project's orchestrator, calls `spawn_child(orchestrator.id, "implementation", &issue.title, None)`, writes issue body to `.galvolite/issue.md`, sets `github_issue_url`, returns `{session_id}` |
| `src/api/mod.rs` | Register route |
| `ui/src/types.ts` | `Session.github_issue_url?` + `SpawnFromIssueResp` |
| `ui/src/api.ts` | `spawnFromIssue(projectId, number)` |
| `ui/src/components/GithubPanel.tsx` | Per-issue "Start session" button — calls `api.spawnFromIssue`, parent refreshes sessions list on success |

**Locked design calls:**
- task_summary = issue title (truncated to 200 chars). Body goes to `.galvolite/issue.md` in the worktree (orchestrator reads it for context).
- Spawn under the project's orchestrator via existing `find_orchestrator_for_project` + `spawn_child` machinery (Plan 4/8). Returns 409 if no orchestrator exists.
- No comment-back to GitHub (defer to 10e webhook ingest or 10d.x).
- No new bus event — caller refetches sessions list after spawn (existing pattern).

## Error matrix

| Trigger | Backend response |
|---|---|
| Project not found | 404 |
| No linked repo | 404 `{"error": "no linked repo"}` |
| No orchestrator for project | 409 `{"error": "no orchestrator session running"}` |
| Token unset | 503 |
| GitHub error (issue not found / 404) | 502 |
| spawn_child failure | maps via `err_to_response` |

## Testing

- `tests/github_client.rs` — extend with `get_issue` mockito tests (200 + 404)
- `tests/db_crud.rs` — `session_github_issue_url_round_trip`
- `tests/api_github.rs` — `spawn_from_issue_404_when_no_linked_repo`, `spawn_from_issue_503_when_token_unset`, `spawn_from_issue_409_when_no_orchestrator`
- `ui/src/components/GithubPanel.test.tsx` — extend with "Start session button calls spawnFromIssue"

## Out of scope
- Comment-back to issue on session state changes (10e or 10d.x)
- Issue assignee / label sync
- Closing the issue when session ships a PR
