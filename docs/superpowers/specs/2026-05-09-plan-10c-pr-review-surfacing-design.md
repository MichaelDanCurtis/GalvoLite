# Plan 10c — PR Review Surfacing Design

**Date:** 2026-05-09
**Status:** Auto-approved (operator delegated autonomous execution)
**Scope:** Single PLAN.md. Third sub-plan of the 10a-10e GitHub-native effort.

---

## Goal

Surface GitHub PR reviews (top-level reviews + inline review comments)
in the GalvoLite dashboard for sessions that have a linked PR. Operator
sees what Copilot / human reviewers wrote without context-switching
to github.com. Read-only — fix-spawning (operator clicks comments to
spawn a Claude fix session) is explicitly out of scope, deferred to a
future 10c.x sub-plan.

---

## Position in the GitHub-native decomposition

| Sub-plan | Status | Scope |
|---|---|---|
| **10a** Auth + read-only browsing | DONE 2026-05-08 | Foundation |
| **10b** PR creation pipeline | DONE 2026-05-09 | Operator opens PRs |
| **10c** PR review surfacing | THIS spec | Read review comments in dashboard |
| **10c.x** Fix-spawning (future) | Not started | "Fix selected" spawns Claude session with comments |
| **10d** Issue → session ingestion | Not started (parallel) | |
| **10e** Webhook ingest | Not started | |

10c depends on 10b (sessions need `pr_url` to exercise the panel).

---

## Architecture

No new subsystems, no migration, no new bus events. Touches:

| File | Role |
|---|---|
| `src/github/client.rs` | New `ReviewSummary { id, author, state, body, submitted_at, url }` + `ReviewCommentSummary { id, author, body, path, line, url }`; new `list_pr_reviews(owner, name, pr_number)` returning both as a single struct `PrReviewsBundle { reviews, comments }` |
| `src/github/mod.rs` | Re-export `ReviewSummary`, `ReviewCommentSummary`, `PrReviewsBundle` |
| `src/api/github.rs` | New `pr_reviews` handler at `GET /api/sessions/:id/pr-reviews` — 404 chain like other session endpoints, 503 if no token, 502 on GitHub error |
| `src/api/mod.rs` | Register route |
| `ui/src/types.ts` | New `PrReview`, `PrReviewComment`, `PrReviewsBundle` types |
| `ui/src/api.ts` | New `getPrReviews(sessionId)` |
| `ui/src/components/PrReviewsPanel.tsx` (new) | Renders the bundle: top-level reviews list + grouped inline comments by file. Manual Refresh button (no polling). |
| `ui/src/components/ExpandedSessionView.tsx` | Mount `<PrReviewsPanel>` when `session.pr_url` is non-null |

**Locked architectural choices:**
- Read-only — no mutations, no fix-spawning
- Manual refresh, no polling (per `feedback_idle_sessions_no_cost.md`)
- No persistence — every panel-open hits GitHub
- PR number derived from `session.pr_url` via regex (`/pull/(\d+)$`); falls back gracefully if URL shape unexpected
- Bundle struct returned as single JSON payload (one client call, two GitHub-side fetches in parallel via `tokio::join!`)

---

## Data flow

1. Operator opens ExpandedSessionView for a session with `pr_url` set
2. UI calls `GET /api/sessions/:id/pr-reviews`
3. Backend handler:
   - Resolves session + project (reuses `resolve_session_for_pr` from 10b)
   - Extracts PR number from `session.pr_url` via regex
   - Calls `client.list_pr_reviews(owner, name, pr_number)` which itself does `tokio::join!(reviews_fut, comments_fut)`
   - Returns `PrReviewsBundle { reviews, comments }` as JSON
4. UI renders top-level reviews (with state badges: APPROVED / CHANGES_REQUESTED / COMMENTED) and grouped inline comments

---

## Error handling

| Trigger | Backend response | UI surfacing |
|---|---|---|
| Session has no pr_url | 404 `{"error": "no PR for this session"}` | Panel hidden when pr_url is null |
| pr_url has unexpected shape | 500 `{"error": "could not parse PR number from url"}` | Error banner |
| Token unset | 503 | Error banner |
| GitHub error | 502 with error string | Error banner with retry |

---

## Testing

- `tests/github_client.rs` — extend with mockito tests for `list_pr_reviews` happy path + 404
- `tests/api_github.rs` — extend with `pr_reviews_404_when_no_pr_url`, `pr_reviews_503_when_token_unset`
- `ui/src/components/PrReviewsPanel.test.tsx` — render with mocked bundle, refresh button, error banner

## Out of scope
- Fix-spawning (defer to 10c.x)
- Caching reviews
- Webhook-driven review-ready notifications (defer to 10e)
- "Resolve thread" / "Reply to comment" mutations (defer)
