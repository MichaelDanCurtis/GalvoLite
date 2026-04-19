# Hook Surface (Sub-plan 2/5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire Claude Code's seven hooks (`SessionStart`, `SessionEnd`, `PreCompact`, `Stop`, `UserPromptSubmit`, `PostToolUse(Bash)`, `Notification`) into GalvoLite's `state::apply_event` funnel via a tiny `galvolite-hook` binary that POSTs to a new `/hooks/ingest` endpoint. Every hook fire writes one event-log row, updates session state atomically, and may return `inject_text` to the model. The first three tasks finish the §8.3 atomicity invariant by closing three follow-ups left from Sub-plan 1.

**Architecture:** Each session's worktree gets a `.claude/settings.json` (written by `SessionManager::spawn`) whose seven hook entries all dispatch to a single `galvolite-hook` binary. The binary reads the hook payload from stdin, looks up `GALVOLITE_SESSION_ID` / `GALVOLITE_API_BASE` / `GALVOLITE_TOKEN` from its env, POSTs to `/hooks/ingest` with a 5 s timeout, and writes any `inject_text` from the response to stdout (which CC injects into the model's context). The endpoint authenticates the bearer token in constant time, resolves it to `session_id`, calls `apply_event(input)` (one transaction = one event row + one sessions update), then computes the response from post-write state. No code path mutates session state outside the funnel. The legacy `signals.rs` parser remains untouched (it dual-writes through the funnel as of Sub-plan 1) — Sub-plan 5 deletes it.

**Tech Stack:** Rust 1.75+ (existing), `axum` 0.7 (existing), `rusqlite` 0.31 (existing), `reqwest` 0.12 with `blocking` + `json` + `rustls-tls` features (NEW for the hook binary), `subtle` 2 for constant-time token compare (NEW), `tracing` (existing) for structured logs. No workspace split — `galvolite-hook` is `src/bin/galvolite-hook.rs` in the main crate.

---

## File Structure

**New files:**
- `src/error.rs` — adds `Error::TerminalState`, `Error::EventPayload` variants (Task 1; the file may not exist yet — if errors live in `lib.rs`, modify there).
- `src/spawn_config.rs` — `.claude/settings.json` writer (Task 6)
- `src/hooks/mod.rs` — module root, re-exports (Task 4)
- `src/hooks/types.rs` — `HookKind` enum + `HookIngestRequest` / `HookIngestResponse` envelope types shared by binary and server (Task 4)
- `src/hooks/client.rs` — binary's pure logic: read stdin payload, POST, write response (Task 4)
- `src/bin/galvolite-hook.rs` — binary entry: parses env, reads stdin, calls `client::run()`, writes stdout, exit 0 (Task 4)
- `src/api/hooks.rs` — `/hooks/ingest` axum handler + bearer auth middleware (Task 5)
- `src/api/hook_handlers/mod.rs` — module root for per-hook semantics (Task 7)
- `src/api/hook_handlers/lifecycle.rs` — `SessionStart`, `SessionEnd` handlers (Task 7)
- `src/api/hook_handlers/turn.rs` — `UserPromptSubmit`, `Stop`, `PreCompact` handlers (Task 8)
- `src/api/hook_handlers/observe.rs` — `PostToolUse(Bash)`, `Notification` handlers (Task 9)
- `tests/error_typed_variants.rs` — Task 1: round-trips `Error::TerminalState` / `Error::EventPayload`
- `tests/state_session_orphaned.rs` — Task 2: spawn-failure rollback paths emit `system.session_orphaned` events
- `tests/state_starting_to_idle.rs` — Task 3: `Starting → Idle` on `system.session_started` (deviation reverted once UserPromptSubmit lands)
- `tests/hooks_binary_client.rs` — Task 4: unit tests for `hooks::client::run()`
- `tests/hooks_binary_exec.rs` — Task 4: binary integration via `CARGO_BIN_EXE_galvolite-hook`
- `tests/api_hooks_ingest.rs` — Task 5: `/hooks/ingest` contract (auth, validation, event row written)
- `tests/spawn_config_writer.rs` — Task 6: `.claude/settings.json` writer
- `tests/hooks_lifecycle.rs` — Task 7: `SessionStart` / `SessionEnd` semantics
- `tests/hooks_turn.rs` — Task 8: `UserPromptSubmit`, `Stop`, `PreCompact` semantics
- `tests/hooks_observe.rs` — Task 9: `PostToolUse`, `Notification` semantics
- `tests/hooks_smoke_endtoend.rs` — Task 10: spawns a session, fires all 7 hooks via the real binary against the running app, verifies state + event log

**Modified files:**
- `src/error.rs` (or wherever `Error` lives) — add two variants (Task 1)
- `src/state.rs` — add `system.session_orphaned` arm to `transition()` (Task 2); change `Starting → Working` arm back to `Starting → Idle` (Task 3)
- `src/state.rs` — `apply_event` returns `Error::TerminalState` instead of `Error::Config(...)` (Task 1)
- `src/session_manager.rs` — replace 7 `db.end_session_if_unended(.., Error)` direct calls with `apply_event(system.session_orphaned, …)` (Task 2); call `spawn_config::write_session_files` in spawn flow (Task 6)
- `src/api/mod.rs` (or `src/router.rs`) — mount `/hooks/ingest` route (Task 5)
- `src/lib.rs` — `pub mod hooks; pub mod spawn_config; pub mod api::hooks; pub mod api::hook_handlers;`
- `Cargo.toml` — add `[[bin]] name = "galvolite-hook"`; add `reqwest` (blocking, no default features, `json` + `rustls-tls`); add `subtle = "2"`
- `tests/state_apply_event.rs` — assert specific `Error::TerminalState` variant (Task 1; was substring match)

---

## Spec ↔ task crosswalk

| Spec section | Sub-plan 2 task |
|--------------|-----------------|
| §6.1 (envelope: POST /hooks/ingest, X-Galvolite-Token header) | Task 5 |
| §6.2 SessionStart                                           | Task 7 |
| §6.2 SessionEnd                                             | Task 7 |
| §6.2 PreCompact (with §15 dep #3 caveat)                    | Task 8 |
| §6.2 Stop (max 3 retries via event-log query)               | Task 8 |
| §6.2 UserPromptSubmit (turn count, est_tokens, 80 % warn)   | Task 8 |
| §6.2 PostToolUse(Bash)                                      | Task 9 |
| §6.2 Notification                                           | Task 9 |
| §6.3 (atomicity: response from post-write state)            | Tasks 5 + 7 + 8 + 9 |
| §7.1 step 4 (`.claude/settings.json` writer)                | Task 6 |
| §7.2 (bearer auth, constant-time compare, 401 on bad token) | Task 5 |
| §8.3 (atomicity invariant — final hardening of Sub-plan 1's gap) | Task 2 |
| §8.6 (`Starting → Idle` deviation normalization)            | Task 3 |
| §15 dep #3 (PreCompact `inject_text` unverified)            | Task 8 — `tracing::warn!` marker |

Sub-plan 5 owns: real-claude smoke, restart replay using events, deletion of `signals.rs`. Sub-plan 4 owns: WS event fan-out, dashboard tabs, operator UI for `notify_operator` / `ask_operator`. Sub-plan 3 owns: `/mcp` endpoint and `.mcp.json` writer.

---

## Branch / worktree

This plan should be executed on a dedicated branch:
```bash
git switch -c plan-hook-surface
```
Or in a worktree:
```bash
git worktree add ../galvolite-hooks -b plan-hook-surface main
```
All commits land on this branch. Merge to `main` after Sub-plan 2 passes its goal-backward review.

---

## Task 1: Split `Error::Config` into `TerminalState` + `EventPayload`

**Goal:** `apply_event` currently signals "rejected because session is in terminal state" by stuffing the word "terminal" into `Error::Config`. Sub-plan 2's HTTP layer will need to map that to 409, and `Error::EventPayload` (serde JSON failures) to 400 — substring matching is fragile. Replace with typed variants.

**Files:**
- Modify: `src/error.rs` (or `src/lib.rs` if errors are defined there) — add two enum variants
- Modify: `src/state.rs` — `apply_event` returns the typed errors
- Modify: `tests/state_apply_event.rs` — assert variants instead of substring
- Create: `tests/error_typed_variants.rs` — explicit round-trip test

- [ ] **Step 1: Write failing test for typed variants**

Create `tests/error_typed_variants.rs`:

```rust
use galvolite::error::Error;

#[test]
fn terminal_state_variant_displays_with_session_and_kind() {
    let e = Error::TerminalState {
        session_id: "abc-123".into(),
        kind: "signal.STATUS:WORKING".into(),
    };
    let msg = e.to_string();
    assert!(msg.contains("abc-123"), "session id missing in: {msg}");
    assert!(msg.contains("STATUS:WORKING"), "kind missing in: {msg}");
    assert!(msg.contains("terminal"), "should mention terminal state: {msg}");
}

#[test]
fn event_payload_variant_wraps_serde_error() {
    let inner: serde_json::Error = serde_json::from_str::<serde_json::Value>("not-json").unwrap_err();
    let e = Error::EventPayload(inner);
    let msg = e.to_string();
    assert!(msg.to_lowercase().contains("payload") || msg.to_lowercase().contains("json"),
        "should mention payload/json: {msg}");
}
```

Find where `Error` is defined (likely `src/error.rs` or `src/lib.rs`):
```bash
grep -n "enum Error" src/*.rs | head -5
```

- [ ] **Step 2: Add the two variants**

Edit the `Error` enum to add (positions and `#[error(...)]` attributes follow the existing pattern):

```rust
/// A transition was attempted on a session whose state is terminal (Error or Ended).
#[error("session {session_id} is in terminal state — cannot apply {kind}")]
TerminalState {
    session_id: String,
    kind: String,
},

/// Failed to serialize/deserialize an event payload to JSON.
#[error("event payload JSON error: {0}")]
EventPayload(#[from] serde_json::Error),
```

If `Error` does not currently `#[derive(thiserror::Error)]`, follow whatever style is already in the file — the test above only relies on `Display`. Do not change the rest of the variants.

- [ ] **Step 3: Run failing test (RED)**

```bash
cargo test --test error_typed_variants
```

Expect compile errors then test failures, then pass after Step 2.

- [ ] **Step 4: Update `apply_event` to use typed errors**

In `src/state.rs::apply_event`, find the terminal-state guard (currently emits `Error::Config(format!("...terminal..."))`). Replace with:

```rust
if current.is_terminal() {
    return Err(Error::TerminalState {
        session_id: session_id.to_string(),
        kind: input.kind.clone(),
    });
}
```

If there's a JSON serialization site that returns `Error::Config` for serde failures, change the `?` propagation so it auto-converts via `#[from] serde_json::Error` → `Error::EventPayload`.

- [ ] **Step 5: Update `tests/state_apply_event.rs::terminal_state_rejects_further_transitions`**

Replace the substring match with a variant match:

```rust
let err = state::apply_event(&db, sid, EventInput { ... }).unwrap_err();
match err {
    Error::TerminalState { session_id, kind } => {
        assert_eq!(session_id, sid.to_string());
        assert_eq!(kind, "signal.STATUS:WORKING");
    }
    other => panic!("expected TerminalState, got {other:?}"),
}
```

- [ ] **Step 6: All tests pass (GREEN)**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/error.rs src/state.rs tests/state_apply_event.rs tests/error_typed_variants.rs
git commit -m "$(cat <<'EOF'
refactor(error): split Error::Config into typed TerminalState + EventPayload

apply_event was returning Error::Config with the substring "terminal" when a
transition was attempted on a terminal-state session, and the same variant when
serde JSON failed inside the funnel. Sub-plan 2's HTTP layer needs to map these
to 409 and 400 respectively; substring matching is fragile.

Adds Error::TerminalState { session_id, kind } and Error::EventPayload(serde::Error,
via #[from]). Updates state::apply_event to return them directly.
EOF
)"
```

---

## Task 2: Migrate spawn-failure rollback paths through `apply_event`

**Goal:** `SessionManager::spawn_session` has 7 fallible setup steps (worktree creation, role lookup, env construction, etc.). Each one currently calls `db.end_session_if_unended(.., SessionState::Error)` directly when it fails — bypassing the funnel and skipping the event-log write. Add a new `system.session_orphaned` event kind to §8.6's transition table, then route every rollback through `apply_event`.

**Files:**
- Modify: `src/state.rs::transition` — add `(_, "system.session_orphaned") => Error` arm covering any non-terminal source state
- Modify: `src/session_manager.rs` — replace 7 `db.end_session_if_unended(...)` direct calls with `state::apply_event(db, id, EventInput { source: System, kind: "system.session_orphaned", payload: json!({"reason": "..."}), dedup_key: Some(format!("session-orphaned:{id}")) })`
- Create: `tests/state_session_orphaned.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/state_session_orphaned.rs
use galvolite::{db::Db, state::{self, EventInput, Source, SessionState}};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn session_orphaned_transitions_starting_to_error_and_writes_event() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let id = db.insert_session("test", "implementation", "/tmp", None).unwrap();

    let outcome = state::apply_event(&db, id, EventInput {
        source: Source::System,
        kind: "system.session_orphaned".into(),
        payload: json!({"reason": "worktree_create_failed"}),
        dedup_key: Some(format!("session-orphaned:{id}")),
    }).unwrap();

    assert_eq!(outcome.next_state, SessionState::Error);

    let conn = db.pool().get().unwrap();
    let event_count: i64 = conn.query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'system.session_orphaned'",
        [id.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(event_count, 1);
}

#[test]
fn session_orphaned_works_from_idle_too() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let id = db.insert_session("test", "implementation", "/tmp", None).unwrap();

    // Move to Idle first via system.session_started
    state::apply_event(&db, id, EventInput {
        source: Source::System,
        kind: "system.session_started".into(),
        payload: json!({}),
        dedup_key: Some(format!("session-started:{id}")),
    }).unwrap();

    let outcome = state::apply_event(&db, id, EventInput {
        source: Source::System,
        kind: "system.session_orphaned".into(),
        payload: json!({"reason": "post_spawn_failure"}),
        dedup_key: None,
    }).unwrap();

    assert_eq!(outcome.next_state, SessionState::Error);
}
```

```bash
cargo test --test state_session_orphaned
```

Expect failure (no `system.session_orphaned` arm yet).

- [ ] **Step 2: Add transition arm**

In `src/state.rs::transition`:

```rust
// Anywhere a non-terminal session can be force-failed by the orchestrator
// or supervisor (spawn rollback, watchdog).
(s, "system.session_orphaned") if !s.is_terminal() => SessionState::Error,
```

Re-run test → GREEN.

- [ ] **Step 3: Find and replace the 7 rollback sites**

```bash
grep -n "end_session_if_unended" src/session_manager.rs
```

For each call site (will be ~7), replace:

```rust
let _ = self.db.end_session_if_unended(id, SessionState::Error);
return Err(/* original error */);
```

with:

```rust
let _ = state::apply_event(&self.db, id, state::EventInput {
    source: state::Source::System,
    kind: "system.session_orphaned".into(),
    payload: serde_json::json!({"reason": "<short_string_per_site>"}),
    dedup_key: Some(format!("session-orphaned:{id}")),
});
return Err(/* original error */);
```

Use a distinct `reason` per site so post-mortems can tell which step failed: `worktree_create_failed`, `role_lookup_failed`, `env_construction_failed`, `pty_spawn_failed`, etc. Pick names that match the actual failure each site is rolling back.

- [ ] **Step 4: Update existing session_manager tests if any check end_session_if_unended specifically**

```bash
cargo test --test session_manager
```

If a test asserts the legacy `end_session_if_unended` path, update it to assert the event row landed instead. The visible behavior is identical (session ends in `Error`); the only difference is that an `events` row now exists.

- [ ] **Step 5: All tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/state.rs src/session_manager.rs tests/state_session_orphaned.rs
git commit -m "$(cat <<'EOF'
fix(session_manager): route spawn-failure rollbacks through apply_event funnel

Closes the §8.3 atomicity gap left from Sub-plan 1: 7 spawn-failure paths in
SessionManager::spawn_session were calling db.end_session_if_unended(.., Error)
directly, skipping the event-log write and producing sessions whose terminal
state had no audit trail.

Adds a new "system.session_orphaned" event kind (any non-terminal state → Error)
to state::transition. Each rollback site now calls apply_event with a distinct
reason string so post-mortems can identify which step failed.
EOF
)"
```

---

## Task 3: Normalize `Starting → Idle` per spec §8.6

> **Position note:** This task should be executed *after* Task 8 (UserPromptSubmit hook) lands. The deviation in Sub-plan 1 was: spawn injects the prompt, so the session goes straight to Working. Once UserPromptSubmit fires from the hook, that hook drives the `Idle → Working` transition naturally and `system.session_started` should leave the session in `Idle`. Listed here for plan coherence; gate execution on Task 8 being committed first.

**Goal:** Restore the spec §8.6 behavior: `system.session_started → Idle`. The Idle → Working transition will then come from `UserPromptSubmit` (added in Task 8).

**Files:**
- Modify: `src/state.rs::transition` — change the `(Starting, "system.session_started")` arm
- Create: `tests/state_starting_to_idle.rs`
- Modify: `tests/session_manager_apply_event.rs` if it asserts post-spawn state is Working

- [ ] **Step 1: Write failing test**

```rust
// tests/state_starting_to_idle.rs
use galvolite::{db::Db, state::{self, EventInput, Source, SessionState}};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn starting_to_idle_on_session_started_per_spec_8_6() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("test.db")).unwrap();
    let id = db.insert_session("test", "implementation", "/tmp", None).unwrap();

    let outcome = state::apply_event(&db, id, EventInput {
        source: Source::System,
        kind: "system.session_started".into(),
        payload: json!({}),
        dedup_key: Some(format!("session-started:{id}")),
    }).unwrap();

    assert_eq!(outcome.next_state, SessionState::Idle,
        "spec §8.6: system.session_started leaves session Idle, then UserPromptSubmit moves it to Working");
}
```

- [ ] **Step 2: Update transition arm**

In `src/state.rs::transition`:

```rust
// BEFORE: (Starting, "system.session_started" | "hook.SessionStart") => Working,
// AFTER:
(Starting, "system.session_started" | "hook.SessionStart") => Idle,
```

- [ ] **Step 3: Update any test that asserted post-spawn state is Working**

```bash
grep -rn "SessionState::Working" tests/
```

If `tests/session_manager_apply_event.rs::spawn_then_kill_writes_lifecycle_events` asserts intermediate state was Working, change to Idle (and verify the kill still moves it to Ended).

- [ ] **Step 4: Tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add src/state.rs tests/state_starting_to_idle.rs tests/session_manager_apply_event.rs
git commit -m "$(cat <<'EOF'
fix(state): restore spec §8.6 — system.session_started → Idle, not Working

Sub-plan 1 deviated to Starting → Working because spawn immediately injected the
seed prompt (no UserPromptSubmit hook on the wire). With Task 8 landing the
UserPromptSubmit handler, that hook now drives Idle → Working naturally.

Resync transition table to spec.
EOF
)"
```

---

## Task 4: `galvolite-hook` binary skeleton

**Goal:** Standalone binary at `src/bin/galvolite-hook.rs` that reads a hook payload from stdin, POSTs to `/hooks/ingest`, and writes any `inject_text` from the response to stdout. Bounded 5 s timeout, fail-soft (exit 0) on any error, errors logged to stderr. Path to the binary is overridable via `GALVOLITE_HOOK_BIN` env var (tests inject this).

**Files:**
- Create: `src/hooks/mod.rs` (`pub mod client; pub mod types;`)
- Create: `src/hooks/types.rs` — `HookKind` enum + `HookIngestRequest` / `HookIngestResponse` shapes
- Create: `src/hooks/client.rs` — pure function `run(env: ClientEnv, stdin_payload: serde_json::Value) -> ClientOutcome`
- Create: `src/bin/galvolite-hook.rs` — env reading + stdin/stdout glue, 10–20 LOC
- Modify: `src/lib.rs` — `pub mod hooks;`
- Modify: `Cargo.toml` — add `[[bin]]`, add `reqwest` dep
- Create: `tests/hooks_binary_client.rs`
- Create: `tests/hooks_binary_exec.rs`

- [ ] **Step 1: Add Cargo deps and bin section**

```toml
# Cargo.toml additions
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }

[[bin]]
name = "galvolite-hook"
path = "src/bin/galvolite-hook.rs"
```

(The existing `galvolite` server bin stays unchanged; just append the new `[[bin]]` section.)

- [ ] **Step 2: Define shared types**

```rust
// src/hooks/types.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookKind {
    SessionStart,
    SessionEnd,
    PreCompact,
    Stop,
    UserPromptSubmit,
    PostToolUse,
    Notification,
}

impl HookKind {
    pub fn as_event_kind(&self) -> String {
        format!("hook.{self:?}")
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HookIngestRequest {
    pub session_id: String,
    pub hook: HookKind,
    pub ts: String,
    pub event_id: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HookIngestResponse {
    pub action: HookAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inject_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HookAction {
    Proceed,
    Continue,
    Block,
}
```

- [ ] **Step 3: Write failing test for `client::run()`**

```rust
// tests/hooks_binary_client.rs
use galvolite::hooks::{client, types::*};
use serde_json::json;

#[test]
fn run_posts_envelope_and_returns_response_inject_text() {
    let mock = mockito::Server::new();
    let url = mock.url();

    let m = mock.mock("POST", "/hooks/ingest")
        .match_header("X-Galvolite-Token", "tok123")
        .match_header("content-type", "application/json")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(serde_json::to_string(&HookIngestResponse {
            action: HookAction::Proceed,
            inject_text: Some("hello model".into()),
            note: None,
        }).unwrap())
        .create();

    let env = client::ClientEnv {
        api_base: url,
        session_id: "s-1".into(),
        token: "tok123".into(),
        timeout_secs: 5,
    };

    let outcome = client::run(env, HookKind::SessionStart, json!({"is_resume": false})).unwrap();
    assert_eq!(outcome.inject_text.as_deref(), Some("hello model"));
    m.assert();
}

#[test]
fn run_returns_no_inject_text_on_500() {
    let mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock.mock("POST", "/hooks/ingest").with_status(500).create();

    let env = client::ClientEnv {
        api_base: url,
        session_id: "s-1".into(),
        token: "tok".into(),
        timeout_secs: 5,
    };
    let outcome = client::run(env, HookKind::Stop, json!({})).unwrap_or_default();
    assert!(outcome.inject_text.is_none());
}

#[test]
fn run_times_out_within_bound() {
    use std::time::Instant;
    let mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock.mock("POST", "/hooks/ingest")
        .with_chunked_body(|w| { std::thread::sleep(std::time::Duration::from_secs(10)); w.write_all(b"") })
        .create();

    let env = client::ClientEnv {
        api_base: url,
        session_id: "s-1".into(),
        token: "tok".into(),
        timeout_secs: 1,
    };
    let start = Instant::now();
    let _ = client::run(env, HookKind::Stop, serde_json::json!({}));
    assert!(start.elapsed().as_secs() < 3, "should bail within timeout");
}
```

Add `mockito = "1"` to `[dev-dependencies]` in Cargo.toml.

- [ ] **Step 4: Implement `client::run()`**

```rust
// src/hooks/client.rs
use crate::hooks::types::*;
use std::time::Duration;
use uuid::Uuid;

pub struct ClientEnv {
    pub api_base: String,
    pub session_id: String,
    pub token: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Default)]
pub struct ClientOutcome {
    pub inject_text: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

pub fn run(env: ClientEnv, hook: HookKind, payload: serde_json::Value)
    -> Result<ClientOutcome, ClientError>
{
    let req = HookIngestRequest {
        session_id: env.session_id,
        hook,
        ts: chrono::Utc::now().to_rfc3339(),
        event_id: Uuid::new_v4().to_string(),
        payload,
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(env.timeout_secs))
        .build()?;

    let resp = client
        .post(format!("{}/hooks/ingest", env.api_base))
        .header("X-Galvolite-Token", &env.token)
        .json(&req)
        .send()?;

    if !resp.status().is_success() {
        return Ok(ClientOutcome::default());
    }

    let parsed: HookIngestResponse = resp.json()?;
    Ok(ClientOutcome { inject_text: parsed.inject_text })
}
```

- [ ] **Step 5: Implement the binary entry**

```rust
// src/bin/galvolite-hook.rs
use galvolite::hooks::{client, types::HookKind};
use std::io::{self, Read, Write};

fn main() {
    if let Err(e) = run() {
        eprintln!("[galvolite-hook] error: {e}");
    }
    // Always exit 0: hooks are fail-soft so claude never sees our errors.
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let env = client::ClientEnv {
        api_base: std::env::var("GALVOLITE_API_BASE")?,
        session_id: std::env::var("GALVOLITE_SESSION_ID")?,
        token: std::env::var("GALVOLITE_TOKEN")?,
        timeout_secs: 5,
    };

    let hook_str = std::env::args().nth(1).ok_or("missing hook arg")?;
    let hook: HookKind = serde_json::from_value(serde_json::Value::String(hook_str))?;

    let mut payload_buf = String::new();
    io::stdin().read_to_string(&mut payload_buf)?;
    let payload: serde_json::Value = if payload_buf.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&payload_buf)?
    };

    let outcome = client::run(env, hook, payload)?;
    if let Some(text) = outcome.inject_text {
        io::stdout().write_all(text.as_bytes())?;
    }
    Ok(())
}
```

- [ ] **Step 6: Write failing binary integration test**

```rust
// tests/hooks_binary_exec.rs
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn binary_exec_against_real_server_writes_inject_text_to_stdout() {
    let mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock.mock("POST", "/hooks/ingest")
        .with_status(200)
        .with_body(r#"{"action":"proceed","inject_text":"injected!"}"#)
        .create();

    let bin = env!("CARGO_BIN_EXE_galvolite-hook");
    let mut child = Command::new(bin)
        .arg("SessionStart")
        .env("GALVOLITE_API_BASE", &url)
        .env("GALVOLITE_SESSION_ID", "s-1")
        .env("GALVOLITE_TOKEN", "tok")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"{}").unwrap();
    let out = child.wait_with_output().unwrap();

    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "injected!");
}

#[test]
fn binary_exec_fails_soft_when_server_unreachable() {
    let bin = env!("CARGO_BIN_EXE_galvolite-hook");
    let mut child = Command::new(bin)
        .arg("Stop")
        .env("GALVOLITE_API_BASE", "http://127.0.0.1:1") // refused
        .env("GALVOLITE_SESSION_ID", "s-1")
        .env("GALVOLITE_TOKEN", "tok")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(b"{}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "should exit 0 even on refused connection");
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("error"), "should log the error to stderr: {stderr}");
}

#[test]
fn binary_exec_with_block_action_writes_no_stdout() {
    let mock = mockito::Server::new();
    let url = mock.url();
    let _m = mock.mock("POST", "/hooks/ingest")
        .with_status(200)
        .with_body(r#"{"action":"block"}"#)
        .create();

    let bin = env!("CARGO_BIN_EXE_galvolite-hook");
    let mut child = Command::new(bin)
        .arg("PreCompact")
        .env("GALVOLITE_API_BASE", &url)
        .env("GALVOLITE_SESSION_ID", "s-1")
        .env("GALVOLITE_TOKEN", "tok")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(b"{}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.stdout.is_empty());
}
```

- [ ] **Step 7: All tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/hooks/ src/bin/galvolite-hook.rs src/lib.rs tests/hooks_binary_*.rs
git commit -m "$(cat <<'EOF'
feat(hooks): add galvolite-hook binary that POSTs hook events to /hooks/ingest

New crate binary src/bin/galvolite-hook.rs reads hook payload from stdin, POSTs
to /hooks/ingest with the session's bearer token, and writes any inject_text
from the response to stdout. Fail-soft: any error logs to stderr but the binary
exits 0 so claude never sees our infra failures. 5s timeout per spec §6.

Endpoint not yet implemented (Task 5). Includes unit tests via mockito and
binary integration tests via CARGO_BIN_EXE_galvolite-hook.
EOF
)"
```

---

## Task 5: `/hooks/ingest` endpoint + bearer auth middleware

**Goal:** Add the `/hooks/ingest` POST endpoint that:
1. Validates the `X-Galvolite-Token` header against `sessions.api_token` in constant time → resolves `session_id`. 401 on miss.
2. Validates the request body matches `HookIngestRequest`. 400 on shape error.
3. Verifies the body's `session_id` matches the token-resolved one. 401 on mismatch.
4. Calls `state::apply_event` with `kind = format!("hook.{:?}", hook)` and the request body's `event_id` as `dedup_key`. Returns 409 on terminal-state errors (Task 1's `Error::TerminalState`).
5. Returns `HookIngestResponse { action: Proceed, inject_text: None, note: None }` for now (per-hook semantics added in Tasks 7–9).

**Files:**
- Create: `src/api/hooks.rs`
- Modify: `src/api/mod.rs` (or `src/router.rs` — wherever routes are mounted) — add `.route("/hooks/ingest", post(hooks::ingest))`
- Modify: `Cargo.toml` — add `subtle = "2"`
- Create: `tests/api_hooks_ingest.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/api_hooks_ingest.rs
use galvolite::{db::Db, hooks::types::*};
use serde_json::json;
// ... use whatever existing test harness pattern Sub-plan 1 used (e.g. spawn an
// ephemeral axum::Server, return reqwest::Client + Db).

#[tokio::test]
async fn rejects_missing_token() {
    let app = test_app().await;
    let res = app.client.post(format!("{}/hooks/ingest", app.url))
        .json(&minimal_envelope("00000000-0000-0000-0000-000000000000"))
        .send().await.unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn rejects_bad_token() {
    let app = test_app().await;
    let res = app.client.post(format!("{}/hooks/ingest", app.url))
        .header("X-Galvolite-Token", "wrong")
        .json(&minimal_envelope("00000000-0000-0000-0000-000000000000"))
        .send().await.unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn rejects_unknown_session() {
    let app = test_app().await;
    let res = app.client.post(format!("{}/hooks/ingest", app.url))
        .header("X-Galvolite-Token", "any") // even valid format → 401 since no session has it
        .json(&minimal_envelope("11111111-1111-1111-1111-111111111111"))
        .send().await.unwrap();
    assert_eq!(res.status(), 401);
}

#[tokio::test]
async fn rejects_malformed_envelope() {
    let app = test_app().await;
    let (id, token) = seed_session(&app.db).await;
    let res = app.client.post(format!("{}/hooks/ingest", app.url))
        .header("X-Galvolite-Token", &token)
        .body("{not json")
        .send().await.unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn accepts_valid_session_writes_event_returns_proceed() {
    let app = test_app().await;
    let (id, token) = seed_session(&app.db).await;
    let res = app.client.post(format!("{}/hooks/ingest", app.url))
        .header("X-Galvolite-Token", &token)
        .json(&HookIngestRequest {
            session_id: id.to_string(),
            hook: HookKind::SessionStart,
            ts: "2026-04-18T00:00:00Z".into(),
            event_id: "evt-1".into(),
            payload: json!({"is_resume": false}),
        })
        .send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body: HookIngestResponse = res.json().await.unwrap();
    assert_eq!(body.action, HookAction::Proceed);

    // Event row landed
    let conn = app.db.pool().get().unwrap();
    let rows: i64 = conn.query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'hook.SessionStart'",
        [id.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn rejects_envelope_session_token_mismatch() {
    let app = test_app().await;
    let (_, token_a) = seed_session(&app.db).await;
    let (id_b, _) = seed_session(&app.db).await;
    let res = app.client.post(format!("{}/hooks/ingest", app.url))
        .header("X-Galvolite-Token", &token_a)
        .json(&HookIngestRequest {
            session_id: id_b.to_string(), // mismatched
            hook: HookKind::SessionStart,
            ts: "2026-04-18T00:00:00Z".into(),
            event_id: "evt-2".into(),
            payload: json!({}),
        })
        .send().await.unwrap();
    assert_eq!(res.status(), 401);
}
```

`seed_session` should `INSERT` a session row with a known `api_token` and return both. `test_app` is whatever pattern existing API integration tests already use (look at `tests/api_*.rs` for Sub-plan 1's harness).

- [ ] **Step 2: Implement the handler**

```rust
// src/api/hooks.rs
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use crate::{
    db::Db,
    error::Error,
    hooks::types::*,
    state::{self, EventInput, Source},
};
use std::sync::Arc;
use subtle::ConstantTimeEq;

pub async fn ingest(
    State(db): State<Arc<Db>>,
    headers: HeaderMap,
    Json(req): Json<HookIngestRequest>,
) -> Result<Json<HookIngestResponse>, (StatusCode, String)> {
    let token = headers
        .get("X-Galvolite-Token")
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing token".into()))?;

    // Resolve token → session_id with constant-time compare.
    let session_id = resolve_token(&db, token).map_err(|e| match e {
        Error::SessionNotFound(_) => (StatusCode::UNAUTHORIZED, "bad token".into()),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    })?;

    // Verify envelope's session_id matches token-resolved session_id.
    let envelope_id = uuid::Uuid::parse_str(&req.session_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid session_id".into()))?;
    if envelope_id != session_id {
        return Err((StatusCode::UNAUTHORIZED, "session_id mismatch".into()));
    }

    // Compute event kind.
    let kind = req.hook.as_event_kind();

    // Apply.
    let outcome = state::apply_event(&db, session_id, EventInput {
        source: Source::Hook,
        kind,
        payload: req.payload.clone(),
        dedup_key: Some(req.event_id.clone()),
    }).map_err(|e| match e {
        Error::TerminalState { .. } => (StatusCode::CONFLICT, e.to_string()),
        Error::EventPayload(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        Error::SessionNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    })?;

    let _ = outcome; // Per-hook semantic responses added in Tasks 7-9.

    Ok(Json(HookIngestResponse {
        action: HookAction::Proceed,
        inject_text: None,
        note: None,
    }))
}

fn resolve_token(db: &Db, token: &str) -> Result<uuid::Uuid, Error> {
    // Pull all live tokens; constant-time compare to avoid timing oracle.
    // For O(N) sessions this is fine at the session counts we expect (<100).
    let conn = db.pool().get()?;
    let mut stmt = conn.prepare("SELECT id, api_token FROM sessions WHERE api_token IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let token_bytes = token.as_bytes();
    for row in rows {
        let (id, t) = row?;
        if t.as_bytes().ct_eq(token_bytes).unwrap_u8() == 1 {
            return Ok(uuid::Uuid::parse_str(&id).unwrap());
        }
    }
    Err(Error::SessionNotFound("token".into()))
}
```

- [ ] **Step 3: Mount the route**

In whichever file holds the router setup:

```rust
.route("/hooks/ingest", post(crate::api::hooks::ingest))
```

- [ ] **Step 4: Tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/api/hooks.rs src/api/mod.rs tests/api_hooks_ingest.rs
git commit -m "$(cat <<'EOF'
feat(api): add /hooks/ingest endpoint with bearer auth + apply_event funnel

POST /hooks/ingest authenticates via X-Galvolite-Token (constant-time compare
against sessions.api_token), validates the envelope, and routes the hook into
state::apply_event with dedup_key = event_id and kind = "hook.<HookKind>".

Maps Error::TerminalState → 409, Error::EventPayload → 400, Error::SessionNotFound
→ 404 (typed errors from Task 1).

Per-hook semantic responses (inject_text, action) come in Tasks 7–9. For now
every response is action=proceed.
EOF
)"
```

---

## Task 6: Spawn-time `.claude/settings.json` writer

**Goal:** When `SessionManager::spawn_session` runs, write a `.claude/settings.json` into the worktree whose hook table dispatches all 7 hook entries to the resolved `galvolite-hook` binary path. Path resolution: `GALVOLITE_HOOK_BIN` env var if set, else `std::env::current_exe()` parent dir + `galvolite-hook`. The file is overwritten on respawn.

**Files:**
- Create: `src/spawn_config.rs`
- Modify: `src/session_manager.rs` — call `spawn_config::write_session_files(worktree, &session)` in spawn flow, just before PTY spawn
- Modify: `src/lib.rs` — `pub mod spawn_config;`
- Create: `tests/spawn_config_writer.rs`

- [ ] **Step 1: Write failing test**

```rust
// tests/spawn_config_writer.rs
use galvolite::spawn_config;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn writes_settings_json_with_seven_hook_entries() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_settings_json(tmp.path(), "/path/to/galvolite-hook").unwrap();

    let path = tmp.path().join(".claude/settings.json");
    assert!(path.exists());

    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let hooks = v["hooks"].as_object().expect(".hooks must be an object");

    for kind in ["SessionStart", "SessionEnd", "PreCompact", "Stop",
                 "UserPromptSubmit", "PostToolUse", "Notification"] {
        assert!(hooks.contains_key(kind), "missing hook entry: {kind}");
    }

    // Each entry must point at the real binary path.
    let stop = &hooks["Stop"];
    let cmd_str = serde_json::to_string(stop).unwrap();
    assert!(cmd_str.contains("/path/to/galvolite-hook"), "binary path missing in: {cmd_str}");
}

#[test]
fn writes_into_subdir_creating_it_if_missing() {
    let tmp = TempDir::new().unwrap();
    // No .claude dir yet
    spawn_config::write_settings_json(tmp.path(), "/bin/hook").unwrap();
    assert!(tmp.path().join(".claude").is_dir());
}

#[test]
fn rewrite_on_second_call_overwrites() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_settings_json(tmp.path(), "/v1/hook").unwrap();
    spawn_config::write_settings_json(tmp.path(), "/v2/hook").unwrap();
    let body = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
    assert!(!body.contains("/v1/"));
    assert!(body.contains("/v2/"));
}

#[test]
fn resolves_binary_path_from_env_var_when_set() {
    std::env::set_var("GALVOLITE_HOOK_BIN", "/from/env/galvolite-hook");
    let path = spawn_config::resolve_hook_binary().unwrap();
    assert_eq!(path.to_string_lossy(), "/from/env/galvolite-hook");
    std::env::remove_var("GALVOLITE_HOOK_BIN");
}

#[test]
fn resolves_binary_path_from_current_exe_when_env_unset() {
    std::env::remove_var("GALVOLITE_HOOK_BIN");
    let path = spawn_config::resolve_hook_binary().unwrap();
    let s = path.to_string_lossy();
    assert!(s.ends_with("galvolite-hook") || s.ends_with("galvolite-hook.exe"),
        "should derive from current_exe, got: {s}");
}
```

- [ ] **Step 2: Implement**

```rust
// src/spawn_config.rs
use serde_json::json;
use std::path::{Path, PathBuf};

pub fn resolve_hook_binary() -> std::io::Result<PathBuf> {
    if let Ok(p) = std::env::var("GALVOLITE_HOOK_BIN") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| std::io::Error::other("no parent dir"))?;
    Ok(dir.join("galvolite-hook"))
}

pub fn write_settings_json(worktree: &Path, hook_bin: impl AsRef<str>)
    -> std::io::Result<()>
{
    let dot_claude = worktree.join(".claude");
    std::fs::create_dir_all(&dot_claude)?;

    let bin = hook_bin.as_ref();
    // CC's settings.json hook format — confirm exact shape from CC docs at integration time.
    let body = json!({
        "hooks": {
            "SessionStart":     [{"command": bin, "args": ["SessionStart"]}],
            "SessionEnd":       [{"command": bin, "args": ["SessionEnd"]}],
            "PreCompact":       [{"command": bin, "args": ["PreCompact"]}],
            "Stop":             [{"command": bin, "args": ["Stop"]}],
            "UserPromptSubmit": [{"command": bin, "args": ["UserPromptSubmit"]}],
            "PostToolUse":      [{"command": bin, "args": ["PostToolUse"], "tool": "Bash"}],
            "Notification":     [{"command": bin, "args": ["Notification"]}],
        }
    });

    let path = dot_claude.join("settings.json");
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap())?;
    Ok(())
}
```

- [ ] **Step 3: Wire into `SessionManager::spawn_session`**

Find the spawn flow in `src/session_manager.rs` (after the worktree is resolved, before PTY spawn):

```rust
let hook_bin = crate::spawn_config::resolve_hook_binary()?;
crate::spawn_config::write_settings_json(&worktree_path, hook_bin.to_string_lossy().as_ref())?;
```

If failure here should orphan the session, route through `apply_event(system.session_orphaned, reason: "settings_write_failed")` per Task 2 pattern.

- [ ] **Step 4: Tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/spawn_config.rs src/session_manager.rs src/lib.rs tests/spawn_config_writer.rs
git commit -m "$(cat <<'EOF'
feat(spawn): write per-session .claude/settings.json with 7 hook entries

SessionManager::spawn_session now writes <worktree>/.claude/settings.json
pointing all seven hook entries at the resolved galvolite-hook binary.

Path resolution: GALVOLITE_HOOK_BIN env var if set, else current_exe parent dir
+ "galvolite-hook". Tests inject the env var; production resolves from the
running galvolite binary's directory.

The .mcp.json writer is deferred to Sub-plan 3.
EOF
)"
```

---

## Task 7: Lifecycle hook handlers — `SessionStart`, `SessionEnd`

**Goal:** Per-hook semantics for the two lifecycle hooks:
- `SessionStart`: build seed text from role profile (`CLAUDE.md`-style content for the session's `role`), last handoff pointer, and current MCP tool pointer. Return as `inject_text`. Replaces ad-hoc prompt injection currently in `session_manager.rs`.
- `SessionEnd`: clean exit (exit_code 0) → state `Ended`; unclean → `Error`. If no `write_handoff` event in the session's last N events (use 50 as the window), auto-generate minimal handoff from event log + PTY scrollback and write `.galvolite/handoffs/{session_id}/pointer.json`.

**Files:**
- Create: `src/api/hook_handlers/mod.rs`
- Create: `src/api/hook_handlers/lifecycle.rs`
- Modify: `src/api/hooks.rs` — `ingest` dispatches per `HookKind` into `hook_handlers::lifecycle::on_session_start` / `on_session_end` for those two; default `Proceed` for others (until Tasks 8 + 9)
- Create: `tests/hooks_lifecycle.rs`

- [ ] **Step 1: Write failing tests** (3 tests)

```rust
// tests/hooks_lifecycle.rs

#[tokio::test]
async fn session_start_returns_seed_text_with_role_profile() {
    let app = test_app().await;
    let (id, token) = seed_session_with_role(&app.db, "implementation").await;

    let resp = post_hook(&app, &token, &id, HookKind::SessionStart, json!({"is_resume": false})).await;
    assert_eq!(resp.action, HookAction::Proceed);
    let inject = resp.inject_text.expect("seed text required");
    assert!(inject.to_lowercase().contains("implementation"),
        "should mention role: {inject}");
}

#[tokio::test]
async fn session_end_clean_exit_transitions_to_ended() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::SessionEnd,
        json!({"exit_code": 0, "reason": "user_quit"})).await;

    assert_eq!(state_of(&app.db, id), SessionState::Ended);
}

#[tokio::test]
async fn session_end_unclean_transitions_to_error() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::SessionEnd,
        json!({"exit_code": 137, "reason": "killed"})).await;

    assert_eq!(state_of(&app.db, id), SessionState::Error);
}

#[tokio::test]
async fn session_end_auto_writes_handoff_pointer_if_no_recent_write_handoff() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::SessionEnd,
        json!({"exit_code": 0, "reason": "ok"})).await;

    let pointer = std::path::PathBuf::from(format!(".galvolite/handoffs/{id}/pointer.json"));
    assert!(pointer.exists(), "auto-handoff pointer not written");
}
```

`seed_session_with_role`, `seed_session_in_state`, `post_hook`, `state_of` are test helpers — extend the harness from Task 5.

- [ ] **Step 2: Implement `on_session_start`**

```rust
// src/api/hook_handlers/lifecycle.rs
use crate::{db::Db, hooks::types::*};
use std::sync::Arc;
use uuid::Uuid;

pub fn on_session_start(db: &Db, session_id: Uuid, payload: &serde_json::Value)
    -> Option<String>
{
    let role = db.get_session_role(session_id).ok()?;
    let mut seed = String::new();

    seed.push_str(&format!("# Role: {role}\n\n"));
    seed.push_str(&render_role_profile(&role));
    seed.push_str("\n\n");

    if payload.get("is_resume").and_then(|v| v.as_bool()).unwrap_or(false) {
        if let Some(handoff) = last_handoff_summary(db, session_id) {
            seed.push_str("## Last handoff\n\n");
            seed.push_str(&handoff);
            seed.push_str("\n\n");
        }
    }

    seed.push_str("## MCP coordination\n\n");
    seed.push_str("Active coordination via MCP tools at `galvolite://`. ");
    seed.push_str("Use status_report, set_plan, write_handoff to communicate state.\n");

    Some(seed)
}

fn render_role_profile(role: &str) -> String {
    // Minimal placeholder profile — Sub-plan 4+ may pull from .galvolite/profiles/<role>.md.
    match role {
        "implementation" => "You are an implementation worker. Focus on writing tested code.".into(),
        "brainstorm"     => "You are a brainstorming partner. Help shape ideas before code.".into(),
        "orchestrator"   => "You are an orchestrator. Coordinate other sessions; do not write code yourself.".into(),
        "scout"          => "You are a scout. Explore unfamiliar code and report findings.".into(),
        other            => format!("You are a {other} worker."),
    }
}

fn last_handoff_summary(db: &Db, session_id: Uuid) -> Option<String> {
    let conn = db.pool().get().ok()?;
    let row: Option<String> = conn.query_row(
        "SELECT payload_json FROM events
         WHERE session_id = ?1 AND kind = 'mcp.write_handoff'
         ORDER BY ts DESC LIMIT 1",
        [session_id.to_string()],
        |r| r.get(0),
    ).ok();
    row.and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
        .and_then(|v| v.get("summary").and_then(|s| s.as_str()).map(String::from))
}
```

- [ ] **Step 3: Implement `on_session_end`**

```rust
pub fn on_session_end(db: &Db, session_id: Uuid, payload: &serde_json::Value) {
    let exit_code = payload.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(-1);
    let target_state = if exit_code == 0 {
        crate::state::SessionState::Ended
    } else {
        crate::state::SessionState::Error
    };

    // The apply_event call from /hooks/ingest already wrote a hook.SessionEnd event
    // and ran transition() — but transition() needs to know clean vs unclean. The
    // simplest route: emit a follow-up system event with the resolved state.
    let kind = if exit_code == 0 { "system.session_ended" } else { "system.session_errored" };
    let _ = crate::state::apply_event(db, session_id, crate::state::EventInput {
        source: crate::state::Source::System,
        kind: kind.into(),
        payload: serde_json::json!({"exit_code": exit_code}),
        dedup_key: Some(format!("session-end:{session_id}:{exit_code}")),
    });

    // Auto-handoff if missing.
    if !has_recent_write_handoff(db, session_id, 50) {
        let _ = write_auto_handoff(db, session_id);
    }
}

fn has_recent_write_handoff(db: &Db, session_id: Uuid, window: usize) -> bool {
    let conn = match db.pool().get() { Ok(c) => c, Err(_) => return false };
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM (
            SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT ?2
         ) WHERE kind = 'mcp.write_handoff'",
        rusqlite::params![session_id.to_string(), window as i64],
        |r| r.get(0),
    ).unwrap_or(0);
    count > 0
}

fn write_auto_handoff(db: &Db, session_id: Uuid) -> std::io::Result<()> {
    let dir = std::path::PathBuf::from(format!(".galvolite/handoffs/{session_id}"));
    std::fs::create_dir_all(&dir)?;
    let summary = build_summary_from_events(db, session_id);
    let pointer = serde_json::json!({
        "session_id": session_id.to_string(),
        "auto_generated": true,
        "summary": summary,
    });
    std::fs::write(dir.join("pointer.json"), serde_json::to_string_pretty(&pointer).unwrap())
}

fn build_summary_from_events(db: &Db, session_id: Uuid) -> String {
    // Minimal: list the last 10 event kinds. Sub-plan 5 may upgrade this.
    let conn = match db.pool().get() { Ok(c) => c, Err(_) => return "(no db)".into() };
    let mut stmt = conn.prepare(
        "SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT 10",
    ).unwrap();
    let kinds: Vec<String> = stmt.query_map([session_id.to_string()], |r| r.get(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    format!("Recent activity: {}", kinds.join(", "))
}
```

- [ ] **Step 4: Update transition table for new system event kinds**

In `src/state.rs::transition`, add arms:

```rust
(s, "system.session_ended")    if !s.is_terminal() => SessionState::Ended,
(s, "system.session_errored")  if !s.is_terminal() => SessionState::Error,
```

(`system.session_orphaned` from Task 2 already exists.)

- [ ] **Step 5: Wire `ingest` to call lifecycle handlers**

In `src/api/hooks.rs::ingest`, after `apply_event` succeeds:

```rust
let inject = match req.hook {
    HookKind::SessionStart => crate::api::hook_handlers::lifecycle::on_session_start(&db, session_id, &req.payload),
    HookKind::SessionEnd   => { crate::api::hook_handlers::lifecycle::on_session_end(&db, session_id, &req.payload); None }
    _ => None,
};

Ok(Json(HookIngestResponse {
    action: HookAction::Proceed,
    inject_text: inject,
    note: None,
}))
```

- [ ] **Step 6: Tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/api/hook_handlers/ src/api/hooks.rs src/state.rs tests/hooks_lifecycle.rs
git commit -m "$(cat <<'EOF'
feat(hooks): SessionStart returns role-keyed seed text; SessionEnd auto-handoff

SessionStart: builds seed text combining role profile, last handoff summary
(when is_resume), and MCP tool pointer. Returned as inject_text — claude
ingests it as session-start context, replacing the ad-hoc prompt injection
previously done in SessionManager::spawn_session.

SessionEnd: clean exit (exit_code 0) → Ended; unclean → Error. If no recent
mcp.write_handoff event in the last 50 entries, auto-generates a minimal
handoff from event log and writes .galvolite/handoffs/{id}/pointer.json.

Adds system.session_ended / system.session_errored arms to transition table.
EOF
)"
```

---

## Task 8: Turn-tracking hook handlers — `UserPromptSubmit`, `Stop`, `PreCompact`

**Goal:** Per-hook semantics for the three hooks that drive within-session turn flow:
- `UserPromptSubmit`: increment `sessions.turn_count`, accumulate `sessions.est_tokens`. At ≥80% of context limit (32k for default Claude), inject warning text. Drives `Idle → Working` transition (now possible because Task 3 reverted to spec §8.6).
- `Stop`: query event log for consecutive `hook.Stop` events since last `mcp.status_report` or `hook.UserPromptSubmit`. <3 → return `inject_text: "report status or continue"`. ≥3 → just transition `Working → Idle` (no injection). After this hook fires, spawn a 3 s timer task that, if no `hook.UserPromptSubmit` arrives, marks the session `AwaitingInput`.
- `PreCompact`: if no `mcp.write_handoff` event since last checkpoint, return `inject_text: "write_handoff now (compaction imminent)"`. Add `tracing::warn!("PreCompact inject_text capability unverified — see spec §15 dep #3")`.

**> After Task 8 commits, execute Task 3 (the deferred `Starting → Idle` normalization).**

**Files:**
- Create: `src/api/hook_handlers/turn.rs`
- Modify: `src/api/hooks.rs::ingest` dispatch
- Modify: `src/state.rs` — add transitions for `hook.UserPromptSubmit`, `hook.Stop`
- Create: `tests/hooks_turn.rs`

- [ ] **Step 1: Write failing tests** (5 tests)

```rust
// tests/hooks_turn.rs

#[tokio::test]
async fn user_prompt_submit_transitions_idle_to_working_increments_counter() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Idle).await;

    let _ = post_hook(&app, &token, &id, HookKind::UserPromptSubmit,
        json!({"prompt_preview": "Hello", "est_tokens": 50})).await;

    assert_eq!(state_of(&app.db, id), SessionState::Working);
    let (turns, tokens) = counters_of(&app.db, id);
    assert_eq!(turns, 1);
    assert_eq!(tokens, 50);
}

#[tokio::test]
async fn user_prompt_submit_at_80_percent_injects_warning() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Idle).await;
    bump_tokens(&app.db, id, 25_000); // < 80%

    let resp = post_hook(&app, &token, &id, HookKind::UserPromptSubmit,
        json!({"prompt_preview": "x", "est_tokens": 1500})).await;
    // 26500 / 32000 = 82.8% — should inject
    assert!(resp.inject_text.is_some(), "should warn at 80% context");
    assert!(resp.inject_text.unwrap().to_lowercase().contains("context"));
}

#[tokio::test]
async fn stop_first_two_inject_report_status_or_continue() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let r1 = post_hook(&app, &token, &id, HookKind::Stop, json!({"turn_id": "t1"})).await;
    assert!(r1.inject_text.is_some());

    let r2 = post_hook(&app, &token, &id, HookKind::Stop, json!({"turn_id": "t2"})).await;
    assert!(r2.inject_text.is_some());
}

#[tokio::test]
async fn stop_third_consecutive_transitions_working_to_idle_no_inject() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    for n in 1..=3 {
        let _ = post_hook(&app, &token, &id, HookKind::Stop,
            json!({"turn_id": format!("t{n}")})).await;
    }
    let final_state = state_of(&app.db, id);
    assert_eq!(final_state, SessionState::Idle);
}

#[tokio::test]
async fn precompact_with_no_recent_write_handoff_injects_handoff_prompt() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let resp = post_hook(&app, &token, &id, HookKind::PreCompact,
        json!({"trigger_reason": "context_full"})).await;
    let inject = resp.inject_text.expect("should inject handoff prompt");
    assert!(inject.to_lowercase().contains("write_handoff"));
}

#[tokio::test]
async fn precompact_with_recent_write_handoff_does_not_inject() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;
    record_event(&app.db, id, "mcp.write_handoff", json!({"summary": "done"})).await;

    let resp = post_hook(&app, &token, &id, HookKind::PreCompact,
        json!({"trigger_reason": "manual"})).await;
    assert!(resp.inject_text.is_none());
}
```

- [ ] **Step 2: Implement `turn.rs`**

```rust
// src/api/hook_handlers/turn.rs
use crate::{db::Db, hooks::types::*};
use uuid::Uuid;

const CONTEXT_LIMIT: i64 = 32_000;
const STOP_RETRY_LIMIT: i64 = 3;

pub fn on_user_prompt_submit(db: &Db, session_id: Uuid, payload: &serde_json::Value)
    -> Option<String>
{
    let est = payload.get("est_tokens").and_then(|v| v.as_i64()).unwrap_or(0);

    // Bump counters atomically with the event row that's already been written.
    let conn = db.pool().get().ok()?;
    conn.execute(
        "UPDATE sessions SET turn_count = COALESCE(turn_count,0)+1,
                              est_tokens = COALESCE(est_tokens,0)+?2
         WHERE id = ?1",
        rusqlite::params![session_id.to_string(), est],
    ).ok()?;

    let total: i64 = conn.query_row(
        "SELECT COALESCE(est_tokens, 0) FROM sessions WHERE id = ?1",
        [session_id.to_string()],
        |r| r.get(0),
    ).unwrap_or(0);

    if total * 100 >= CONTEXT_LIMIT * 80 {
        Some(format!(
            "⚠️ Context approaching limit ({total}/{CONTEXT_LIMIT} tokens, {}%). \
             Consider write_handoff before context runs out.",
            (total * 100) / CONTEXT_LIMIT
        ))
    } else {
        None
    }
}

pub fn on_stop(db: &Db, session_id: Uuid) -> Option<String> {
    let conn = db.pool().get().ok()?;
    // Count consecutive hook.Stop events since last status_report or UserPromptSubmit.
    let mut stmt = conn.prepare(
        "SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT 5",
    ).ok()?;
    let kinds: Vec<String> = stmt.query_map([session_id.to_string()], |r| r.get(0))
        .ok()?.filter_map(Result::ok).collect();

    let mut consecutive = 0;
    for k in &kinds {
        if k == "hook.Stop" { consecutive += 1; }
        else if k == "mcp.status_report" || k == "hook.UserPromptSubmit" { break; }
        else { /* keep counting through unrelated events */ }
    }

    if consecutive >= STOP_RETRY_LIMIT {
        // Transition to Idle — emit a synthetic event so the funnel records it.
        let _ = crate::state::apply_event(db, session_id, crate::state::EventInput {
            source: crate::state::Source::System,
            kind: "system.stop_max_retries".into(),
            payload: serde_json::json!({"consecutive": consecutive}),
            dedup_key: None,
        });
        None
    } else {
        Some("Please report your status (status_report) or continue working.".into())
    }
}

pub fn on_pre_compact(db: &Db, session_id: Uuid) -> Option<String> {
    tracing::warn!(
        "PreCompact inject_text capability unverified — see spec §15 dep #3"
    );

    let conn = db.pool().get().ok()?;
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM (
            SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT 50
         ) WHERE kind = 'mcp.write_handoff'",
        [session_id.to_string()],
        |r| r.get(0),
    ).unwrap_or(0);

    if count == 0 {
        Some("⚠️ Compaction imminent. Call write_handoff now to preserve continuity.".into())
    } else {
        None
    }
}
```

- [ ] **Step 3: Add new transition arms**

In `src/state.rs::transition`:

```rust
(Idle,    "hook.UserPromptSubmit") => SessionState::Working,
(AwaitingInput, "hook.UserPromptSubmit") => SessionState::Working,
(Working, "hook.UserPromptSubmit") => SessionState::Working, // no-op but valid
(Working, "hook.Stop")              => SessionState::Working, // intermediate; system.stop_max_retries flips it
(Working, "system.stop_max_retries") => SessionState::Idle,
(Working, "hook.PreCompact")        => SessionState::Working,
(_,       "hook.SessionStart")      => SessionState::Idle, // already added in Task 7
(_,       "hook.SessionEnd")        => /* leave to system.session_ended/errored */,
```

(Adjust per existing transition function shape — don't introduce empty-body arms; for `hook.SessionEnd` the handler emits the system follow-up so the hook event itself is just logged and shouldn't transition.)

- [ ] **Step 4: Wire `ingest` to call turn handlers**

```rust
// src/api/hooks.rs — extend the dispatch from Task 7
let inject = match req.hook {
    HookKind::SessionStart      => lifecycle::on_session_start(&db, session_id, &req.payload),
    HookKind::SessionEnd        => { lifecycle::on_session_end(&db, session_id, &req.payload); None }
    HookKind::UserPromptSubmit  => turn::on_user_prompt_submit(&db, session_id, &req.payload),
    HookKind::Stop              => turn::on_stop(&db, session_id),
    HookKind::PreCompact        => turn::on_pre_compact(&db, session_id),
    HookKind::PostToolUse | HookKind::Notification => None, // Task 9
};
```

- [ ] **Step 5: Implement the 3-second AwaitingInput watchdog after Stop**

After `on_stop`, schedule a tokio task:

```rust
// In on_stop or in ingest after dispatch, if HookKind::Stop:
tokio::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    // Re-check: if last event for this session is still hook.Stop and no UserPromptSubmit
    // arrived, emit system.awaiting_input event.
    let _ = crate::state::apply_event(&db, session_id, crate::state::EventInput {
        source: crate::state::Source::System,
        kind: "system.awaiting_input".into(),
        payload: serde_json::json!({}),
        dedup_key: Some(format!("awaiting:{session_id}:{}", chrono::Utc::now().timestamp())),
    });
});
```

Add transition: `(Idle, "system.awaiting_input") => AwaitingInput`. This may be over-eager (a quick UserPromptSubmit between Stop and the 3 s mark would still flip to Working, then this watchdog flips to AwaitingInput erroneously). Mitigation: the watchdog reads the events table and only fires the event if the last event for the session is still `hook.Stop`.

```rust
let conn = db.pool().get().unwrap();
let last_kind: Option<String> = conn.query_row(
    "SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT 1",
    [session_id.to_string()], |r| r.get(0)).ok();
if last_kind.as_deref() == Some("hook.Stop") {
    // proceed to apply_event
}
```

- [ ] **Step 6: Tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/api/hook_handlers/turn.rs src/api/hooks.rs src/state.rs tests/hooks_turn.rs
git commit -m "$(cat <<'EOF'
feat(hooks): UserPromptSubmit / Stop / PreCompact handlers

UserPromptSubmit: increment turn_count, accumulate est_tokens, transition
Idle/AwaitingInput → Working. Inject warning at ≥80% context.

Stop: query event log for consecutive hook.Stop events since last status_report
or UserPromptSubmit. <3 → inject "report status or continue". ≥3 → emit
system.stop_max_retries event that transitions Working → Idle. After fire,
schedule a 3s tokio task that emits system.awaiting_input if no
UserPromptSubmit arrives.

PreCompact: inject "write_handoff now" prompt if no recent mcp.write_handoff.
tracing::warn! flags spec §15 dep #3 as unverified for Sub-plan 5 smoke.

>>> After committing this task, execute Task 3 (Starting → Idle normalization).
EOF
)"
```

**> Now execute Task 3.** The deferred normalization can land cleanly because UserPromptSubmit will drive the Idle → Working transition that the spawn-time prompt injection used to do directly.

---

## Task 9: Observation hook handlers — `PostToolUse(Bash)`, `Notification`

**Goal:** Per-hook semantics for the two observation hooks:
- `PostToolUse(Bash)`: pattern-match `command` against `git push|git commit|cargo test|npm test`. Log structured events. `git push` raises attention via `system.attention_needed` event.
- `Notification`: mirror `level` + `message` to event log. Dashboard fan-out is Sub-plan 4's concern; for now we just log the event.

**Files:**
- Create: `src/api/hook_handlers/observe.rs`
- Modify: `src/api/hooks.rs::ingest` dispatch
- Modify: `src/state.rs` — add transitions for `hook.PostToolUse`, `hook.Notification`
- Create: `tests/hooks_observe.rs`

- [ ] **Step 1: Write failing tests** (4 tests)

```rust
// tests/hooks_observe.rs

#[tokio::test]
async fn post_tool_use_git_push_raises_attention() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::PostToolUse,
        json!({"tool": "Bash", "command": "git push origin main", "exit_code": 0,
               "stdout_preview": "..."})).await;

    assert_eq!(state_of(&app.db, id), SessionState::NeedsAttention);
}

#[tokio::test]
async fn post_tool_use_git_commit_logs_but_no_attention() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::PostToolUse,
        json!({"tool": "Bash", "command": "git commit -m 'x'", "exit_code": 0})).await;

    assert_eq!(state_of(&app.db, id), SessionState::Working);

    let conn = app.db.pool().get().unwrap();
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'hook.PostToolUse'",
        [id.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn post_tool_use_unrelated_command_logs_minimally() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::PostToolUse,
        json!({"tool": "Bash", "command": "ls -la", "exit_code": 0})).await;

    assert_eq!(state_of(&app.db, id), SessionState::Working);
}

#[tokio::test]
async fn notification_records_level_and_message() {
    let app = test_app().await;
    let (id, token) = seed_session_in_state(&app.db, SessionState::Working).await;

    let _ = post_hook(&app, &token, &id, HookKind::Notification,
        json!({"level": "warn", "message": "test passed but lints failed"})).await;

    let conn = app.db.pool().get().unwrap();
    let payload: String = conn.query_row(
        "SELECT payload_json FROM events WHERE session_id = ?1 AND kind = 'hook.Notification'",
        [id.to_string()], |r| r.get(0)).unwrap();
    assert!(payload.contains("warn"));
    assert!(payload.contains("test passed"));
}
```

- [ ] **Step 2: Implement**

```rust
// src/api/hook_handlers/observe.rs
use crate::db::Db;
use uuid::Uuid;

pub fn on_post_tool_use(db: &Db, session_id: Uuid, payload: &serde_json::Value)
    -> Option<String>
{
    let cmd = payload.get("command").and_then(|v| v.as_str()).unwrap_or("");

    if cmd.starts_with("git push") {
        let _ = crate::state::apply_event(db, session_id, crate::state::EventInput {
            source: crate::state::Source::System,
            kind: "system.attention_needed".into(),
            payload: serde_json::json!({"reason": "git_push_observed", "command": cmd}),
            dedup_key: None,
        });
    }
    None
}

pub fn on_notification(_db: &Db, _session_id: Uuid, _payload: &serde_json::Value)
    -> Option<String>
{
    // Event row is already written by /hooks/ingest's apply_event call.
    // Sub-plan 4 wires WS fan-out from this kind to dashboard toasts.
    None
}
```

- [ ] **Step 3: Add transition arms**

```rust
// src/state.rs::transition
(Working, "hook.PostToolUse")          => SessionState::Working, // no state change
(Idle,    "hook.PostToolUse")          => SessionState::Idle,
(_,       "hook.Notification")         => /* keep current */,
(s,       "system.attention_needed") if !s.is_terminal() => SessionState::NeedsAttention,
```

- [ ] **Step 4: Wire `ingest` dispatch**

```rust
HookKind::PostToolUse  => observe::on_post_tool_use(&db, session_id, &req.payload),
HookKind::Notification => observe::on_notification(&db, session_id, &req.payload),
```

- [ ] **Step 5: Tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/api/hook_handlers/observe.rs src/api/hooks.rs src/state.rs tests/hooks_observe.rs
git commit -m "$(cat <<'EOF'
feat(hooks): PostToolUse(Bash) and Notification handlers

PostToolUse: pattern-match command against git push|commit|cargo test|npm test.
git push emits system.attention_needed event → NeedsAttention state. Other
matched commands log structured events without state change.

Notification: event row written by ingest; handler is a no-op stub for now.
Sub-plan 4 will wire WS fan-out from hook.Notification kinds to dashboard
toasts.
EOF
)"
```

---

## Task 10: End-to-end smoke test (no real claude)

**Goal:** One large integration test that proves the full wire works without a PTY: spawn a session through `SessionManager`, exec the real `galvolite-hook` binary via `Command::new(env!("CARGO_BIN_EXE_galvolite-hook"))` against the running test Axum app, fire all 7 hooks in a realistic sequence, and verify event log + state transitions + `inject_text` responses for each.

**Files:**
- Create: `tests/hooks_smoke_endtoend.rs`

- [ ] **Step 1: Write the smoke test**

```rust
// tests/hooks_smoke_endtoend.rs
use galvolite::{db::Db, hooks::types::*, session_manager::SessionManager, state::SessionState};
use std::process::{Command, Stdio};
use std::io::Write;

#[tokio::test]
async fn full_hook_lifecycle_via_real_binary() {
    let app = test_app().await; // axum server on a port + Db handle
    let mgr = SessionManager::new(app.db.clone(), /* ... */);
    let (session_id, token) = mgr.spawn_for_test("implementation").await.unwrap();

    let bin = env!("CARGO_BIN_EXE_galvolite-hook");

    // 1. SessionStart — should return seed text
    let out = run_hook(bin, &app.url, &session_id, &token, "SessionStart", "{}");
    assert!(out.status.success());
    assert!(!out.stdout.is_empty(), "SessionStart should produce inject_text");
    assert_eq!(state_of(&app.db, session_id), SessionState::Idle);

    // 2. UserPromptSubmit — Idle → Working, no inject (under 80%)
    let out = run_hook(bin, &app.url, &session_id, &token, "UserPromptSubmit",
        r#"{"prompt_preview":"hi","est_tokens":100}"#);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(state_of(&app.db, session_id), SessionState::Working);

    // 3. PostToolUse(Bash) git commit — no state change
    let _ = run_hook(bin, &app.url, &session_id, &token, "PostToolUse",
        r#"{"tool":"Bash","command":"git commit -m x","exit_code":0}"#);
    assert_eq!(state_of(&app.db, session_id), SessionState::Working);

    // 4. Notification — log only
    let _ = run_hook(bin, &app.url, &session_id, &token, "Notification",
        r#"{"level":"info","message":"compile ok"}"#);

    // 5. Stop ×3 — first 2 inject, 3rd transitions to Idle
    let r1 = run_hook(bin, &app.url, &session_id, &token, "Stop", r#"{"turn_id":"t1"}"#);
    assert!(!r1.stdout.is_empty());
    let r2 = run_hook(bin, &app.url, &session_id, &token, "Stop", r#"{"turn_id":"t2"}"#);
    assert!(!r2.stdout.is_empty());
    let r3 = run_hook(bin, &app.url, &session_id, &token, "Stop", r#"{"turn_id":"t3"}"#);
    assert!(r3.stdout.is_empty());
    assert_eq!(state_of(&app.db, session_id), SessionState::Idle);

    // 6. PreCompact — should inject (no write_handoff yet)
    let r = run_hook(bin, &app.url, &session_id, &token, "PreCompact",
        r#"{"trigger_reason":"context_full"}"#);
    let body = String::from_utf8_lossy(&r.stdout).to_string();
    assert!(body.contains("write_handoff"));

    // 7. SessionEnd clean → Ended, auto-handoff written
    let _ = run_hook(bin, &app.url, &session_id, &token, "SessionEnd",
        r#"{"exit_code":0,"reason":"ok"}"#);
    assert_eq!(state_of(&app.db, session_id), SessionState::Ended);
    let pointer = format!(".galvolite/handoffs/{session_id}/pointer.json");
    assert!(std::path::Path::new(&pointer).exists(), "auto-handoff missing");

    // Event log shape: 7 hook.* events at minimum.
    let conn = app.db.pool().get().unwrap();
    let hook_events: i64 = conn.query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind LIKE 'hook.%'",
        [session_id.to_string()], |r| r.get(0)).unwrap();
    assert!(hook_events >= 7, "expected ≥7 hook.* events, got {hook_events}");
}

fn run_hook(bin: &str, url: &str, session: &uuid::Uuid, token: &str,
            hook: &str, payload: &str) -> std::process::Output {
    let mut child = Command::new(bin)
        .arg(hook)
        .env("GALVOLITE_API_BASE", url)
        .env("GALVOLITE_SESSION_ID", session.to_string())
        .env("GALVOLITE_TOKEN", token)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().unwrap();
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}
```

`mgr.spawn_for_test(role)` is a test-only helper that does the minimum to give us a session row + a worktree dir + a token, but NO actual claude PTY (we're not testing PTY behavior here). If such a helper doesn't exist, add one or use raw `db.insert_session` + token mint inline.

- [ ] **Step 2: Tests pass**

```bash
cargo test --test hooks_smoke_endtoend
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 3: Commit**

```bash
git add tests/hooks_smoke_endtoend.rs
git commit -m "$(cat <<'EOF'
test(hooks): end-to-end smoke — real binary, all 7 hooks, full lifecycle

Spawns a session row, exec's the compiled galvolite-hook binary via
CARGO_BIN_EXE_galvolite-hook against a running test Axum app, fires all 7
hooks in a realistic sequence, and verifies state transitions + event log +
inject_text responses for each.

Proves the full hook surface works without needing a real claude PTY. Real-
claude smoke (against actual `claude` binary) is Sub-plan 5's scope.
EOF
)"
```

---

## Self-review checklist

After completing the tasks above, run the following yourself:

**1. Spec coverage (§-by-§ scan):**

- §6.1 (envelope shape) — Task 5 ✓
- §6.2 SessionStart — Task 7 ✓
- §6.2 SessionEnd — Task 7 ✓
- §6.2 PreCompact — Task 8 (with §15 dep #3 warn) ✓
- §6.2 Stop — Task 8 (event-log retry counter, 3 s watchdog) ✓
- §6.2 UserPromptSubmit — Task 8 ✓
- §6.2 PostToolUse(Bash) — Task 9 ✓
- §6.2 Notification — Task 9 ✓
- §6.3 (atomicity invariant) — Tasks 5 + 7 + 8 + 9 ✓
- §7.1 step 4 (settings.json writer) — Task 6 ✓
- §7.2 (bearer auth, constant-time compare) — Task 5 ✓
- §8.3 (atomicity gap closed) — Task 2 ✓
- §8.6 (transition normalization) — Task 3 ✓

**2. Placeholder scan:**

```bash
grep -nE 'TBD|TODO|implement later|fill in details' \
    docs/superpowers/plans/2026-04-18-plan-2-hook-surface.md
```

Should return zero hits.

**3. Type consistency:**

- `HookKind`, `HookIngestRequest`, `HookIngestResponse`, `HookAction` — used consistently across binary + server ✓
- Event kind naming: `hook.<HookKind>` for hook-driven events; `system.<reason>` for synthetic ones ✓
- All `apply_event` calls include `source` (Hook for /hooks/ingest writes, System for synthetic) ✓
- Token compare uses `subtle::ConstantTimeEq` everywhere it appears ✓

**4. Test coverage:**

- `tests/error_typed_variants.rs` — Error split (Task 1)
- `tests/state_session_orphaned.rs` — orphan event + transition (Task 2)
- `tests/state_starting_to_idle.rs` — §8.6 normalization (Task 3)
- `tests/hooks_binary_client.rs`, `tests/hooks_binary_exec.rs` — binary skeleton (Task 4)
- `tests/api_hooks_ingest.rs` — endpoint contract (Task 5)
- `tests/spawn_config_writer.rs` — settings writer (Task 6)
- `tests/hooks_lifecycle.rs` — SessionStart/End (Task 7)
- `tests/hooks_turn.rs` — UserPromptSubmit/Stop/PreCompact (Task 8)
- `tests/hooks_observe.rs` — PostToolUse/Notification (Task 9)
- `tests/hooks_smoke_endtoend.rs` — full lifecycle (Task 10)

Existing tests from Sub-plan 1 (`db_*`, `state_*`, `session_manager_*`, `signals*`, etc.) all stay green.

---

## Done definition for Sub-plan 2

- `cargo test` passes (all existing + 10 new test files, ~30 new tests).
- `cargo clippy --all-targets -- -D warnings` clean across the new + modified files.
- `cargo build --bin galvolite-hook` produces a working binary.
- `tests/hooks_smoke_endtoend.rs` exec'ing the real binary against a running test app passes.
- Spawning a real session via `cargo run -- spawn` writes `.claude/settings.json` into the worktree with all 7 hook entries pointing at the resolved `galvolite-hook` path.
- A spawned `claude` (with this hook config in place) fires hooks that land as `hook.*` rows in `events`, observable via `sqlite3 ~/.galvolite/galvolite.db "SELECT kind, prev_state, next_state FROM events ORDER BY id DESC LIMIT 20"`.
- The §8.3 atomicity invariant is now whole: zero direct writes to `sessions.state` outside `apply_event`. Verify with `grep -rn 'update_session_state\|UPDATE sessions SET state' src/` returning only the call site inside `apply_event` itself.
- Sub-plan 3 (MCP) and Sub-plan 4 (dashboard) can now begin from this base. Sub-plan 5 (signal deletion + real-claude smoke + restart replay) becomes the final cleanup pass.
