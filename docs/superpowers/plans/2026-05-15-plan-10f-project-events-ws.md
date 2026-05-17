# Plan 10f — Per-Project DomainEvent WebSocket Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-project WebSocket endpoint (`/api/projects/:id/events`) that fans out project-scoped `DomainEvent`s to dashboard subscribers, plus a `useProjectEvents` UI hook that drives auto-refresh of `GithubPanel` issues + PRs when relevant events arrive.

**Architecture:** Backend extends `DomainEvent` with `applies_to_project(project_id)` and adds a new WS handler reusing the funnel pattern from `terminal_ws` (unidirectional, no PTY pump, no client→server reader). UI adds a hook that opens the WS, parses text frames, calls back into the consumer with reconnect on transient close. `GithubPanel` mounts the hook and refetches its lists on matching events.

**Tech Stack:** Rust (axum 0.7, tokio 1.x broadcast, tokio-tungstenite 0.21 in tests), React 18 + TypeScript, Vitest, existing `EventBroadcaster` from `src/api/events.rs`, existing `serve_ephemeral` test helper.

**Context links (read first):**
- Spec: `docs/superpowers/specs/2026-05-15-plan-10f-project-events-ws-design.md` (committed in 631a6c9)
- 10a-10e smoke memo: `~/.claude/projects/-Volumes-Main-External-Development-lucidity-GalvoLite/memory/project_plan10_smoke_findings.md`
- Reference handler to mirror: `src/api/mod.rs:340-509` (`terminal_ws` + `handle_terminal_ws` funnel)
- Reference hook to mirror: `ui/src/hooks/useSessionEvents.ts`
- Reference WS test to mirror: `tests/api_event_broadcast.rs:96-150` (`ws_emits_domain_events_as_text_frames`)

---

## Task 1: `applies_to_project` discriminator + unit tests

**Files:**
- Modify: `src/api/events.rs:153-186` (add new method after existing `applies_to`)
- Test: `src/api/events.rs` (inline `#[cfg(test)] mod tests` at bottom)

- [ ] **Step 1: Write the failing unit tests**

Append to the bottom of `src/api/events.rs`:

```rust
#[cfg(test)]
mod applies_to_project_tests {
    use super::*;

    fn p() -> Uuid { Uuid::new_v4() }
    fn s() -> Uuid { Uuid::new_v4() }

    #[test]
    fn matches_issue_opened_on_same_project_id() {
        let pid = p();
        let evt = DomainEvent::IssueOpened {
            project_id: pid, issue_url: "u".into(), issue_number: 1, title: "t".into(),
        };
        assert!(evt.applies_to_project(pid));
        assert!(!evt.applies_to_project(Uuid::new_v4()));
    }

    #[test]
    fn matches_pull_request_merged_on_same_project_id() {
        let pid = p();
        let evt = DomainEvent::PullRequestMerged {
            project_id: pid, pr_url: "u".into(), pr_number: 1,
        };
        assert!(evt.applies_to_project(pid));
        assert!(!evt.applies_to_project(Uuid::new_v4()));
    }

    #[test]
    fn matches_session_pr_created_dual_scoped() {
        let pid = p();
        let sid = s();
        let evt = DomainEvent::SessionPrCreated {
            session_id: sid, project_id: pid, pr_url: "u".into(), pr_number: 1,
        };
        // Project-scoped match
        assert!(evt.applies_to_project(pid));
        // Session-scoped match (existing behavior preserved)
        assert!(evt.applies_to(sid));
        // Neither matches
        assert!(!evt.applies_to_project(Uuid::new_v4()));
        assert!(!evt.applies_to(Uuid::new_v4()));
    }

    #[test]
    fn matches_project_lifecycle_variants() {
        let pid = p();
        assert!(DomainEvent::ProjectAdded { project_id: pid }.applies_to_project(pid));
        assert!(DomainEvent::ProjectDeleted { project_id: pid }.applies_to_project(pid));
        assert!(DomainEvent::OverseerDegraded { project_id: pid, reason: "r".into() }
            .applies_to_project(pid));
        assert!(DomainEvent::OverseerRecovered { project_id: pid }.applies_to_project(pid));
    }

    #[test]
    fn does_not_match_session_only_variants() {
        let pid = p();
        let sid = s();
        let evt = DomainEvent::SessionRoleChanged { session_id: sid, role: "implementation".into() };
        assert!(!evt.applies_to_project(pid));
        let evt2 = DomainEvent::OperatorToast { session_id: sid, level: "info".into(), message: "x".into() };
        assert!(!evt2.applies_to_project(pid));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo test --lib applies_to_project 2>&1 | tail -20
```

Expected: compile error `no method named applies_to_project found for ...`.

- [ ] **Step 3: Implement `applies_to_project`**

Insert immediately after the `applies_to` method (before the closing `}` of `impl DomainEvent`) in `src/api/events.rs`:

```rust
    /// True if this event is scoped to (or fan-outs to) the given project.
    /// The per-project dashboard WebSocket uses this to filter the global
    /// event bus down to the per-project text-frame channel. The match is
    /// exhaustive (no wildcard arm) so future variants force a compile error
    /// here — opt in by extracting `project_id` from the payload.
    pub fn applies_to_project(&self, pid: Uuid) -> bool {
        match self {
            DomainEvent::IssueOpened { project_id, .. }
            | DomainEvent::PullRequestMerged { project_id, .. }
            | DomainEvent::SessionPrCreated { project_id, .. }
            | DomainEvent::ProjectAdded { project_id, .. }
            | DomainEvent::ProjectDeleted { project_id, .. }
            | DomainEvent::OverseerDegraded { project_id, .. }
            | DomainEvent::OverseerRecovered { project_id, .. }
            | DomainEvent::DirectiveChanged { project_id, .. }
            | DomainEvent::PostmortemWritten { project_id, .. }
            | DomainEvent::DecisionRecorded { project_id, .. }
            | DomainEvent::PhaseReviewNeeded { project_id, .. }
            | DomainEvent::PhaseAdvanced { project_id, .. }
            | DomainEvent::RecommendationCreated { project_id, .. } => *project_id == pid,
            DomainEvent::SessionStateChanged { .. }
            | DomainEvent::SessionEventAppended { .. }
            | DomainEvent::SessionRoleChanged { .. }
            | DomainEvent::SessionMetaChanged { .. }
            | DomainEvent::SessionPlanReplaced { .. }
            | DomainEvent::SessionPlanStepChanged { .. }
            | DomainEvent::OperatorPrompt { .. }
            | DomainEvent::OperatorToast { .. }
            | DomainEvent::SessionResumed { .. }
            | DomainEvent::SessionChildrenResolved { .. }
            | DomainEvent::RecommendationActed { .. } => false,
        }
    }
```

**Note on scope:** `DirectiveChanged`, `PostmortemWritten`, `DecisionRecorded`, `PhaseReviewNeeded`, `PhaseAdvanced`, `RecommendationCreated` all have a `project_id` field per `src/api/events.rs:70-89`. They're not on the 10f consumer list (`GithubPanel` ignores them), but they SHOULD be routed correctly through the WS so future consumers get them for free. `RecommendationActed` has no `project_id` and stays session/recommendation-scoped.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo test --lib applies_to_project 2>&1 | tail -10
```

Expected: `test result: ok. 5 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add src/api/events.rs
git commit -m "feat(events): add DomainEvent::applies_to_project discriminator

Counterpart to applies_to(session_id) but matches by project_id. Returns
true for any variant whose payload carries a matching project_id field
(github.*, project.*, overseer.*, session.pr_created, directive.changed,
postmortem.written, decision.recorded, phase.*, recommendation.created).

Will be used by the upcoming /api/projects/:id/events WS handler to
filter the global bus. Exhaustive match (no wildcard) so future variants
force a compile error here.

Plan 10f Task 1."
```

---

## Task 2: WS route + handler

**Files:**
- Modify: `src/api/mod.rs:88` (add new route line)
- Modify: `src/api/mod.rs:509` (add new handler functions after `handle_terminal_ws`)

- [ ] **Step 1: Add the route**

In `src/api/mod.rs`, find the line:

```rust
        .route("/api/github/webhook", post(webhook::webhook_handler))
```

Insert a new route line ABOVE it (so it appears with the other project routes):

```rust
        .route("/api/projects/:id/events", get(project_events_ws))
        .route("/api/github/webhook", post(webhook::webhook_handler))
```

- [ ] **Step 2: Add `project_events_ws` upgrade handler**

In `src/api/mod.rs`, immediately after the closing `}` of `handle_terminal_ws` (currently line ~509), insert:

```rust
/// `GET /api/projects/:id/events` — per-project DomainEvent WebSocket
/// (Plan 10f). Subscribes to the global event bus and fans out only
/// events for which `applies_to_project(id)` returns true. Unidirectional:
/// no client→server frames are processed (Close terminates the socket).
///
/// Returns 404 before the WS upgrade if the project id is unknown.
async fn project_events_ws(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    ws: axum::extract::WebSocketUpgrade,
) -> Response {
    if let Err(e) = mgr.db().get_project(id) {
        return err_to_response(e);
    }
    ws.on_upgrade(move |socket| handle_project_events_ws(socket, mgr, id))
        .into_response()
}

async fn handle_project_events_ws(
    socket: axum::extract::ws::WebSocket,
    mgr: AppState,
    project_id: uuid::Uuid,
) {
    use axum::extract::ws::Message;
    use futures_util::{sink::SinkExt, stream::StreamExt};

    let (mut sink, mut stream) = socket.split();

    // Funnel: writer_task owns the sink; event_forwarder + close_watcher
    // both push into a bounded mpsc. Matches the terminal_ws pattern but
    // without the PTY pump and without client-frame processing.
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Message>(256);

    let writer_task = async move {
        while let Some(msg) = frame_rx.recv().await {
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    };

    // Reader half: drain incoming frames so the socket terminates on Close.
    // Any non-Close inbound frame is silently dropped (no client→server
    // protocol on this WS).
    let close_watcher = async move {
        while let Some(Ok(msg)) = stream.next().await {
            if matches!(msg, Message::Close(_)) {
                break;
            }
        }
    };

    let mut event_rx = mgr.subscribe_events();
    let evt_tx = frame_tx.clone();
    let event_forwarder = async move {
        loop {
            match event_rx.recv().await {
                Ok(evt) => {
                    if !evt.applies_to_project(project_id) {
                        continue;
                    }
                    let json = match serde_json::to_string(&evt) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, "project-event serialization failed; dropping frame");
                            continue;
                        }
                    };
                    if evt_tx.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    // Drop the original sender so writer_task exits once all clones drop.
    drop(frame_tx);

    tokio::select! {
        _ = writer_task => {},
        _ = close_watcher => {},
        _ = event_forwarder => {},
    }
}
```

- [ ] **Step 3: Verify the file compiles**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo check --lib 2>&1 | tail -10
```

Expected: `Finished` with no errors. If `Path`, `State`, `Response`, `err_to_response`, or `AppState` aren't resolved, they are already imported at the top of `src/api/mod.rs` — no new imports needed.

- [ ] **Step 4: Verify clippy is clean for the new code**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo clippy --lib --tests 2>&1 | grep -E 'warning|error' | grep -v 'galvolite v0.1.0' | head -20
```

Expected: no new warnings from our changes. Pre-existing warnings unrelated to this plan are OK.

- [ ] **Step 5: Commit**

```bash
git add src/api/mod.rs
git commit -m "feat(api): per-project DomainEvent WebSocket at /api/projects/:id/events

Subscribes to the global event bus, filters by applies_to_project, and
fans out as text JSON frames. Reuses the terminal_ws funnel pattern
(single mpsc writer task) but unidirectional: no PTY pump, no
client→server frame processing (Close frames still terminate the socket
cleanly).

404 before upgrade if the project id is unknown (mgr.db().get_project
returns Error::ProjectNotFound which err_to_response maps to 404).

Closes the consumer gap surfaced by the 10a-10e smoke: 10e events
(IssueOpened, PullRequestMerged) and earlier project-scoped variants
(ProjectAdded, OverseerDegraded, etc.) can now reach dashboard
subscribers — terminal_ws filtered them out because they fail
applies_to(session_id).

Plan 10f Task 2."
```

---

## Task 3: Integration tests for the new WS

**Files:**
- Create: `tests/api_project_events.rs`

- [ ] **Step 1: Write the failing integration tests**

Create `tests/api_project_events.rs` with full contents:

```rust
mod common;

use futures_util::StreamExt;
use galvolite::api::events::DomainEvent;
use tokio_tungstenite::tungstenite::Message;

/// Helper — drain text frames until the first one matching `predicate`,
/// or panic on timeout after `secs`.
async fn await_text_frame<F>(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    secs: u64,
    predicate: F,
) -> serde_json::Value
where
    F: Fn(&serde_json::Value) -> bool,
{
    let fut = async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(t) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if predicate(&v) {
                        return v;
                    }
                }
                _ => continue,
            }
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(secs), fut)
        .await
        .expect("did not receive matching frame in time")
}

/// Helper — assert NO text frame matching `predicate` arrives within `ms`.
async fn assert_no_text_frame<F>(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    ms: u64,
    predicate: F,
) where
    F: Fn(&serde_json::Value) -> bool,
{
    let fut = async {
        loop {
            match ws.next().await.unwrap().unwrap() {
                Message::Text(t) => {
                    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                    if predicate(&v) {
                        panic!("unexpected matching frame arrived: {v}");
                    }
                }
                _ => continue,
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_millis(ms), fut).await;
    // timeout elapsing without panic is the success case
}

fn mk_project(app: &common::TestApp) -> uuid::Uuid {
    app.mgr
        .add_project(galvolite::db::NewProject {
            name: "p".into(),
            path: app._dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap()
        .id
}

#[tokio::test]
async fn project_ws_delivers_issue_opened_for_matched_project() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::IssueOpened {
        project_id: pid,
        issue_url: "https://github.com/o/r/issues/1".into(),
        issue_number: 1,
        title: "smoke".into(),
    });

    let v = await_text_frame(&mut ws, 5, |v| v["type"] == "github.issue_opened").await;
    assert_eq!(v["issue_number"], 1);
    assert_eq!(v["project_id"], pid.to_string());
}

#[tokio::test]
async fn project_ws_filters_out_events_for_other_project() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);
    let other_pid = uuid::Uuid::new_v4();

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::IssueOpened {
        project_id: other_pid,
        issue_url: "https://github.com/o/r/issues/99".into(),
        issue_number: 99,
        title: "other".into(),
    });

    assert_no_text_frame(&mut ws, 300, |v| v["type"] == "github.issue_opened").await;
}

#[tokio::test]
async fn project_ws_filters_out_session_only_events() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    // OperatorToast has only session_id, no project_id — must be dropped.
    app.mgr.event_bus().publish(DomainEvent::OperatorToast {
        session_id: uuid::Uuid::new_v4(),
        level: "info".into(),
        message: "should not arrive".into(),
    });

    assert_no_text_frame(&mut ws, 300, |v| v["type"] == "operator.toast").await;
}

#[tokio::test]
async fn project_ws_delivers_pull_request_merged() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::PullRequestMerged {
        project_id: pid,
        pr_url: "https://github.com/o/r/pull/42".into(),
        pr_number: 42,
    });

    let v = await_text_frame(&mut ws, 5, |v| v["type"] == "github.pull_request_merged").await;
    assert_eq!(v["pr_number"], 42);
    assert_eq!(v["project_id"], pid.to_string());
}

#[tokio::test]
async fn project_ws_delivers_session_pr_created_dual_scoped() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;
    let pid = mk_project(&app);

    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    app.mgr.event_bus().publish(DomainEvent::SessionPrCreated {
        session_id: uuid::Uuid::new_v4(),
        project_id: pid,
        pr_url: "https://github.com/o/r/pull/7".into(),
        pr_number: 7,
    });

    let v = await_text_frame(&mut ws, 5, |v| v["type"] == "session.pr_created").await;
    assert_eq!(v["pr_number"], 7);
    assert_eq!(v["project_id"], pid.to_string());
}

#[tokio::test]
async fn project_ws_returns_404_for_unknown_project_before_upgrade() {
    let app = common::test_app_with_bus();
    let port = common::serve_ephemeral(app.app.clone()).await;

    // Use a non-existent project id — server should refuse the WS upgrade with 404.
    let pid = uuid::Uuid::new_v4();
    let url = format!("ws://127.0.0.1:{port}/api/projects/{pid}/events");

    let result = tokio_tungstenite::connect_async(url).await;
    assert!(result.is_err(), "expected upgrade failure for unknown project, got success");
    let err = result.unwrap_err();
    let msg = format!("{err:?}");
    // tokio-tungstenite surfaces the 404 in the error
    assert!(msg.contains("404") || msg.contains("Http"), "expected 404 in error, got: {msg}");
}
```

- [ ] **Step 2: Run tests to verify they fail then pass**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo test --test api_project_events 2>&1 | tail -20
```

Expected after Task 2 (the route + handler are landed): all 6 tests pass. If they fail with `WsClient cannot connect`, add a 50ms `tokio::time::sleep` after `connect_async` in each test to give the subscribe loop a tick to attach — `subscribe_events()` is synchronous so this should not be needed, but call it out as a known-fix-if-flaky.

Run with serial threading to avoid port conflicts:

```bash
cd .worktrees/plan-10f-project-events-ws && cargo test --test api_project_events -- --test-threads=1 2>&1 | tail -20
```

Expected: `test result: ok. 6 passed; 0 failed`.

- [ ] **Step 3: Commit**

```bash
git add tests/api_project_events.rs
git commit -m "test(api): integration tests for /api/projects/:id/events WS

6 tests covering:
- IssueOpened matched-project delivery
- IssueOpened other-project filtered out
- session-only events (OperatorToast) filtered out
- PullRequestMerged matched-project delivery
- SessionPrCreated dual-scoped delivery (project subscribers receive it
  even though it's also session-scoped, validating the spec's intentional
  dual fan-out)
- unknown project_id → 404 before WS upgrade

Helpers: await_text_frame + assert_no_text_frame for clear timeout
semantics on each side.

Plan 10f Task 3."
```

---

## Task 4: UI hook `useProjectEvents` + tests

**Files:**
- Create: `ui/src/hooks/useProjectEvents.ts`
- Create: `ui/src/hooks/useProjectEvents.test.ts`
- Modify: `ui/src/api.ts:184-187` (add `projectEventsWsUrl` helper next to `terminalWsUrl`)

- [ ] **Step 1: Add the WS URL helper to api.ts**

In `ui/src/api.ts`, find:

```typescript
  terminalWsUrl: (id: string): string => {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}/api/sessions/${id}/terminal`;
  },
```

Add immediately after:

```typescript
  projectEventsWsUrl: (projectId: string): string => {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}/api/projects/${projectId}/events`;
  },
```

- [ ] **Step 2: Write the failing hook test**

Create `ui/src/hooks/useProjectEvents.test.ts`:

```typescript
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useProjectEvents } from "./useProjectEvents";

class MockWS {
  static all: MockWS[] = [];
  static last(): MockWS { return MockWS.all[MockWS.all.length - 1]; }
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: ((ev: { code: number }) => void) | null = null;
  onerror: (() => void) | null = null;
  readyState = 1;
  static OPEN = 1;
  closed = false;
  constructor(public url: string) { MockWS.all.push(this); }
  send() {}
  close() { this.closed = true; }
}

describe("useProjectEvents", () => {
  let originalWS: typeof WebSocket;
  beforeEach(() => {
    MockWS.all = [];
    originalWS = globalThis.WebSocket;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = MockWS;
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    (globalThis as any).WebSocket = originalWS;
  });

  it("opens a WS to the per-project events URL and forwards typed events", () => {
    const onEvent = vi.fn();
    renderHook(() => useProjectEvents("p1", { onEvent }));
    expect(MockWS.last().url).toContain("/api/projects/p1/events");
    act(() => {
      MockWS.last().onmessage!({
        data: JSON.stringify({
          type: "github.issue_opened",
          project_id: "p1",
          issue_url: "u",
          issue_number: 1,
          title: "t",
        }),
      } as MessageEvent);
    });
    expect(onEvent).toHaveBeenCalledWith(expect.objectContaining({
      type: "github.issue_opened",
      issue_number: 1,
    }));
  });

  it("drops malformed JSON silently", () => {
    const onEvent = vi.fn();
    renderHook(() => useProjectEvents("p1", { onEvent }));
    act(() => {
      MockWS.last().onmessage!({ data: "not json" } as MessageEvent);
    });
    expect(onEvent).not.toHaveBeenCalled();
  });

  it("closes the WS on unmount", () => {
    const { unmount } = renderHook(() => useProjectEvents("p1", { onEvent: vi.fn() }));
    const ws = MockWS.last();
    unmount();
    expect(ws.closed).toBe(true);
  });

  it("reconnects on unexpected close (code != 1000)", () => {
    renderHook(() => useProjectEvents("p1", { onEvent: vi.fn() }));
    const firstWs = MockWS.last();
    expect(MockWS.all.length).toBe(1);
    act(() => {
      firstWs.onclose!({ code: 1006 }); // abnormal close
    });
    act(() => {
      vi.advanceTimersByTime(1100); // first backoff = 1000ms
    });
    expect(MockWS.all.length).toBe(2);
  });

  it("does NOT reconnect on normal close (code = 1000)", () => {
    renderHook(() => useProjectEvents("p1", { onEvent: vi.fn() }));
    const firstWs = MockWS.last();
    act(() => {
      firstWs.onclose!({ code: 1000 });
    });
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    expect(MockWS.all.length).toBe(1);
  });

  it("calls onError after 10 failed reconnect attempts", () => {
    const onError = vi.fn();
    renderHook(() => useProjectEvents("p1", { onEvent: vi.fn(), onError }));
    // Fail 10 times in a row, advancing past each backoff
    for (let i = 0; i < 10; i++) {
      act(() => { MockWS.last().onclose!({ code: 1006 }); });
      act(() => { vi.advanceTimersByTime(20000); });
    }
    expect(onError).toHaveBeenCalledTimes(1);
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && npx vitest run src/hooks/useProjectEvents.test.ts 2>&1 | tail -15
```

Expected: `Failed to load url ./useProjectEvents` or `useProjectEvents is not a function`.

- [ ] **Step 4: Implement the hook**

Create `ui/src/hooks/useProjectEvents.ts`:

```typescript
import { useEffect, useRef } from "react";
import { api } from "../api";
import type { DomainEvent } from "../types";

interface UseProjectEventsOptions {
  onEvent: (e: DomainEvent) => void;
  onError?: (reason: string) => void;
}

/**
 * Subscribes to the per-project WebSocket (Plan 10f) and forwards
 * DomainEvent JSON text frames whose `applies_to_project(projectId)` is
 * true on the server side. Auto-reconnects on unexpected close with
 * exponential backoff (1s, 2s, 4s, 8s, then capped at 16s) up to 10
 * total attempts; after exhaustion calls `onError(reason)` once.
 *
 * Normal close (code 1000) is treated as intentional — no reconnect.
 *
 * The hook is keyed by `projectId`; passing null disables the
 * subscription and closes any open WS.
 */
export function useProjectEvents(
  projectId: string | null,
  { onEvent, onError }: UseProjectEventsOptions,
) {
  const onEventRef = useRef(onEvent);
  const onErrorRef = useRef(onError);
  onEventRef.current = onEvent;
  onErrorRef.current = onError;

  useEffect(() => {
    if (!projectId) return;

    let cancelled = false;
    let attempt = 0;
    let ws: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

    const open = () => {
      if (cancelled) return;
      ws = new WebSocket(api.projectEventsWsUrl(projectId));
      ws.onmessage = (ev) => {
        if (typeof ev.data !== "string") return;
        try {
          const parsed = JSON.parse(ev.data) as DomainEvent;
          if (parsed && typeof parsed === "object" && "type" in parsed) {
            onEventRef.current(parsed);
          }
        } catch {
          /* malformed; drop */
        }
      };
      ws.onclose = (ev) => {
        if (cancelled) return;
        // Normal close = no reconnect.
        if (ev.code === 1000) return;
        if (attempt >= 10) {
          onErrorRef.current?.(`reconnect exhausted after ${attempt} attempts`);
          return;
        }
        // Backoff schedule: 1s, 2s, 4s, 8s, then cap at 16s for any further attempts.
        const backoffMs = Math.min(16000, 1000 * 2 ** attempt);
        attempt += 1;
        reconnectTimer = setTimeout(open, backoffMs);
      };
    };

    open();

    return () => {
      cancelled = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      try { ws?.close(1000); } catch { /* already closed */ }
    };
  }, [projectId]);
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && npx vitest run src/hooks/useProjectEvents.test.ts 2>&1 | tail -15
```

Expected: `Test Files  1 passed (1)` and `Tests  6 passed (6)`.

- [ ] **Step 6: Commit**

```bash
git add ui/src/hooks/useProjectEvents.ts ui/src/hooks/useProjectEvents.test.ts ui/src/api.ts
git commit -m "feat(ui): useProjectEvents hook + WS URL helper

Subscribes to /api/projects/:id/events, parses DomainEvent text frames,
forwards typed events to onEvent callback. Auto-reconnects on
unexpected close (code != 1000) with exponential backoff (1s, 2s, 4s,
8s, then 16s cap) up to 10 attempts; onError called once after
exhaustion. Normal close (1000) is treated as intentional — no
reconnect — so unmount cleanup doesn't trigger a retry storm.

6 Vitest cases cover happy path, malformed JSON, unmount close,
reconnect-on-abnormal-close, no-reconnect-on-normal-close, and
reconnect-exhaustion → onError.

Plan 10f Task 4."
```

---

## Task 5: Wire `GithubPanel` to refetch on relevant events

**Files:**
- Modify: `ui/src/components/GithubPanel.tsx` (add hook + handler)

- [ ] **Step 1: Add the hook + handler to GithubPanel**

In `ui/src/components/GithubPanel.tsx`, find the imports at the top:

```typescript
import { useEffect, useState, useRef, useCallback } from "react";
import { api } from "../api";
import type { GithubPr, GithubIssue } from "../types";
```

Replace with:

```typescript
import { useEffect, useState, useRef, useCallback } from "react";
import { api } from "../api";
import { useProjectEvents } from "../hooks/useProjectEvents";
import type { GithubPr, GithubIssue, DomainEvent } from "../types";
```

Then find the existing `useEffect(() => { fetchAll(); ... }, [fetchAll])` block (around line 52). Immediately AFTER that effect, insert:

```typescript
  // Plan 10f: auto-refresh when GitHub events arrive for this project.
  useProjectEvents(projectId, {
    onEvent: useCallback((evt: DomainEvent) => {
      switch (evt.type) {
        case "github.issue_opened":
          fetchAll();
          break;
        case "github.pull_request_merged":
        case "session.pr_created":
          fetchAll();
          break;
        default:
          // Other project-scoped events (overseer.*, postmortem.*, etc.)
          // aren't surfaced by this panel.
          break;
      }
    }, [fetchAll]),
  });
```

- [ ] **Step 2: Verify the TS type for `DomainEvent` covers the events we switch on**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && grep -n 'github.issue_opened\|github.pull_request_merged\|session.pr_created' src/types.ts
```

Expected: at least 3 lines, one per discriminator. If any are missing (a known follow-up from Plan 10b status #2), add them to the `DomainEvent` union in `ui/src/types.ts` before this step compiles. The 10e merge added `github.*`; 10b added `session.pr_created`. Confirm with the grep before editing types.

If a variant is missing, append to the `DomainEvent` union in `ui/src/types.ts`:

```typescript
  | { type: "github.issue_opened"; project_id: string; issue_url: string; issue_number: number; title: string }
  | { type: "github.pull_request_merged"; project_id: string; pr_url: string; pr_number: number }
  | { type: "session.pr_created"; session_id: string; project_id: string; pr_url: string; pr_number: number }
```

Only add the lines that grep reports missing.

- [ ] **Step 3: Build the UI to verify TS compiles**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && npx tsc --noEmit 2>&1 | tail -10
```

Expected: no errors.

- [ ] **Step 4: Run the full UI test suite**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && npx vitest run 2>&1 | tail -15
```

Expected: all prior tests still pass + 6 new useProjectEvents tests. If any GithubPanel test breaks because the new hook fires a WS open in test context, the test was using a non-mocked WebSocket — those tests will need the MockWS shim from useProjectEvents.test.ts. Most existing tests already mock `api.*` functions, so the WS open is harmless (it errors but never affects state).

- [ ] **Step 5: Commit**

```bash
git add ui/src/components/GithubPanel.tsx ui/src/types.ts
git commit -m "feat(ui): GithubPanel auto-refreshes on github.* and session.pr_created events

Mounts useProjectEvents on the GithubPanel's projectId. Refetches both
the PR list and issue list when:
- github.issue_opened arrives (a new issue was opened externally)
- github.pull_request_merged arrives (a PR was merged externally)
- session.pr_created arrives (Plan 10b: a session inside this project
  just opened a PR via OpenPrModal)

Other project-scoped events (overseer.*, postmortem.*, decision.*,
phase.*, recommendation.created, directive.changed, project.added,
project.deleted) are not surfaced by this panel — they fan out through
the same WS for future consumers.

Manual Refresh button is unchanged.

Plan 10f Task 5 — final task."
```

---

## Task 6: Final verification + merge

- [ ] **Step 1: Full backend test suite**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo test --lib --tests -- --test-threads=1 2>&1 | tail -30
```

Expected: all tests pass. New green tests: 5 `applies_to_project_tests` + 6 `api_project_events`. No prior tests should regress.

- [ ] **Step 2: Full UI test suite**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && npx vitest run 2>&1 | tail -10
```

Expected: all prior UI tests pass + 6 new `useProjectEvents` tests.

- [ ] **Step 3: Clippy clean for our changes**

```bash
cd .worktrees/plan-10f-project-events-ws && cargo clippy --lib --tests 2>&1 | grep -E 'warning|error' | grep -v 'pre-existing' | head -10
```

Expected: no new warnings from `src/api/events.rs`, `src/api/mod.rs`, or `tests/api_project_events.rs`. Pre-existing warnings unrelated to the plan are OK.

- [ ] **Step 4: Build the UI bundle**

```bash
cd .worktrees/plan-10f-project-events-ws/ui && npm run build 2>&1 | tail -5
```

Expected: `built in <time>` with no errors.

- [ ] **Step 5: Quick E2E smoke via curl + websocat**

Follow the smoke pattern from `memory/project_plan10_smoke_findings.md`:

```bash
# In one terminal — boot server with env vars + the smoke repo from prior smoke:
cd .worktrees/plan-10f-project-events-ws
cargo build --quiet
GITHUB_TOKEN=<your-pat> GITHUB_WEBHOOK_SECRET=867dfc60823ca3445942fe72a930c952 \
  ./target/debug/galvolite serve --port 3811 &

# In another terminal — link a project and watch the WS:
PROJECT_ID=$(curl -s -X POST http://localhost:3811/api/projects \
  -H 'content-type: application/json' \
  -d '{"name":"10f-smoke","path":"/private/tmp/galvolite-smoke-dir","remote_url":null}' \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["id"])')
curl -s -X PUT "http://localhost:3811/api/projects/$PROJECT_ID/github/repo" \
  -H 'content-type: application/json' \
  -d '{"repo":"MichaelDanCurtis/GalvoLite"}'

# Watch the WS — should receive one frame after the webhook fires
(timeout 8 websocat -n1 "ws://localhost:3811/api/projects/$PROJECT_ID/events" > /tmp/10f-ws.log) &
sleep 1

# Fire a signed webhook (use the same SECRET from above)
SECRET=867dfc60823ca3445942fe72a930c952
BODY='{"action":"opened","issue":{"html_url":"https://github.com/MichaelDanCurtis/GalvoLite/issues/2","number":2,"title":"10f WS smoke"},"repository":{"full_name":"MichaelDanCurtis/GalvoLite"}}'
SIG=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -binary | xxd -p -c 256)
curl -s -X POST http://localhost:3811/api/github/webhook \
  -H "x-hub-signature-256: sha256=$SIG" \
  -H 'x-github-event: issues' \
  -H 'content-type: application/json' \
  -d "$BODY"
sleep 3
cat /tmp/10f-ws.log
# Expected: a JSON frame like {"type":"github.issue_opened","project_id":"<PROJECT_ID>","issue_url":"...","issue_number":2,"title":"10f WS smoke"}

pkill -f 'galvolite serve'
```

Expected: WS receives the `github.issue_opened` frame within ~1 second of the webhook firing.

- [ ] **Step 6: Merge to main**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git checkout main
git merge --no-ff plan-10f-project-events-ws -m "Merge branch 'plan-10f-project-events-ws' — Plan 10f Per-Project Events WS

Closes the consumer gap surfaced by the 10a-10e smoke. Project-scoped
DomainEvents (github.*, project.*, session.pr_created, overseer.*,
postmortem.*, decision.*, phase.*, recommendation.created,
directive.changed) now reach dashboard subscribers via a new
/api/projects/:id/events WebSocket. First consumer: GithubPanel
auto-refreshes its issue + PR lists on relevant events.

Backend: applies_to_project discriminator + new WS handler reusing the
terminal_ws funnel pattern. UI: useProjectEvents hook with exponential
backoff reconnect (1s, 2s, 4s, 8s, 16s cap, 10 max attempts).

Plan 10f. 5 tasks, 5 commits."
git branch -d plan-10f-project-events-ws
git worktree remove .worktrees/plan-10f-project-events-ws
git log --oneline -5
```

Expected: merge completes cleanly, branch deleted, worktree removed, last 5 commits show 10f progression on main.

- [ ] **Step 7: Write status memo + update MEMORY.md**

Write `~/.claude/projects/-Volumes-Main-External-Development-lucidity-GalvoLite/memory/project_plan10f_status.md` documenting the merge SHA, commit range, and key design calls. Append a new index entry to `MEMORY.md` pointing at it. Follow the shape of `project_plan10e_status.md` for consistency.

---

## Self-Review Notes (from writing-plans skill)

- **Spec coverage:** All 7 spec sections (architecture, locked design calls, error matrix, testing) map to Tasks 1-5. ✅
- **Placeholder scan:** Every code block contains the actual code. No "TBD" / "TODO" / "appropriate" / "etc.". ✅
- **Type consistency:** `applies_to_project(pid)` returns `bool` in both definition (Task 1) and call site (Task 2). `useProjectEvents(projectId, {onEvent, onError?})` shape matches the test in Task 4 and the consumer in Task 5. WS URL helper `projectEventsWsUrl` is defined in Task 4 step 1 and called by the hook in Task 4 step 4. ✅
- **The integration tests in Task 3 will compile only after Task 2 lands** (they import the running router via `serve_ephemeral`). The TDD order is: Task 1 unit tests pass → Task 2 wires the route → Task 3 integration tests pass against the live route. This is consistent with the prior 10a-10e pattern.
- **The UI hook tests in Task 4 will compile only after the hook file exists** (step 4 of that task). TDD order is preserved: step 2 writes the test (RED), step 4 writes the implementation (GREEN).
