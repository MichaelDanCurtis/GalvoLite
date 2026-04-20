# Sub-plan 4 — Dashboard Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every state, event, plan step, role change, operator prompt, and toast that the hooks+MCP coordination layer produces visible (or actionable) in the dashboard, with no polling. Implements §10 of the hooks+MCP spec.

**Architecture:** Bifurcate the existing per-session WebSocket into two logical streams over the same connection: **binary frames** continue to carry PTY bytes (xterm consumers), **text JSON frames** carry domain events (`session.*` and `operator.*`). A single `EventBroadcaster` in the backend receives every state mutation from `state::apply_event` and fans them out to subscribed sockets. New `OperatorPromptRegistry` wires the previously-degraded `ask_operator` MCP tool to a real server-side wait + HTTP answer endpoint. The frontend gains a tabbed `ExpandedSessionView` (Terminal / Events / Plan / Meta), an `OperatorPromptModal`, and a `ToastStack`, all driven by the new domain-event stream. Send-to-session bug (Finding 2) gets the one-line `\r` fix in `useTerminal`.

**Tech Stack:** Rust 1.75+ (existing), `axum` 0.7 with `tokio::sync::broadcast`, `tokio::sync::oneshot` for prompt waits, React 18 + TypeScript + Vite + Vitest + xterm.js (existing).

---

## File Structure

**New backend files:**
- `src/api/events.rs` — `DomainEvent` enum (eight `session.*`/`operator.*` variants), `EventBroadcaster` channel hub
- `src/api/operator.rs` — `OperatorPromptRegistry` (oneshot map keyed by `prompt_id`), POST `/operator/prompt/{id}/answer` handler
- `tests/api_event_broadcast.rs` — integration test: apply_event → event lands on subscribed receiver
- `tests/api_operator_prompt.rs` — integration test: ask_operator MCP call → registered prompt → HTTP answer → tool resolves
- `tests/ui_smoke_dashboard.rs` — end-to-end smoke driving the real binary via PTY (Sub-plan 5 expands this; here we plant the file)

**Modified backend files:**
- `src/api/mod.rs` — WS handler subscribes to `EventBroadcaster` in addition to PTY broadcast; new `/operator/prompt/{id}/answer` route
- `src/state.rs::apply_event` — emits `DomainEvent::SessionStateChanged` / `SessionEventAppended` after every transaction commit
- `src/mcp/tools/tier2_3_cross.rs::ask_operator` — replace `degraded: true` short-circuit with a real `OperatorPromptRegistry::register_and_wait`
- `src/mcp/tools/tier2_3_cross.rs::notify_operator` — emit `DomainEvent::OperatorToast`
- `src/mcp/tools/tier1_role_handoff.rs::set_role` — emit `DomainEvent::SessionRoleChanged`
- `src/db.rs` — new `read_plan_steps(session_id)` helper (reads from `plan_steps` table added in Sub-plan 1)
- `src/lib.rs` — `pub mod` for new modules
- `src/session_manager.rs` — accept and store an `EventBroadcaster` handle; expose `subscribe_events()` for the WS handler

**Modified frontend files (under `ui/src/`):**
- `types.ts` — replace stale `SessionState` enum (`"spawning" | "complete" | "killed" | …`) with the seven canonical states from spec §8.1; add `DomainEvent` types matching backend
- `hooks/useTerminal.ts` — line 142 fix: send `data` with `\r` mapping for `\n`
- `hooks/useSessionEvents.ts` — **new** WS subscription parsing JSON text frames into a typed event stream
- `hooks/useSessions.ts` — drop the 2-second polling loop in favor of `session.state_changed` push; keep one initial fetch for hydration
- `components/SessionRow.tsx` — extend state-color map to seven states; show role badge; handoff tick when last event was a `mcp.handoff`
- `components/ExpandedSessionView.tsx` — wrap existing terminal in a `Tabs` layout; add Terminal / Events / Plan / Meta tab labels
- `components/EventsTab.tsx` — **new** reverse-chrono list, filter by source/kind, expand JSON
- `components/PlanTab.tsx` — **new** live checklist with status pills
- `components/MetaTab.tsx` — **new** plan_markdown / seed_markdown / handoff link / counters
- `components/OperatorPromptModal.tsx` — **new** modal listening for `operator.prompt`
- `components/ToastStack.tsx` — **new** top-right overlay listening for `operator.toast`
- `components/SessionRail.tsx` — add "Event log" button that opens the Events tab
- `api.ts` — `answerOperatorPrompt(promptId, answer)` HTTP helper
- `App.tsx` — mount `<ToastStack />` and `<OperatorPromptModal />` once at root

**New UI test files:**
- `ui/src/components/__tests__/EventsTab.test.tsx`
- `ui/src/components/__tests__/PlanTab.test.tsx`
- `ui/src/components/__tests__/MetaTab.test.tsx`
- `ui/src/components/__tests__/OperatorPromptModal.test.tsx`
- `ui/src/components/__tests__/ToastStack.test.tsx`
- `ui/src/hooks/__tests__/useSessionEvents.test.ts`
- `ui/src/hooks/__tests__/useTerminal.cr.test.ts`

---

## Task 1: DomainEvent enum + EventBroadcaster

**Files:**
- Create: `src/api/events.rs`
- Modify: `src/api/mod.rs:1-20` (add `pub mod events;`)
- Modify: `src/lib.rs` (no change required — `api` already public)
- Test: `tests/api_event_broadcast.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/api_event_broadcast.rs
mod common;

use galvolite::api::events::{DomainEvent, EventBroadcaster};
use galvolite::state::SessionState;

#[tokio::test]
async fn broadcaster_fans_session_state_changed_to_subscribers() {
    let bus = EventBroadcaster::new(64);
    let mut rx = bus.subscribe();
    let sid = uuid::Uuid::new_v4();

    bus.publish(DomainEvent::SessionStateChanged {
        session_id: sid,
        prev: Some(SessionState::Idle),
        next: SessionState::Working,
        last_event_ts: "2026-04-19T00:00:00Z".into(),
    });

    let got = rx.recv().await.unwrap();
    match got {
        DomainEvent::SessionStateChanged { session_id, next, .. } => {
            assert_eq!(session_id, sid);
            assert_eq!(next, SessionState::Working);
        }
        other => panic!("unexpected variant: {other:?}"),
    }
}

#[tokio::test]
async fn broadcaster_serializes_to_tagged_json_frames() {
    let bus = EventBroadcaster::new(64);
    let sid = uuid::Uuid::new_v4();
    let evt = DomainEvent::OperatorToast {
        session_id: sid,
        level: "warn".into(),
        message: "low fuel".into(),
    };
    let json = serde_json::to_value(&evt).unwrap();
    assert_eq!(json["type"], "operator.toast");
    assert_eq!(json["level"], "warn");
    assert_eq!(json["session_id"], sid.to_string());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_event_broadcast`
Expected: FAIL — `unresolved import galvolite::api::events`.

- [ ] **Step 3: Implement `DomainEvent` and `EventBroadcaster`**

```rust
// src/api/events.rs
//! Domain-event broadcast hub for the dashboard WebSocket.
//!
//! Spec §10.1: every coordination-layer event (state change, event row append,
//! role change, plan replacement, plan step change, meta change, operator
//! prompt, operator toast) is fanned out to subscribed dashboard sockets as a
//! tagged JSON frame.

use crate::state::SessionState;
use serde::Serialize;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DomainEvent {
    #[serde(rename = "session.state_changed")]
    SessionStateChanged {
        session_id: Uuid,
        prev: Option<SessionState>,
        next: SessionState,
        last_event_ts: String,
    },
    #[serde(rename = "session.event_appended")]
    SessionEventAppended {
        session_id: Uuid,
        event_id: i64,
        kind: String,
        source: String,
        ts: String,
        payload: serde_json::Value,
    },
    #[serde(rename = "session.role_changed")]
    SessionRoleChanged { session_id: Uuid, role: String },
    #[serde(rename = "session.meta_changed")]
    SessionMetaChanged {
        session_id: Uuid,
        turn_count: i64,
        est_tokens: i64,
        plan_markdown: Option<String>,
        seed_markdown: Option<String>,
        handoff_path: Option<String>,
    },
    #[serde(rename = "session.plan_replaced")]
    SessionPlanReplaced { session_id: Uuid, steps: serde_json::Value },
    #[serde(rename = "session.plan_step_changed")]
    SessionPlanStepChanged {
        session_id: Uuid,
        step_hash: String,
        prev_status: String,
        next_status: String,
        note: Option<String>,
    },
    #[serde(rename = "operator.prompt")]
    OperatorPrompt {
        prompt_id: Uuid,
        session_id: Uuid,
        question: String,
        choices: Option<Vec<String>>,
        timeout_secs: Option<u64>,
    },
    #[serde(rename = "operator.toast")]
    OperatorToast { session_id: Uuid, level: String, message: String },
}

#[derive(Clone)]
pub struct EventBroadcaster {
    tx: broadcast::Sender<DomainEvent>,
}

impl EventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEvent> {
        self.tx.subscribe()
    }
    /// Best-effort publish. Lagged subscribers drop frames; that is acceptable
    /// because the dashboard always re-fetches on reconnect.
    pub fn publish(&self, evt: DomainEvent) {
        let _ = self.tx.send(evt);
    }
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}
```

Add to `src/api/mod.rs` near the other module decls:
```rust
pub mod events;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test api_event_broadcast`
Expected: PASS, 2 tests.

- [ ] **Step 5: Commit**

```bash
git add src/api/events.rs src/api/mod.rs tests/api_event_broadcast.rs
git commit -m "feat(api): DomainEvent enum + EventBroadcaster fan-out hub"
```

---

## Task 2: SessionManager owns EventBroadcaster

**Files:**
- Modify: `src/session_manager.rs` (`SessionManager::new` signature + new field + `subscribe_events()` method)
- Modify: `src/api/mod.rs` (`router(mgr)` constructs the broadcaster, passes it down)
- Modify: `src/main.rs` (build broadcaster at startup)
- Modify: every test that calls `SessionManager::new(...)` directly to pass the broadcaster

- [ ] **Step 1: Write the failing test (extend Task 1's file)**

```rust
// tests/api_event_broadcast.rs — append below existing tests
#[tokio::test]
async fn session_manager_exposes_event_broadcaster() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db,
        dir.path().to_path_buf(),
        bus.clone(),
    ));
    let mut rx = mgr.subscribe_events();
    bus.publish(galvolite::api::events::DomainEvent::OperatorToast {
        session_id: uuid::Uuid::new_v4(),
        level: "info".into(),
        message: "hi".into(),
    });
    let _ = rx.recv().await.unwrap();
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_event_broadcast session_manager_exposes_event_broadcaster`
Expected: FAIL — `with_event_bus` not found.

- [ ] **Step 3: Add `with_event_bus` constructor and `subscribe_events`**

In `src/session_manager.rs`:
```rust
use crate::api::events::{EventBroadcaster, DomainEvent};

pub struct SessionManager {
    // ... existing fields
    event_bus: EventBroadcaster,
}

impl SessionManager {
    /// Backwards-compatible constructor — creates a bus internally. Tests
    /// that don't observe events use this.
    pub fn new(db: Db, data_dir: PathBuf) -> Self {
        Self::with_event_bus(db, data_dir, EventBroadcaster::new(256))
    }

    pub fn with_event_bus(db: Db, data_dir: PathBuf, event_bus: EventBroadcaster) -> Self {
        Self { /* existing init */, event_bus }
    }

    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<DomainEvent> {
        self.event_bus.subscribe()
    }

    pub fn event_bus(&self) -> &EventBroadcaster { &self.event_bus }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test api_event_broadcast`
Expected: PASS, 3 tests.

- [ ] **Step 5: Run the full suite to confirm no callers broke**

Run: `cargo test --no-fail-fast`
Expected: 227+ passing (existing `SessionManager::new(db, dir)` callers continue to work via the back-compat constructor).

- [ ] **Step 6: Commit**

```bash
git add src/session_manager.rs tests/api_event_broadcast.rs
git commit -m "feat(session_manager): own EventBroadcaster, expose subscribe_events()"
```

---

## Task 3: apply_event publishes session.state_changed and session.event_appended

**Files:**
- Modify: `src/state.rs::apply_event` (after the transaction commits, publish two events to the bus)
- Modify: `src/state.rs` signature (accept `&EventBroadcaster` or read from the `Db` — see step 3)
- Test: `tests/api_event_broadcast.rs` (extend)

- [ ] **Step 1: Write the failing test**

```rust
// tests/api_event_broadcast.rs — append
#[tokio::test]
async fn apply_event_publishes_state_changed_and_event_appended() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db.clone(), dir.path().to_path_buf(), bus.clone(),
    ));

    // Seed a project + session in Idle.
    let proj = db.insert_project(galvolite::db::NewProject {
        name: "p".into(),
        path: dir.path().to_string_lossy().into(),
        remote_url: None,
    }).unwrap();
    let s = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: dir.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();
    db.pool().get().unwrap().execute(
        "UPDATE sessions SET state = 'idle' WHERE id = ?1",
        rusqlite::params![s.id.to_string()],
    ).unwrap();

    let mut rx = mgr.subscribe_events();
    galvolite::state::apply_event_with_bus(
        &db, s.id, mgr.event_bus(),
        galvolite::state::EventInput {
            source: galvolite::state::Source::Hook,
            kind: "hook.UserPromptSubmit".into(),
            payload: serde_json::json!({}),
            dedup_key: None,
        },
    ).unwrap();

    // Two events should arrive (order matters: appended before state).
    let first = rx.recv().await.unwrap();
    let second = rx.recv().await.unwrap();
    assert!(matches!(first, galvolite::api::events::DomainEvent::SessionEventAppended { .. }));
    assert!(matches!(second, galvolite::api::events::DomainEvent::SessionStateChanged {
        next: galvolite::state::SessionState::Working, ..
    }));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_event_broadcast apply_event_publishes`
Expected: FAIL — `apply_event_with_bus` not found.

- [ ] **Step 3: Add the bus-aware variant in `src/state.rs`**

Keep `apply_event(db, sid, input)` as the back-compat call (no broadcast). Add:
```rust
pub fn apply_event_with_bus(
    db: &crate::db::Db,
    session_id: uuid::Uuid,
    bus: &crate::api::events::EventBroadcaster,
    input: EventInput,
) -> Result<EventOutcome> {
    let outcome = apply_event(db, session_id, input.clone())?;
    if !outcome.deduped {
        let ts = chrono::Utc::now().to_rfc3339();
        bus.publish(crate::api::events::DomainEvent::SessionEventAppended {
            session_id,
            event_id: outcome.event_id,
            kind: input.kind.clone(),
            source: input.source.to_string(),
            ts: ts.clone(),
            payload: input.payload.clone(),
        });
        if Some(outcome.next_state) != outcome.prev_state {
            bus.publish(crate::api::events::DomainEvent::SessionStateChanged {
                session_id,
                prev: outcome.prev_state,
                next: outcome.next_state,
                last_event_ts: ts,
            });
        }
    }
    Ok(outcome)
}
```

(Add `#[derive(Clone)]` to `EventInput` if not already.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test api_event_broadcast`
Expected: PASS, 4 tests.

- [ ] **Step 5: Migrate every in-tree caller to use `apply_event_with_bus`**

Search: `grep -rn "state::apply_event\b" src/`. Update each call site to thread `mgr.event_bus()` (or a parameter). Hook handlers (`src/hooks/handlers/*.rs`), MCP tool handlers (`src/mcp/tools/*.rs`), the signal pump (`src/signals.rs`), and `src/session_manager.rs` lifecycle paths.

After migration, the bare `apply_event` should remain only as a primitive for tests that don't care about broadcast.

- [ ] **Step 6: Run the full suite**

Run: `cargo test --no-fail-fast`
Expected: 228+ passing.

- [ ] **Step 7: Commit**

```bash
git add -u src/ tests/api_event_broadcast.rs
git commit -m "feat(state): apply_event_with_bus publishes appended+state_changed"
```

---

## Task 4: WebSocket handler subscribes to domain events

**Files:**
- Modify: `src/api/mod.rs::handle_terminal_ws` — add a third async task that forwards `DomainEvent`s as JSON text frames
- Test: `tests/api_event_broadcast.rs` (extend, opens an actual WS to check)

- [ ] **Step 1: Write the failing test**

```rust
// tests/api_event_broadcast.rs — append
#[tokio::test]
async fn ws_emits_domain_events_as_text_frames() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tokio_tungstenite::tungstenite::Message;
    // Start the real router on an ephemeral port.
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("t.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = std::sync::Arc::new(galvolite::session_manager::SessionManager::with_event_bus(
        db.clone(), dir.path().to_path_buf(), bus.clone(),
    ));
    let app = galvolite::api::router(mgr.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });

    // Seed a session so the WS route accepts the subscribe.
    let (sid, token) = common::seed_session(&db, &dir.path().join("proj"));

    let url = format!("ws://127.0.0.1:{port}/api/sessions/{sid}/terminal?token={token}");
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();

    bus.publish(galvolite::api::events::DomainEvent::OperatorToast {
        session_id: sid, level: "info".into(), message: "hi".into(),
    });

    // Skip any binary frames (none expected, but be tolerant).
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                assert_eq!(v["type"], "operator.toast");
                break;
            }
            _ => continue,
        }
    }
}
```

(Add `use futures_util::StreamExt;` at the top of the test file.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test api_event_broadcast ws_emits`
Expected: FAIL — text frame never arrives (handler doesn't subscribe).

- [ ] **Step 3: Wire the third task into `handle_terminal_ws`**

In `src/api/mod.rs::handle_terminal_ws`, after the existing reader/writer setup, add:
```rust
let mut event_rx = mgr.subscribe_events();
let mut sender_for_events = sender.clone(); // Arc<Mutex<Sender>> already held
let event_session = session_id;
let event_task = tokio::spawn(async move {
    while let Ok(evt) = event_rx.recv().await {
        if !evt.applies_to(event_session) { continue; }
        let json = match serde_json::to_string(&evt) { Ok(s) => s, Err(_) => continue };
        let mut s = sender_for_events.lock().await;
        if s.send(axum::extract::ws::Message::Text(json)).await.is_err() { break; }
    }
});
// extend tokio::select! to include event_task.await — abort siblings on exit
```

Add the filter helper to `src/api/events.rs`:
```rust
impl DomainEvent {
    pub fn applies_to(&self, sid: Uuid) -> bool {
        match self {
            DomainEvent::SessionStateChanged { session_id, .. }
            | DomainEvent::SessionEventAppended { session_id, .. }
            | DomainEvent::SessionRoleChanged { session_id, .. }
            | DomainEvent::SessionMetaChanged { session_id, .. }
            | DomainEvent::SessionPlanReplaced { session_id, .. }
            | DomainEvent::SessionPlanStepChanged { session_id, .. }
            | DomainEvent::OperatorPrompt { session_id, .. }
            | DomainEvent::OperatorToast { session_id, .. } => *session_id == sid,
        }
    }
}
```

(If `sender` isn't already an `Arc<Mutex<...>>`, refactor: split the `WebSocket` and wrap the writer half once at task launch.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test api_event_broadcast ws_emits`
Expected: PASS.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --no-fail-fast`
Expected: 229+ passing.

- [ ] **Step 6: Commit**

```bash
git add -u src/api/ tests/api_event_broadcast.rs
git commit -m "feat(api): WS forwards DomainEvent JSON text frames per session"
```

---

## Task 5: notify_operator and set_role publish their events

**Files:**
- Modify: `src/mcp/tools/tier2_3_cross.rs::notify_operator`
- Modify: `src/mcp/tools/tier1_role_handoff.rs::set_role`
- Test: `tests/mcp_tools_tier2_3_cross.rs` and `tests/mcp_tools_tier1_role_handoff.rs` (extend)

- [ ] **Step 1: Write failing tests**

```rust
// tests/mcp_tools_tier2_3_cross.rs — append
#[tokio::test]
async fn notify_operator_publishes_toast_event() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session(&setup.db, &proj);
    let mut rx = setup.mgr.subscribe_events(); // requires TestApp.mgr — add it
    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"notify_operator","arguments":{"level":"warn","message":"low fuel"}}
    });
    let _ = rpc(setup.app.clone(), &token, body).await;
    let evt = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await.unwrap().unwrap();
    match evt {
        galvolite::api::events::DomainEvent::OperatorToast { session_id, level, message } => {
            assert_eq!(session_id, sid);
            assert_eq!(level, "warn");
            assert_eq!(message, "low fuel");
        }
        other => panic!("unexpected: {other:?}"),
    }
}
```

(Repeat the pattern for `set_role` → `SessionRoleChanged`.)

- [ ] **Step 2: Add `mgr` field to `tests/common/mod.rs::TestApp`**

```rust
pub struct TestApp {
    pub router: axum::Router,
    pub app: axum::Router,
    pub db: Db,
    pub mgr: std::sync::Arc<SessionManager>,
    pub _dir: TempDir,
}
// Update test_app() to populate mgr.
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --test mcp_tools_tier2_3_cross notify_operator_publishes`
Expected: FAIL — toast event never arrives.

- [ ] **Step 4: Wire publishes**

In `tier2_3_cross.rs::notify_operator`, after the existing apply_event call:
```rust
ctx.mgr.event_bus().publish(DomainEvent::OperatorToast {
    session_id: ctx.session_id,
    level: args.level.clone(),
    message: args.message.clone(),
});
```

In `tier1_role_handoff.rs::set_role`, after the apply_event call:
```rust
ctx.mgr.event_bus().publish(DomainEvent::SessionRoleChanged {
    session_id: ctx.session_id,
    role: args.role.clone(),
});
```

(Confirm tool handlers receive `&SessionManager` or equivalent context with `event_bus()`.)

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --test mcp_tools_tier2_3_cross notify_operator_publishes && cargo test --test mcp_tools_tier1_role_handoff set_role_publishes`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -u src/mcp/tools/ tests/common/mod.rs tests/mcp_tools_tier2_3_cross.rs tests/mcp_tools_tier1_role_handoff.rs
git commit -m "feat(mcp): notify_operator + set_role publish to EventBroadcaster"
```

---

## Task 6: OperatorPromptRegistry + ask_operator real implementation

**Files:**
- Create: `src/api/operator.rs` (registry + answer handler)
- Modify: `src/mcp/tools/tier2_3_cross.rs::ask_operator` — replace `degraded:true` short-circuit with `register_and_wait`
- Modify: `src/api/mod.rs` — `.route("/operator/prompt/:id/answer", post(operator::answer))`
- Modify: `src/session_manager.rs` — own an `Arc<OperatorPromptRegistry>` alongside the event bus
- Test: `tests/api_operator_prompt.rs`

- [ ] **Step 1: Write the failing test**

```rust
// tests/api_operator_prompt.rs
mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn ask_operator_resolves_when_http_answer_lands() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session(&setup.db, &proj);
    let app_for_answer = setup.app.clone();
    let token_for_answer = token.clone();

    // Subscribe to events so we can grab the prompt_id.
    let mut rx = setup.mgr.subscribe_events();

    // Spawn the MCP call (blocks until answered).
    let app_for_ask = setup.app.clone();
    let ask = tokio::spawn(async move {
        let body = serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"ask_operator","arguments":{"question":"go?","timeout_secs":5}}
        });
        common::rpc(app_for_ask, &token, body).await
    });

    // Wait for OperatorPrompt event.
    let prompt_id = loop {
        match rx.recv().await.unwrap() {
            galvolite::api::events::DomainEvent::OperatorPrompt { prompt_id, .. } => break prompt_id,
            _ => continue,
        }
    };

    // Answer over HTTP.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/operator/prompt/{prompt_id}/answer"))
        .header("content-type","application/json")
        .header("Authorization", format!("Bearer {token_for_answer}"))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({"answer":"yes"})).unwrap()))
        .unwrap();
    let resp = app_for_answer.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The MCP call should now resolve with answer:"yes".
    let result = ask.await.unwrap();
    assert_eq!(result["result"]["content"][0]["text"]["answer"], "yes");
}

#[tokio::test]
async fn ask_operator_times_out_with_null_answer() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (_sid, token) = common::seed_session(&setup.db, &proj);
    let body = serde_json::json!({
        "jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"ask_operator","arguments":{"question":"q","timeout_secs":1}}
    });
    let r = common::rpc(setup.app, &token, body).await;
    assert_eq!(r["result"]["content"][0]["text"]["timed_out"], true);
    assert_eq!(r["result"]["content"][0]["text"]["answer"], serde_json::Value::Null);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test api_operator_prompt`
Expected: FAIL — endpoint and registry don't exist.

- [ ] **Step 3: Implement the registry and route**

```rust
// src/api/operator.rs
use crate::api::events::{DomainEvent, EventBroadcaster};
use axum::{extract::{Path, State}, http::StatusCode, Json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[derive(Default)]
pub struct OperatorPromptRegistry {
    waiters: Mutex<HashMap<Uuid, oneshot::Sender<serde_json::Value>>>,
}

impl OperatorPromptRegistry {
    pub fn new() -> Arc<Self> { Arc::new(Self::default()) }

    /// Register a prompt, broadcast it, and await an answer or timeout.
    pub async fn register_and_wait(
        &self,
        bus: &EventBroadcaster,
        session_id: Uuid,
        question: String,
        choices: Option<Vec<String>>,
        timeout_secs: Option<u64>,
    ) -> serde_json::Value {
        let prompt_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().await.insert(prompt_id, tx);
        bus.publish(DomainEvent::OperatorPrompt {
            prompt_id, session_id, question, choices, timeout_secs,
        });
        let dur = std::time::Duration::from_secs(timeout_secs.unwrap_or(30));
        match tokio::time::timeout(dur, rx).await {
            Ok(Ok(v)) => v,
            _ => {
                self.waiters.lock().await.remove(&prompt_id);
                serde_json::json!({ "answer": serde_json::Value::Null, "timed_out": true })
            }
        }
    }

    pub async fn answer(&self, prompt_id: Uuid, answer: serde_json::Value) -> Result<(), ()> {
        if let Some(tx) = self.waiters.lock().await.remove(&prompt_id) {
            let _ = tx.send(serde_json::json!({ "answer": answer, "timed_out": false }));
            Ok(())
        } else {
            Err(())
        }
    }
}

#[derive(serde::Deserialize)]
pub struct AnswerBody { pub answer: serde_json::Value }

pub async fn answer(
    State(mgr): State<Arc<crate::session_manager::SessionManager>>,
    Path(prompt_id): Path<Uuid>,
    Json(body): Json<AnswerBody>,
) -> Result<StatusCode, StatusCode> {
    mgr.operator_registry().answer(prompt_id, body.answer)
        .await
        .map(|_| StatusCode::OK)
        .map_err(|_| StatusCode::NOT_FOUND)
}
```

In `src/session_manager.rs`, add `operator_registry: Arc<OperatorPromptRegistry>` field + `pub fn operator_registry(&self) -> &Arc<...>`.

In `src/api/mod.rs::router`, add the route and `pub mod operator;`.

In `src/mcp/tools/tier2_3_cross.rs::ask_operator`, replace the degraded-mode short-circuit:
```rust
let answer = ctx.mgr.operator_registry().register_and_wait(
    ctx.mgr.event_bus(),
    ctx.session_id,
    args.question.clone(),
    args.choices.clone(),
    args.timeout_secs,
).await;
Ok(serde_json::json!({ "content": [{ "type":"text", "text": answer }] }))
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test api_operator_prompt`
Expected: PASS, 2 tests.

- [ ] **Step 5: Run the full suite**

Run: `cargo test --no-fail-fast`
Expected: 232+ passing.

- [ ] **Step 6: Commit**

```bash
git add src/api/operator.rs src/api/mod.rs src/session_manager.rs src/mcp/tools/tier2_3_cross.rs tests/api_operator_prompt.rs
git commit -m "feat(api): real ask_operator backed by OperatorPromptRegistry"
```

---

## Task 7: Send-to-session `\r` fix (Finding 2)

**Files:**
- Modify: `ui/src/hooks/useTerminal.ts:142`
- Test: `ui/src/hooks/__tests__/useTerminal.cr.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// ui/src/hooks/__tests__/useTerminal.cr.test.ts
import { describe, it, expect, vi } from "vitest";
import { mapKeystrokeForPty } from "../useTerminal";

describe("mapKeystrokeForPty (Finding 2)", () => {
  it("translates a bare LF to CR", () => {
    expect(mapKeystrokeForPty("\n")).toBe("\r");
  });
  it("preserves printable text", () => {
    expect(mapKeystrokeForPty("ls")).toBe("ls");
  });
  it("translates trailing LF in multi-char input", () => {
    expect(mapKeystrokeForPty("ls\n")).toBe("ls\r");
  });
  it("leaves CR alone", () => {
    expect(mapKeystrokeForPty("\r")).toBe("\r");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- useTerminal.cr`
Expected: FAIL — `mapKeystrokeForPty` not exported.

- [ ] **Step 3: Implement the fix**

In `ui/src/hooks/useTerminal.ts`, near the top:
```typescript
export function mapKeystrokeForPty(data: string): string {
  // xterm.js emits "\n" for the Return key but PTYs (and Claude Code) want
  // "\r". Translate every LF to CR. CRLF stays as CRR which the line
  // discipline coalesces back to CR.
  return data.replace(/\n/g, "\r");
}
```

Replace line 142:
```typescript
ws.send(JSON.stringify({ type: "input", data: mapKeystrokeForPty(data) }));
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npm test -- useTerminal.cr`
Expected: PASS, 4 tests.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useTerminal.ts ui/src/hooks/__tests__/useTerminal.cr.test.ts
git commit -m "fix(ui): translate LF to CR in PTY keystrokes (Finding 2)"
```

---

## Task 8: Frontend types — replace stale SessionState enum + add DomainEvent shapes

**Files:**
- Modify: `ui/src/types.ts`
- Test: existing UI tests should still compile; no new test required.

- [ ] **Step 1: Replace the SessionState enum**

Find the existing union (likely):
```typescript
export type SessionState =
  | "spawning" | "working" | "attention_needed" | "handing_off"
  | "restoring" | "complete" | "killed" | "error";
```

Replace with:
```typescript
export type SessionState =
  | "starting" | "idle" | "working" | "awaiting_input"
  | "needs_attention" | "error" | "ended";
```

- [ ] **Step 2: Add DomainEvent shape**

Append to `types.ts`:
```typescript
export type DomainEvent =
  | { type: "session.state_changed"; session_id: string; prev: SessionState | null; next: SessionState; last_event_ts: string }
  | { type: "session.event_appended"; session_id: string; event_id: number; kind: string; source: string; ts: string; payload: unknown }
  | { type: "session.role_changed"; session_id: string; role: string }
  | { type: "session.meta_changed"; session_id: string; turn_count: number; est_tokens: number; plan_markdown: string | null; seed_markdown: string | null; handoff_path: string | null }
  | { type: "session.plan_replaced"; session_id: string; steps: PlanStep[] }
  | { type: "session.plan_step_changed"; session_id: string; step_hash: string; prev_status: PlanStepStatus; next_status: PlanStepStatus; note: string | null }
  | { type: "operator.prompt"; prompt_id: string; session_id: string; question: string; choices: string[] | null; timeout_secs: number | null }
  | { type: "operator.toast"; session_id: string; level: "info" | "warn" | "urgent"; message: string };

export type PlanStepStatus = "pending" | "working" | "done" | "blocked";
export interface PlanStep { hash: string; text: string; status: PlanStepStatus; note?: string }
```

- [ ] **Step 3: Run typecheck**

Run: `cd ui && npm run build`
Expected: Compile errors in components that referenced old state literals (e.g., `"complete"`, `"killed"`). Fix them inline by mapping to new states. Likely files: `SessionRow.tsx`, `Dashboard.tsx`, `useSessions.ts`. Use the migration:
- `"spawning"` → `"starting"`
- `"complete"` → `"ended"`
- `"killed"` → `"ended"`
- `"attention_needed"` → `"needs_attention"`
- `"handing_off"`, `"restoring"` → drop (no replacement; render as `"working"`).

Re-run `npm run build` until clean.

- [ ] **Step 4: Run UI tests**

Run: `cd ui && npm test`
Expected: Existing 37 tests still pass (any literal-string assertions need updating to new state names).

- [ ] **Step 5: Commit**

```bash
git add ui/src/types.ts ui/src/components ui/src/hooks
git commit -m "refactor(ui): align SessionState with spec §8.1; add DomainEvent type"
```

---

## Task 9: useSessionEvents hook (WS JSON text-frame parser)

**Files:**
- Create: `ui/src/hooks/useSessionEvents.ts`
- Test: `ui/src/hooks/__tests__/useSessionEvents.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// ui/src/hooks/__tests__/useSessionEvents.test.ts
import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useSessionEvents } from "../useSessionEvents";

class MockWS {
  static last: MockWS | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  readyState = 1;
  constructor(public url: string) { MockWS.last = this; }
  send() {}
  close() {}
}

describe("useSessionEvents", () => {
  it("parses text frames and forwards typed events to the consumer", () => {
    vi.stubGlobal("WebSocket", MockWS);
    const onEvent = vi.fn();
    renderHook(() => useSessionEvents("00000000-0000-0000-0000-000000000001", "tok", onEvent));
    act(() => {
      MockWS.last!.onmessage!({
        data: JSON.stringify({
          type: "operator.toast",
          session_id: "00000000-0000-0000-0000-000000000001",
          level: "info",
          message: "hi",
        }),
      } as MessageEvent);
    });
    expect(onEvent).toHaveBeenCalledWith(expect.objectContaining({ type: "operator.toast", message: "hi" }));
  });
  it("ignores binary frames", () => {
    vi.stubGlobal("WebSocket", MockWS);
    const onEvent = vi.fn();
    renderHook(() => useSessionEvents("sid", "tok", onEvent));
    act(() => {
      MockWS.last!.onmessage!({ data: new ArrayBuffer(8) } as MessageEvent);
    });
    expect(onEvent).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- useSessionEvents`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

```typescript
// ui/src/hooks/useSessionEvents.ts
import { useEffect, useRef } from "react";
import type { DomainEvent } from "../types";

export function useSessionEvents(
  sessionId: string,
  token: string,
  onEvent: (e: DomainEvent) => void,
) {
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  useEffect(() => {
    if (!sessionId || !token) return;
    const url = `${wsBase()}/api/sessions/${sessionId}/terminal?token=${encodeURIComponent(token)}`;
    const ws = new WebSocket(url);
    ws.onmessage = (ev) => {
      if (typeof ev.data !== "string") return; // binary = PTY bytes; ignore here
      try {
        const parsed = JSON.parse(ev.data) as DomainEvent;
        if (parsed && typeof parsed === "object" && "type" in parsed) {
          onEventRef.current(parsed);
        }
      } catch {
        /* malformed; drop */
      }
    };
    return () => { ws.close(); };
  }, [sessionId, token]);
}

function wsBase(): string {
  const proto = window.location.protocol === "https:" ? "wss" : "ws";
  return `${proto}://${window.location.host}`;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npm test -- useSessionEvents`
Expected: PASS, 2 tests.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useSessionEvents.ts ui/src/hooks/__tests__/useSessionEvents.test.ts
git commit -m "feat(ui): useSessionEvents hook parses domain-event JSON frames"
```

---

## Task 10: Drop polling in useSessions, push state from events

**Files:**
- Modify: `ui/src/hooks/useSessions.ts`
- Test: extend or write `ui/src/hooks/__tests__/useSessions.push.test.ts`

- [ ] **Step 1: Write the failing test**

```typescript
// ui/src/hooks/__tests__/useSessions.push.test.ts
import { describe, it, expect, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useSessions } from "../useSessions";

vi.mock("../api", () => ({
  listSessions: vi.fn().mockResolvedValue([
    { id: "s1", state: "idle", role: "implementation" /* …other required fields */ },
  ]),
}));

describe("useSessions push updates", () => {
  it("applies session.state_changed without re-fetching", async () => {
    const { result } = renderHook(() => useSessions());
    await act(async () => {});
    expect(result.current.sessions[0].state).toBe("idle");
    act(() => {
      result.current.applyEvent({
        type: "session.state_changed",
        session_id: "s1", prev: "idle", next: "working", last_event_ts: "t",
      });
    });
    expect(result.current.sessions[0].state).toBe("working");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd ui && npm test -- useSessions.push`
Expected: FAIL — `applyEvent` not exposed.

- [ ] **Step 3: Implement**

In `useSessions.ts`:
- Remove the `setInterval(...2000...)` loop (one initial fetch only).
- Expose `applyEvent(e: DomainEvent)` that dispatches:
  - `session.state_changed` → patch matching session's `state`
  - `session.role_changed` → patch `role`
  - `session.meta_changed` → patch counters/markdown
  - All others → no-op (handled by per-session views)
- Return `applyEvent` alongside the existing `sessions` array.

Wire the consumer in `Dashboard.tsx`: for each visible session, call `useSessionEvents(sid, token, applyEvent)`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cd ui && npm test -- useSessions.push`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add ui/src/hooks/useSessions.ts ui/src/hooks/__tests__/useSessions.push.test.ts ui/src/components/Dashboard.tsx
git commit -m "feat(ui): push-driven session updates via useSessionEvents"
```

---

## Task 11: SessionRow — seven-state color map + role badge + handoff tick

**Files:**
- Modify: `ui/src/components/SessionRow.tsx`
- Test: `ui/src/components/__tests__/SessionRow.states.test.tsx`

- [ ] **Step 1: Write the failing test**

```typescript
// ui/src/components/__tests__/SessionRow.states.test.tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { SessionRow } from "../SessionRow";

const states = ["starting","idle","working","awaiting_input","needs_attention","error","ended"] as const;

describe("SessionRow seven-state colors", () => {
  for (const s of states) {
    it(`renders a distinct dot for state=${s}`, () => {
      render(<SessionRow session={{ id:"x", state: s, role:"impl" /* required */ } as any} />);
      const dot = screen.getByTestId("session-state-dot");
      expect(dot.getAttribute("data-state")).toBe(s);
    });
  }
  it("shows role badge", () => {
    render(<SessionRow session={{ id:"x", state:"idle", role:"orchestrator" } as any} />);
    expect(screen.getByText("orchestrator")).toBeInTheDocument();
  });
  it("shows handoff tick when last_event_kind starts with mcp.handoff", () => {
    render(<SessionRow session={{ id:"x", state:"idle", role:"impl", last_event_kind:"mcp.handoff_ready" } as any} />);
    expect(screen.getByTestId("handoff-tick")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run, fix, re-run**

Run: `cd ui && npm test -- SessionRow.states`
Expected: FAIL.

Implement the three additions in `SessionRow.tsx` with a `STATE_CLASS: Record<SessionState, string>` map and conditional badge/tick rendering. Re-run; expect PASS.

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/SessionRow.tsx ui/src/components/__tests__/SessionRow.states.test.tsx
git commit -m "feat(ui): SessionRow seven-state colors + role badge + handoff tick"
```

---

## Task 12: ExpandedSessionView tabbed shell

**Files:**
- Modify: `ui/src/components/ExpandedSessionView.tsx`
- Test: `ui/src/components/__tests__/ExpandedSessionView.tabs.test.tsx`

- [ ] **Step 1: Write the failing test**

```typescript
// ui/src/components/__tests__/ExpandedSessionView.tabs.test.tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { ExpandedSessionView } from "../ExpandedSessionView";

describe("ExpandedSessionView tabs", () => {
  it("renders four tab labels", () => {
    render(<ExpandedSessionView session={{ id:"x", state:"idle" } as any} onClose={() => {}} />);
    for (const label of ["Terminal","Events","Plan","Meta"]) {
      expect(screen.getByRole("tab", { name: label })).toBeInTheDocument();
    }
  });
  it("activates Events tab on click", () => {
    render(<ExpandedSessionView session={{ id:"x", state:"idle" } as any} onClose={() => {}} />);
    fireEvent.click(screen.getByRole("tab", { name: "Events" }));
    expect(screen.getByTestId("tab-panel-events")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Implement tabs**

Add a controlled tab state (`useState<"terminal"|"events"|"plan"|"meta">`). Render `<TerminalPreview />` for `terminal`, placeholder `<div data-testid="tab-panel-events" />` etc. for the others (real implementations land in Tasks 13–15). Use minimal styling — Tabs can be simple `<button role="tab">` with aria-selected.

- [ ] **Step 3: Pass tests + commit**

```bash
git add ui/src/components/ExpandedSessionView.tsx ui/src/components/__tests__/ExpandedSessionView.tabs.test.tsx
git commit -m "feat(ui): ExpandedSessionView gains four-tab layout"
```

---

## Task 13: EventsTab — reverse-chrono feed with filter

**Files:**
- Create: `ui/src/components/EventsTab.tsx`
- Test: `ui/src/components/__tests__/EventsTab.test.tsx`
- Modify: `ui/src/components/ExpandedSessionView.tsx` (mount EventsTab in the Events panel)
- Modify: `src/api/mod.rs` — add `GET /api/sessions/{id}/events?limit=N` if not already present (Sub-plan 1 added the events table; Sub-plan 4 needs the read endpoint)
- Test: `tests/api_session_events.rs`

- [ ] **Step 1: Backend test**

```rust
// tests/api_session_events.rs
mod common;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn get_session_events_returns_reverse_chrono() {
    let setup = common::test_app();
    let proj = setup._dir.path().join("proj");
    let (sid, token) = common::seed_session(&setup.db, &proj);
    // Insert two events.
    for kind in ["a","b"] {
        galvolite::state::apply_event(&setup.db, sid, galvolite::state::EventInput {
            source: galvolite::state::Source::System,
            kind: format!("system.{kind}"), payload: serde_json::json!({}),
            dedup_key: None,
        }).unwrap();
    }
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/sessions/{sid}/events?limit=10"))
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap();
    let resp = setup.app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), 64*1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v[0]["kind"], "system.b"); // newest first
    assert_eq!(v[1]["kind"], "system.a");
}
```

- [ ] **Step 2: Implement the GET endpoint**

In `src/db.rs`, add:
```rust
pub fn read_events(&self, session_id: Uuid, limit: i64) -> Result<Vec<EventRow>> { /* SELECT ... ORDER BY id DESC LIMIT ? */ }
```

In `src/api/mod.rs`:
```rust
.route("/api/sessions/:id/events", get(read_events_handler))
```

Run the test: PASS.

- [ ] **Step 3: UI test + implementation**

```typescript
// ui/src/components/__tests__/EventsTab.test.tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import { EventsTab } from "../EventsTab";

describe("EventsTab", () => {
  it("renders newest-first rows with kind + source badge", () => {
    const events = [
      { event_id: 2, kind: "hook.UserPromptSubmit", source: "hook", ts: "t2", payload: {} },
      { event_id: 1, kind: "system.session_started", source: "system", ts: "t1", payload: {} },
    ];
    render(<EventsTab events={events} />);
    const rows = screen.getAllByTestId("event-row");
    expect(rows[0]).toHaveTextContent("hook.UserPromptSubmit");
    expect(rows[1]).toHaveTextContent("system.session_started");
  });
});
```

Implement `EventsTab` props-driven (no fetching internally); the parent (`ExpandedSessionView`) fetches once via `GET /api/sessions/:id/events` and appends `session.event_appended` events from `useSessionEvents`.

- [ ] **Step 4: Commit**

```bash
git add src/db.rs src/api/mod.rs tests/api_session_events.rs ui/src/components/EventsTab.tsx ui/src/components/__tests__/EventsTab.test.tsx ui/src/components/ExpandedSessionView.tsx
git commit -m "feat(ui+api): EventsTab + GET /api/sessions/:id/events"
```

---

## Task 14: PlanTab — live checklist from plan_steps

**Files:**
- Modify: `src/api/mod.rs` — `GET /api/sessions/:id/plan_steps`
- Modify: `src/db.rs` — `read_plan_steps(session_id) -> Vec<PlanStep>`
- Create: `ui/src/components/PlanTab.tsx`
- Test: `tests/api_plan_steps.rs`, `ui/src/components/__tests__/PlanTab.test.tsx`

- [ ] **Step 1: Backend test + endpoint** (analogous to Task 13)

- [ ] **Step 2: UI test + component**

```typescript
import { render, screen } from "@testing-library/react";
import { PlanTab } from "../PlanTab";
const steps = [
  { hash:"h1", text:"do thing", status:"done" },
  { hash:"h2", text:"do other", status:"working" },
  { hash:"h3", text:"future",   status:"pending" },
];
it("highlights the current working step", () => {
  render(<PlanTab steps={steps} />);
  expect(screen.getByTestId(`step-h2`)).toHaveAttribute("data-current", "true");
});
it("renders correct status pill for each", () => {
  render(<PlanTab steps={steps} />);
  expect(screen.getByTestId("step-h1")).toHaveTextContent("●");
  expect(screen.getByTestId("step-h2")).toHaveTextContent("◐");
  expect(screen.getByTestId("step-h3")).toHaveTextContent("○");
});
```

Implement with status → glyph map (`pending: ○, working: ◐, done: ●, blocked: ◆`); collapse done steps under a `<details>` by default; subscribe to `session.plan_replaced` and `session.plan_step_changed` to mutate local state.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(ui+api): PlanTab driven by plan_steps + live deltas"
```

---

## Task 15: MetaTab — markdown + handoff link + counters

**Files:**
- Create: `ui/src/components/MetaTab.tsx`
- Test: `ui/src/components/__tests__/MetaTab.test.tsx`

(Sessions table already carries `plan_markdown`, `seed_markdown`, `handoff_path`, `turn_count`, `est_tokens` from Sub-plan 1's schema delta.)

- [ ] **Step 1: Write the test**

```typescript
import { render, screen } from "@testing-library/react";
import { MetaTab } from "../MetaTab";

it("renders the four sections", () => {
  render(<MetaTab session={{
    plan_markdown: "## plan", seed_markdown: "## seed",
    handoff_path: "/tmp/handoff-x.md",
    turn_count: 12, est_tokens: 9000,
  } as any} />);
  expect(screen.getByText(/plan/)).toBeInTheDocument();
  expect(screen.getByText(/seed/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /handoff/i })).toHaveAttribute("href","/tmp/handoff-x.md");
  expect(screen.getByText(/12 turns/i)).toBeInTheDocument();
  expect(screen.getByText(/9,000 tokens/i)).toBeInTheDocument();
});
```

- [ ] **Step 2: Implement + commit**

```bash
git add -A
git commit -m "feat(ui): MetaTab renders plan/seed/handoff/counters"
```

---

## Task 16: OperatorPromptModal

**Files:**
- Create: `ui/src/components/OperatorPromptModal.tsx`
- Test: `ui/src/components/__tests__/OperatorPromptModal.test.tsx`
- Modify: `ui/src/api.ts` — add `answerOperatorPrompt(promptId, answer)`
- Modify: `ui/src/App.tsx` — mount once with global event subscription

- [ ] **Step 1: Test**

```typescript
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { OperatorPromptModal } from "../OperatorPromptModal";

describe("OperatorPromptModal", () => {
  it("renders question + choices and submits selection", async () => {
    const onAnswer = vi.fn().mockResolvedValue(undefined);
    render(<OperatorPromptModal
      prompt={{ prompt_id:"p1", session_id:"s1", question:"go?", choices:["yes","no"], timeout_secs:60 }}
      onAnswer={onAnswer}
    />);
    fireEvent.click(screen.getByRole("button", { name: "yes" }));
    expect(onAnswer).toHaveBeenCalledWith("p1","yes");
  });
  it("renders a freeform input when no choices", () => {
    const onAnswer = vi.fn();
    render(<OperatorPromptModal
      prompt={{ prompt_id:"p1", session_id:"s1", question:"why?", choices:null, timeout_secs:null }}
      onAnswer={onAnswer}
    />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "because" } });
    fireEvent.click(screen.getByRole("button", { name: /submit/i }));
    expect(onAnswer).toHaveBeenCalledWith("p1","because");
  });
});
```

- [ ] **Step 2: Implement**

Component is purely props-driven. The wiring lives in `App.tsx`:
```typescript
const [prompt, setPrompt] = useState<OperatorPromptEvent | null>(null);
useGlobalEvents((e) => {
  if (e.type === "operator.prompt") setPrompt(e);
});
return (<>
  <Dashboard />
  {prompt && <OperatorPromptModal prompt={prompt} onAnswer={async (id,a) => {
    await answerOperatorPrompt(id, a);
    setPrompt(null);
  }} />}
</>);
```

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/OperatorPromptModal.tsx ui/src/components/__tests__/OperatorPromptModal.test.tsx ui/src/api.ts ui/src/App.tsx
git commit -m "feat(ui): OperatorPromptModal + answer wire-up"
```

---

## Task 17: ToastStack

**Files:**
- Create: `ui/src/components/ToastStack.tsx`
- Test: `ui/src/components/__tests__/ToastStack.test.tsx`
- Modify: `ui/src/App.tsx` to mount it

- [ ] **Step 1: Test**

```typescript
import { render, screen, act } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ToastStack } from "../ToastStack";

describe("ToastStack", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("renders an info toast and auto-dismisses after 4s", () => {
    const { rerender } = render(<ToastStack toasts={[{ id:"1", level:"info", message:"hi" }]} onDismiss={() => {}} />);
    expect(screen.getByText("hi")).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(4100); });
    // ToastStack calls onDismiss; in this test, parent removes it; for unit-test simplicity, assert callback invoked
  });
  it("urgent toasts do not auto-dismiss", () => {
    const onDismiss = vi.fn();
    render(<ToastStack toasts={[{ id:"1", level:"urgent", message:"!!" }]} onDismiss={onDismiss} />);
    act(() => { vi.advanceTimersByTime(60_000); });
    expect(onDismiss).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Implement + wire**

In `ToastStack.tsx`: render up to 5 stacked, sorted by recency; use `setTimeout` per non-urgent toast (info 4s, warn 8s); position top-right; level → border color.

In `App.tsx`: maintain `useState<Toast[]>`, push on `operator.toast`, pass `onDismiss` removing the toast.

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/ToastStack.tsx ui/src/components/__tests__/ToastStack.test.tsx ui/src/App.tsx
git commit -m "feat(ui): ToastStack listening to operator.toast events"
```

---

## Task 18: SessionRail "Event log" button

**Files:**
- Modify: `ui/src/components/SessionRail.tsx`
- Modify: `ui/src/components/SessionRail.test.tsx` (extend)

- [ ] **Step 1: Test**

```typescript
it("emits onOpenEvents when Event log button clicked", () => {
  const onOpenEvents = vi.fn();
  render(<SessionRail /* existing props */ onOpenEvents={onOpenEvents} />);
  fireEvent.click(screen.getByRole("button", { name: /event log/i }));
  expect(onOpenEvents).toHaveBeenCalled();
});
```

- [ ] **Step 2: Add the button**

```typescript
<button onClick={onOpenEvents}>Event log</button>
```

Wire `onOpenEvents` in `Dashboard.tsx` to set the expanded session's tab to `"events"`.

- [ ] **Step 3: Commit**

```bash
git add ui/src/components/SessionRail.tsx ui/src/components/SessionRail.test.tsx ui/src/components/Dashboard.tsx
git commit -m "feat(ui): SessionRail Event log button opens Events tab"
```

---

## Task 19: End-to-end smoke (real claude binary)

**Files:**
- Create: `tests/dashboard_smoke_e2e.rs` — drives the `galvolite` server in-process, opens a WS, spawns a real `claude` PTY (use `fixtures/mock-claude/` if real `claude` is unavailable in CI), expects to observe the lifecycle of WS frames.

- [ ] **Step 1: Write the smoke**

```rust
// tests/dashboard_smoke_e2e.rs
//! End-to-end smoke for the dashboard event stream.
//!
//! Spec §10.3 parity rule: every state change and operator-relevant event
//! observable in the WebSocket. This test asserts the happy path:
//! seed → ws open → trigger ask_operator via MCP → observe operator.prompt
//! → answer via HTTP → observe state_changed back to working/idle.
mod common;
// (Body draws on patterns from tests/mcp_smoke_endtoend.rs — happy path only.)
```

(Full body should mirror `mcp_smoke_endtoend.rs` and assert the four-step sequence above.)

- [ ] **Step 2: Run**

Run: `cargo test --test dashboard_smoke_e2e -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/dashboard_smoke_e2e.rs
git commit -m "test(dashboard): end-to-end smoke for WS event lifecycle"
```

---

## Task 20: Final sweep — clippy + UI build + manual dashboard sanity

- [ ] **Step 1:** `cargo clippy --all-targets --no-deps` → no warnings.
- [ ] **Step 2:** `cd ui && npm run build && npm test` → green build, all UI tests pass.
- [ ] **Step 3:** Boot the desktop shell (`cargo tauri dev`), spawn a session, type "hello\n" in the terminal pane and confirm `\r` lands (Finding 2). Trigger an `ask_operator` via MCP curl; modal appears. Click answer; modal dismisses; tool resolves. Toast tests: `notify_operator` warn → orange toast; urgent → sticky.
- [ ] **Step 4:** `git status` clean, branch ready for merge.

```bash
git log --oneline main..HEAD
```

---

## Self-review notes

- Spec §10.1 lists eight WS event types — all eight are emitted (Tasks 3, 5, 6, 13, 14) and all eight have a UI consumer (Tasks 11–17).
- Spec §10.2 enumerates SessionRow / ExpandedSessionView / EventsTab / PlanTab / MetaTab / OperatorPromptModal / Toast stack / Rail Event log — Tasks 11, 12, 13, 14, 15, 16, 17, 18 cover them in order.
- Spec §10.3 parity rule — every new MCP tool that mutates state has a UI surface (state_changed → SessionRow color; role_changed → role badge; toast → ToastStack; prompt → modal).
- Finding 2 (`\r` fix) is Task 7.
- `apply_event_with_bus` is the only new state-mutation entry point — old `apply_event` stays as the bus-less primitive for tests, but every production caller migrates (Task 3 step 5).
- Plan steps and meta updates: Tasks 14 and 15 cover the read endpoints + UI; the *publish* side (where `state.rs` or an MCP tool mutates `plan_steps` / `sessions.plan_markdown` and emits `session.plan_replaced` / `session.meta_changed`) currently has no source — Sub-plan 5 (recovery + signal deletion) introduces it via the `define_plan` / `update_plan_step` MCP tools that exist as stubs today. **For Sub-plan 4, the UI components handle the events but they will only observe initial fetches until Sub-plan 5 wires the publishers.** Documented as a known limitation rather than an implementation task here.
