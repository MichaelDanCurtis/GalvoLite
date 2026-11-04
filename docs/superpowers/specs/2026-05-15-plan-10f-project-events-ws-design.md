# Plan 10f — Per-Project DomainEvent WebSocket Design

**Date:** 2026-05-15
**Status:** Approved (operator approval 2026-05-15)
**Scope:** Single PLAN.md. Closes the consumer gap for 10e bus events surfaced by the 10a-10e smoke (`memory/project_plan10_smoke_findings.md`).

## Goal

Project-scoped `DomainEvent`s fired by Plan 10e (and earlier `ProjectAdded`) currently traverse the global event bus but are unreachable by any consumer. The only WebSocket fan-out, `terminal_ws` at `/api/sessions/:id/terminal`, filters by `evt.applies_to(session_id)` — every project-scoped event returns `false` and is dropped.

This plan adds a project-scoped WebSocket so the dashboard can react to `IssueOpened`, `PullRequestMerged`, and future project events without polling.

## Architecture

| File | Role |
|---|---|
| `src/api/events.rs` | Add `applies_to_project(project_id: Uuid) -> bool` method on `DomainEvent`. Returns `true` for any variant whose payload contains a matching `project_id` field. Today (post-10e) that set is: `IssueOpened`, `PullRequestMerged`, `SessionPrCreated`, `ProjectAdded`, `ProjectDeleted`, `OverseerDegraded`, `OverseerRecovered`. `SessionPrCreated` is dual-scoped (matches both `applies_to` and `applies_to_project`) — natural and intended: the GithubPanel refetches PRs when ANY session under the project ships a PR, not just when a webhook reports a merge. Returns `false` for purely session-scoped or project-less variants. Coexists with existing `applies_to(session_id)` — neither is modified. Future variants opt in by adding a `project_id` field. |
| `src/api/mod.rs` | New route: `.route("/api/projects/:id/events", get(project_events_ws))`. New handler `project_events_ws(State, Path<Uuid>, WebSocketUpgrade)` that validates the project exists (404 before upgrade if not), then upgrades and runs `handle_project_events_ws(socket, mgr, project_id)`. |
| `src/api/mod.rs` | New helper `handle_project_events_ws(socket, mgr, project_id)`. Uses the funnel pattern from `terminal_ws` but unidirectional: single `mpsc::channel<Message>` consumed by a writer task; one `event_forwarder` producer that subscribes to `mgr.event_bus()`, filters by `applies_to_project(project_id)`, JSON-serializes, and pushes `Message::Text(json)` to the funnel. No PTY pump, no client-frame reader (we still drain incoming Close frames so the socket terminates cleanly). |
| `tests/api_project_events.rs` (new) | 5 integration tests covering match-by-project, non-match-by-project, session-scoped event ignored, github webhook event delivery, and `SessionPrCreated` dual-scoped delivery. |
| `ui/src/hooks/useProjectEvents.ts` (new) | `useProjectEvents(projectId, { onEvent, onError? })` hook. Opens WS to `/api/projects/:projectId/events`, parses each `Message.data` as JSON `DomainEvent`, calls `onEvent(evt)` per frame. Cleans up on unmount or `projectId` change. Auto-reconnect with exponential backoff per attempt (1s, 2s, 4s, 8s, then capped at 16s for any further attempts) up to 10 total attempts; after exhaustion calls `onError(reason)` once. Quiet on close code 1000 (normal). |
| `ui/src/components/GithubPanel.tsx` | Mount `useProjectEvents(project.id, handleProjectEvent)`. `handleProjectEvent(evt)` switches on `evt.type`: refetches issues on `github.issue_opened`; refetches PRs on `github.pull_request_merged` OR `session.pr_created` (latter fires when a session opens a PR via Plan 10b); no-op on others. Manual Refresh button stays. |
| `ui/src/hooks/__tests__/useProjectEvents.test.ts` (new) | Vitest test using `vitest-websocket-mock` (or hand-rolled mock): mount hook, fire mock event, assert `onEvent` called; unmount, assert WS closed. |

## Locked design calls

- **WebSocket, not SSE.** Consistent with existing `terminal_ws`. Funnel pattern is proven. Unidirectional usage is fine.
- **Subscription lifetime tied to GithubPanel mount.** Hook only opens the WS while the panel is on screen. Closes on project switch or panel unmount. Avoids consuming a connection when the operator is not looking at GitHub data.
- **Refetch, don't patch.** Consumer refetches the affected list (issues or PRs) rather than splicing the new payload into local state. Always correct, costs one HTTP roundtrip per event (rare).
- **`applies_to_project` is additive.** Does not modify existing `applies_to(session_id)`. Both methods coexist on `DomainEvent`. Session-scoped consumers stay untouched.
- **No new bus event variants in 10f.** Just plumbing for the existing project-scoped variants (`IssueOpened`, `PullRequestMerged`, `SessionPrCreated`, `ProjectAdded`, `ProjectDeleted`, `OverseerDegraded`, `OverseerRecovered`). Future additions get free routing as long as their payload includes `project_id`.
- **Project existence check before WS upgrade.** Returns 404 JSON (matching `ProjectNotFound` mapping in `err_to_response`) if the project id is unknown — avoids handing out a WS for a phantom project.
- **No client→server messages on this WS.** The reader half drains Close frames only; any other inbound frame is silently dropped.

## Error matrix

| Trigger | Backend response | UI behavior |
|---|---|---|
| Project not found | 404 `{"error":"project not found"}` before WS upgrade | Hook surfaces error; GithubPanel keeps last known data |
| WS broadcast lag (`RecvError::Lagged`) | log + continue (matches `terminal_ws`) | UI sees nothing — events are best-effort |
| WS broadcast closed (`RecvError::Closed`) | end forwarder task, sink drops | Hook reconnects (close code != 1000) |
| Client disconnect | drop subscription server-side, no retry | n/a |
| UI WS unexpected close | n/a | Reconnect with exponential backoff (1s, 2s, 4s, 8s, 16s) up to 10 attempts; then surface error to consumer via `onError` callback |
| Bad JSON from server | log warn, skip frame | Hook ignores malformed frames |

## Testing

- `tests/api_project_events.rs` (new) — 5 integration tests:
  1. WS connect to project P1 → `mgr.event_bus().publish(IssueOpened { project_id: P1, ... })` → WS receives JSON within 200ms with `type=github.issue_opened`
  2. WS connect to project P1 → `mgr.event_bus().publish(IssueOpened { project_id: P2, ... })` → WS receives nothing within 200ms (proves project-id filter discriminates)
  3. WS connect to project P1 → publish a purely session-scoped variant (e.g. `RoleChanged` or `MetaChanged`, whichever has no `project_id`) → WS receives nothing within 200ms (proves session-scoped events are filtered out)
  4. WS connect to project P1 → `mgr.event_bus().publish(PullRequestMerged { project_id: P1, ... })` → WS receives JSON within 200ms with `type=github.pull_request_merged`
  5. WS connect to project P1 → `mgr.event_bus().publish(SessionPrCreated { session_id, project_id: P1, ... })` → WS receives JSON within 200ms (proves dual-scoped variant fires for project subscribers)
- Unit tests in `src/api/events.rs` for `applies_to_project` — assert each variant returns expected boolean for matching/non-matching project_id
- Vitest unit test for `useProjectEvents` — mock WS via class shim, fire event, assert `onEvent` called with parsed payload; unmount, assert close called; reconnect path: simulate transient close, assert reopen
- E2E smoke (manual, post-merge): start server with `GITHUB_TOKEN` + `GITHUB_WEBHOOK_SECRET`, link project to a real repo, fire signed `issues opened` curl as in `memory/project_plan10_smoke_findings.md`, observe GithubPanel issues list refresh without manual Refresh click

## Out of scope

- Toast notifications on events (style/UX discussion belongs in its own plan)
- Auto-close issue when matching session ships PR (needs design — which sessions, retry semantics, idempotency)
- Comment-back to issue on session lifecycle changes
- Adding session-scoped events to project stream (rejected option from design discussion)
- Reconnection telemetry surfaced in dashboard (premature)
- Server-pushed authentication on the WS (single-user assumption — same as terminal_ws today)
- Multiple-project subscription multiplexing (one project per hook instance — switching projects closes and reopens)

## Performance notes

- Project event volume is bounded by webhook delivery rate (typically <1/min in normal use). Refetch cost per event is one `GET /api/projects/:id/github/issues` or `/github/prs` — both already cached on the GitHub side and have small payloads. Far cheaper than polling.
- Bus capacity is the existing `EventBus` channel size (set in `EventBus::new` constructor). 10f does not change it; lag is logged and tolerated.
