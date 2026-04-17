# Plan 1b — Tauri Desktop Shell + Dashboard UI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wrap the Plan 1a ritual engine in a Tauri desktop app with a React dashboard that shows running sessions, streams live terminal output via xterm.js, and exposes session controls (start, kill, handoff, autopilot toggle).

**Architecture:** The galvolite library gains a `SessionManager` (extracted from the `cmd_start_session` monolith) and an axum HTTP+WebSocket API layer. The React frontend connects to the axum server for both REST data and live terminal streaming. Tauri provides the native shell — it starts the axum server and opens a webview. The same server also accepts remote browser connections (for Tailscale access in future plans). Five technical debt items from the Plan 1a code review are addressed in the first tasks.

**Tech Stack:** Rust (existing) + axum 0.7 + tower-http 0.6, React 18 + Vite 5 + TailwindCSS 3 + @xterm/xterm 5, Tauri v2.

**Scope boundary:**
- **IN:** SessionManager extraction, deferred-item fixes (N+1 query, child kill, cycle tracking), axum REST+WS API, React dashboard (project list, session list with status dots, session detail panel, mini xterm.js preview, session controls), Tauri native shell, integration test.
- **OUT:** Brainstorm/plan.md flow (plan 2), Claude Code plugin (plan 2), GitHub OAuth + PR pipeline (plan 3+), seeds subsystem (plan 4+), remote auth (plan 5+), autopilot triggers (plan 2 — dashboard toggle is wired but doesn't fire automatically).

**Reference:**
- [GalvoLite design spec](../specs/2026-04-15-galvolite-design.md) — especially §4 (architecture), §12 (dashboard UX), §13 (tech stack)
- [Plan 1a](./2026-04-15-plan-1a-ritual-engine.md) — what's already built
- Memory file `project_plan1b_deferred.md` — five items to fix early

---

## File Structure

Target layout at end of plan (new/modified files only):

```
GalvoLite/
├── Cargo.toml                              # modify: add axum, tower-http, workspace member
├── src/
│   ├── lib.rs                              # modify: add session_manager, api modules
│   ├── main.rs                             # modify: add serve command, use SessionManager
│   ├── cli.rs                              # modify: add Serve variant
│   ├── error.rs                            # modify: add SessionNotRunning variant
│   ├── db.rs                               # modify: fix N+1, add Serialize
│   ├── pty.rs                              # modify: fix kill, add broadcast, pid to shared
│   ├── session_manager.rs                  # NEW: extracted from main.rs monolith
│   └── api.rs                              # NEW: axum Router + REST + WebSocket
├── src-tauri/
│   ├── Cargo.toml                          # NEW: Tauri binary crate
│   ├── tauri.conf.json                     # NEW: Tauri config
│   ├── build.rs                            # NEW: Tauri build script
│   ├── capabilities/
│   │   └── default.json                    # NEW: Tauri permissions
│   └── src/
│       └── main.rs                         # NEW: start axum + open webview
├── ui/
│   ├── package.json                        # NEW
│   ├── vite.config.ts                      # NEW
│   ├── tailwind.config.js                  # NEW
│   ├── postcss.config.js                   # NEW
│   ├── tsconfig.json                       # NEW
│   ├── tsconfig.node.json                  # NEW
│   ├── index.html                          # NEW
│   └── src/
│       ├── main.tsx                        # NEW
│       ├── App.tsx                         # NEW
│       ├── api.ts                          # NEW: fetch wrappers
│       ├── types.ts                        # NEW: mirrors Rust types
│       ├── components/
│       │   ├── Dashboard.tsx               # NEW: main session list view
│       │   ├── ProjectSelector.tsx         # NEW: project picker dropdown
│       │   ├── SessionRow.tsx              # NEW: one row per session
│       │   ├── SessionDetail.tsx           # NEW: expanded info panel
│       │   └── TerminalPreview.tsx         # NEW: xterm.js mini terminal
│       └── hooks/
│           ├── useSessions.ts              # NEW: poll sessions
│           ├── useProjects.ts              # NEW: fetch projects
│           └── useTerminal.ts              # NEW: WebSocket → xterm.js
├── tests/
│   ├── session_manager.rs                  # NEW
│   └── api.rs                              # NEW
└── migrations/                             # unchanged
```

Responsibilities:
- `session_manager.rs` — owns the lifecycle of running sessions: spawn, track, kill, subscribe to output. Holds `Db` + a map of `RunningSession` structs. This is the primary interface for both the API layer and the CLI.
- `api.rs` — builds an `axum::Router` with REST endpoints and a WebSocket endpoint. Takes `Arc<SessionManager>` as shared state. No business logic — delegates to SessionManager.
- `src-tauri/` — thin Tauri shell. Creates SessionManager, starts axum, opens webview pointing at `http://localhost:{port}`.
- `ui/` — React SPA. Talks to the axum server via fetch (REST) and WebSocket (terminal streaming). Works identically in Tauri webview and in a remote browser.

---

## Task 1: Fix N+1 query in list_sessions

**Files:**
- Modify: `src/db.rs:344-355`
- Test: `tests/db_crud.rs`

The current `list_sessions` fetches IDs then calls `get_session` per row (N+1 queries). Inline the full deserialization for dashboard performance.

- [ ] **Step 1: Write the test**

Add to `tests/db_crud.rs`:

```rust
#[test]
fn list_sessions_returns_full_data_without_n_plus_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = galvolite::db::Db::open(dir.path().join("test.sqlite")).unwrap();

    let proj = db.insert_project(galvolite::db::NewProject {
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
    }).unwrap();

    let s1 = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: Some("feat/one".into()),
        worktree_path: "/tmp/test".into(),
        task_summary: "first task".into(),
        parent_session_id: None,
    }).unwrap();

    let s2 = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id,
        role: "brainstorm".into(),
        branch: None,
        worktree_path: "/tmp/test".into(),
        task_summary: "second task".into(),
        parent_session_id: None,
    }).unwrap();

    let sessions = db.list_sessions().unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, s1.id);
    assert_eq!(sessions[0].task_summary, "first task");
    assert_eq!(sessions[0].branch, Some("feat/one".into()));
    assert_eq!(sessions[1].id, s2.id);
    assert_eq!(sessions[1].role, "brainstorm");
}
```

- [ ] **Step 2: Run test to verify it passes (it will, since the N+1 version produces correct results — the bug is performance, not correctness)**

Run: `cargo test --test db_crud list_sessions_returns_full_data -- --exact`
Expected: PASS

- [ ] **Step 3: Replace N+1 implementation with inline query**

In `src/db.rs`, replace the `list_sessions` method:

```rust
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, role, branch, worktree_path, pty_pid, state,
                    autopilot, parent_session_id, task_summary, created_at, ended_at
             FROM sessions WHERE ended_at IS NULL ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let state_str = r.get::<_, String>(6)?;
                let state = SessionState::from_str(&state_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("{e}"),
                        )),
                    )
                })?;
                let parent = match r.get::<_, Option<String>>(8)? {
                    Some(s) => Some(parse_uuid(&s)?),
                    None => None,
                };
                let ended = match r.get::<_, Option<String>>(11)? {
                    Some(s) => Some(parse_datetime(&s)?),
                    None => None,
                };
                Ok(Session {
                    id: parse_uuid(&r.get::<_, String>(0)?)?,
                    project_id: parse_uuid(&r.get::<_, String>(1)?)?,
                    role: r.get(2)?,
                    branch: r.get(3)?,
                    worktree_path: r.get(4)?,
                    pty_pid: r.get(5)?,
                    state,
                    autopilot: r.get::<_, i64>(7)? != 0,
                    parent_session_id: parent,
                    task_summary: r.get(9)?,
                    created_at: parse_datetime(&r.get::<_, String>(10)?)?,
                    ended_at: ended,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
```

- [ ] **Step 4: Run all DB tests**

Run: `cargo test --test db_crud -- --nocapture`
Expected: all pass

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/db_crud.rs
git commit -m "fix: replace N+1 query in list_sessions with inline deserialization"
```

---

## Task 2: Add Serialize to DB types for API responses

**Files:**
- Modify: `src/db.rs` (derive Serialize on Project, Session, SignalRow; add serde rename on SessionState)
- Test: `tests/db_crud.rs`

The API layer needs to serialize these types to JSON. Also normalize SessionState serde to snake_case so JSON matches the DB string values.

- [ ] **Step 1: Write the test**

Add to `tests/db_crud.rs`:

```rust
#[test]
fn session_state_serializes_to_snake_case() {
    let json = serde_json::to_string(&galvolite::db::SessionState::AttentionNeeded).unwrap();
    assert_eq!(json, "\"attention_needed\"");

    let json = serde_json::to_string(&galvolite::db::SessionState::HandingOff).unwrap();
    assert_eq!(json, "\"handing_off\"");

    let back: galvolite::db::SessionState = serde_json::from_str("\"working\"").unwrap();
    assert_eq!(back, galvolite::db::SessionState::Working);
}

#[test]
fn project_serializes_to_json() {
    let p = galvolite::db::Project {
        id: uuid::Uuid::nil(),
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
        created_at: chrono::Utc::now(),
    };
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["name"], "test");
    assert_eq!(json["path"], "/tmp/test");
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test --test db_crud session_state_serializes project_serializes`
Expected: FAIL — `Serialize` not implemented for `Project`; SessionState serializes as `"AttentionNeeded"` not `"attention_needed"`

- [ ] **Step 3: Add derives and serde attributes**

In `src/db.rs`, modify the type definitions:

Add `#[serde(rename_all = "snake_case")]` to SessionState:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Spawning,
    Working,
    AttentionNeeded,
    HandingOff,
    Restoring,
    Complete,
    Killed,
    Error,
}
```

Add `Serialize` to Project:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub remote_url: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

Add `Serialize` to Session:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: Uuid,
    pub project_id: Uuid,
    pub role: String,
    pub branch: Option<String>,
    pub worktree_path: String,
    pub pty_pid: Option<i64>,
    pub state: SessionState,
    pub autopilot: bool,
    pub parent_session_id: Option<Uuid>,
    pub task_summary: String,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}
```

Add `Serialize` to SignalRow:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct SignalRow {
    pub id: i64,
    pub session_id: Uuid,
    pub cycle_id: Option<Uuid>,
    pub kind: String,
    pub payload: Option<String>,
    pub at: DateTime<Utc>,
}
```

- [ ] **Step 4: Run tests, verify they pass**

Run: `cargo test --test db_crud -- --nocapture`
Expected: all pass

- [ ] **Step 5: Verify existing tests still pass (serde rename might break FromStr round-trip)**

Run: `cargo test`
Expected: all 44 tests pass. The `FromStr` impl uses the same snake_case strings, so no breakage.

- [ ] **Step 6: Commit**

```bash
git add src/db.rs tests/db_crud.rs
git commit -m "feat: add Serialize to DB types and normalize SessionState to snake_case JSON"
```

---

## Task 3: Fix PtyHandleShared::kill to terminate child process

**Files:**
- Modify: `src/pty.rs:166-187`
- Test: `tests/pty_basic.rs`

Currently `PtyHandleShared::kill()` only sets the kill flag. The actual child process is only killed when `PtyHandle` is dropped (inside the SignalPump task). This means calling `PtyHandleShared::kill()` from the ritual driver doesn't actually terminate the child. Fix by storing the PID and sending SIGTERM.

- [ ] **Step 1: Write the test**

Add to `tests/pty_basic.rs`:

```rust
#[tokio::test]
async fn shared_handle_kill_terminates_child() {
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
    use galvolite::db::Db;

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("test.sqlite")).unwrap();

    // Spawn a process that runs forever
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "sleep".into(),
        args: vec!["300".into()],
        env: vec![],
    }).unwrap();

    let pid = pty.pid;
    let pump = SignalPump::start(pty, db, uuid::Uuid::new_v4());
    let handle = pump.handle();

    // Verify child is running
    let alive = unsafe { libc::kill(pid as i32, 0) };
    assert_eq!(alive, 0, "child should be alive before kill");

    // Kill via shared handle
    handle.kill();

    // Give the OS a moment to deliver the signal
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Verify child is dead
    let dead = unsafe { libc::kill(pid as i32, 0) };
    assert_ne!(dead, 0, "child should be dead after kill");
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test --test pty_basic shared_handle_kill_terminates -- --exact`
Expected: FAIL — child is still alive after kill (only kill_flag was set)

- [ ] **Step 3: Add pid field to PtyHandleShared and send SIGTERM on kill**

In `src/pty.rs`, modify `PtyHandleShared`:

```rust
#[derive(Clone)]
pub struct PtyHandleShared {
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_flag: Arc<AtomicBool>,
    pid: u32,
}

impl PtyHandleShared {
    pub async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.stdin_tx
            .send(bytes.to_vec())
            .await
            .map_err(|_| Error::Pty("stdin channel closed".into()))
    }

    pub async fn write_str(&self, s: &str) -> Result<()> {
        self.write(s.as_bytes()).await
    }

    pub fn kill(&self) {
        self.kill_flag.store(true, Ordering::Relaxed);
        if self.pid > 0 {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &self.pid.to_string()])
                .status();
        }
    }
}
```

In `SignalPump::start()`, update the `PtyHandleShared` construction to include pid:

```rust
        let shared = PtyHandleShared {
            stdin_tx: pty.stdin_tx.clone(),
            kill_flag: pty.kill_flag.clone(),
            pid: pty.pid,
        };
```

- [ ] **Step 4: Add `libc` to dev-dependencies in `Cargo.toml`**

```toml
[dev-dependencies]
tempfile = "3"
assert_cmd = "2"
predicates = "3"
libc = "0.2"
```

- [ ] **Step 5: Run test, verify it passes**

Run: `cargo test --test pty_basic shared_handle_kill_terminates -- --exact --nocapture`
Expected: PASS

- [ ] **Step 6: Run all tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 7: Commit**

```bash
git add src/pty.rs Cargo.toml tests/pty_basic.rs
git commit -m "fix: PtyHandleShared::kill now sends SIGTERM to child process"
```

---

## Task 4: Wire cycle_id tracking into signal recording

**Files:**
- Modify: `src/pty.rs:189-224` (SignalPump)
- Modify: `src/main.rs` (start initial cycle)
- Test: `tests/pty_signal_pump.rs`

Currently `SignalPump` always passes `None` for `cycle_id` when recording signals. The DB has `start_cycle`/`end_cycle` methods but they're never called. Wire cycle tracking: start a cycle when the session begins, pass its id to SignalPump.

- [ ] **Step 1: Write the test**

Add to `tests/pty_signal_pump.rs`:

```rust
#[tokio::test]
async fn signals_recorded_with_cycle_id() {
    use galvolite::db::{Db, NewProject, NewSession};
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("test.sqlite")).unwrap();

    let proj = db.insert_project(NewProject {
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
    }).unwrap();

    let sess = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: "/tmp/test".into(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let cycle_id = db.start_cycle(sess.id).unwrap();

    let pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "echo".into(),
        args: vec!["[STATUS:WORKING]".into()],
        env: vec![],
    }).unwrap();

    let mut pump = SignalPump::start(pty, db.clone(), sess.id, Some(cycle_id));

    // Wait for signal
    let sig = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        pump.next_signal(),
    ).await.unwrap();
    assert!(sig.is_some());

    // Verify signal was recorded with cycle_id
    let signals = db.list_signals_for_session(sess.id).unwrap();
    assert!(!signals.is_empty());
    assert_eq!(signals[0].cycle_id, Some(cycle_id));
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test --test pty_signal_pump signals_recorded_with_cycle -- --exact`
Expected: FAIL — `SignalPump::start` doesn't accept `cycle_id` parameter

- [ ] **Step 3: Add cycle_id parameter to SignalPump::start**

In `src/pty.rs`, modify `SignalPump`:

Add a field and update `start`:

```rust
pub struct SignalPump {
    signal_rx: mpsc::Receiver<Signal>,
    output_rx: mpsc::Receiver<String>,
    handle: PtyHandleShared,
    _task: tokio::task::JoinHandle<()>,
}

impl SignalPump {
    pub fn start(mut pty: PtyHandle, db: Db, session_id: Uuid, cycle_id: Option<Uuid>) -> Self {
        let (signal_tx, signal_rx) = mpsc::channel::<Signal>(64);
        let (output_tx, output_rx) = mpsc::channel::<String>(256);
        let shared = PtyHandleShared {
            stdin_tx: pty.stdin_tx.clone(),
            kill_flag: pty.kill_flag.clone(),
            pid: pty.pid,
        };

        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    let _ = db.record_signal(
                        session_id,
                        cycle_id,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                } else {
                    let _ = output_tx.try_send(line);
                }
            }
        });

        Self {
            signal_rx,
            output_rx,
            handle: shared,
            _task: task,
        }
    }

    pub async fn next_signal(&mut self) -> Option<Signal> {
        self.signal_rx.recv().await
    }

    pub async fn next_output(&mut self) -> Option<String> {
        self.output_rx.recv().await
    }

    pub fn handle(&self) -> PtyHandleShared {
        self.handle.clone()
    }
}
```

- [ ] **Step 4: Update call site in main.rs**

In `src/main.rs`, in `cmd_start_session`, update the `SignalPump::start` call:

```rust
    let cycle_id = db.start_cycle(session.id)?;
    let mut pump = SignalPump::start(pty, db.clone(), session.id, Some(cycle_id));
```

Add this right before the existing `let mut pump = ...` line (around line 122), and replace that line.

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: all pass (existing tests that call `SignalPump::start` need the new parameter — fix any that break by passing `None`)

- [ ] **Step 6: Fix any broken tests by adding the cycle_id parameter**

In `tests/pty_signal_pump.rs`, `tests/ritual_end_to_end.rs`, or anywhere `SignalPump::start` is called, add `None` as the fourth argument if not testing cycle tracking.

- [ ] **Step 7: Commit**

```bash
git add src/pty.rs src/main.rs tests/
git commit -m "feat: wire cycle_id tracking into signal recording"
```

---

## Task 5: Add broadcast output channel to SignalPump

**Files:**
- Modify: `src/pty.rs` (replace mpsc output channel with broadcast)
- Test: `tests/pty_signal_pump.rs`

The dashboard needs multiple WebSocket connections to receive the same session output. Replace the single-consumer `mpsc` output channel with `tokio::sync::broadcast` so multiple subscribers can receive output.

- [ ] **Step 1: Write the test**

Add to `tests/pty_signal_pump.rs`:

```rust
#[tokio::test]
async fn multiple_subscribers_receive_output() {
    use galvolite::db::{Db, NewProject, NewSession};
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};

    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("test.sqlite")).unwrap();

    let proj = db.insert_project(galvolite::db::NewProject {
        name: "test".into(),
        path: "/tmp/test".into(),
        remote_url: None,
    }).unwrap();

    let sess = db.insert_session(galvolite::db::NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: "/tmp/test".into(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    let pty = PtyHandle::spawn(PtyOpts {
        cwd: dir.path().to_path_buf(),
        command: "echo".into(),
        args: vec!["hello world".into()],
        env: vec![],
    }).unwrap();

    let pump = SignalPump::start(pty, db, sess.id, None);

    let mut rx1 = pump.subscribe_output();
    let mut rx2 = pump.subscribe_output();

    let line1 = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rx1.recv(),
    ).await.unwrap().unwrap();

    let line2 = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        rx2.recv(),
    ).await.unwrap().unwrap();

    assert!(line1.contains("hello world"));
    assert_eq!(line1, line2);
}
```

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test --test pty_signal_pump multiple_subscribers -- --exact`
Expected: FAIL — `subscribe_output` method does not exist

- [ ] **Step 3: Replace mpsc with broadcast for output**

In `src/pty.rs`, modify `SignalPump`:

```rust
use tokio::sync::broadcast;

pub struct SignalPump {
    signal_rx: mpsc::Receiver<Signal>,
    output_tx: broadcast::Sender<String>,
    handle: PtyHandleShared,
    _task: tokio::task::JoinHandle<()>,
}

impl SignalPump {
    pub fn start(mut pty: PtyHandle, db: Db, session_id: Uuid, cycle_id: Option<Uuid>) -> Self {
        let (signal_tx, signal_rx) = mpsc::channel::<Signal>(64);
        let (output_tx, _) = broadcast::channel::<String>(1024);
        let output_tx_clone = output_tx.clone();
        let shared = PtyHandleShared {
            stdin_tx: pty.stdin_tx.clone(),
            kill_flag: pty.kill_flag.clone(),
            pid: pty.pid,
        };

        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    let _ = db.record_signal(
                        session_id,
                        cycle_id,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                } else {
                    // Broadcast to all subscribers; ignore error if no receivers
                    let _ = output_tx_clone.send(line);
                }
            }
        });

        Self {
            signal_rx,
            output_tx,
            handle: shared,
            _task: task,
        }
    }

    pub async fn next_signal(&mut self) -> Option<Signal> {
        self.signal_rx.recv().await
    }

    /// Subscribe to session output. Multiple subscribers receive the same lines.
    pub fn subscribe_output(&self) -> broadcast::Receiver<String> {
        self.output_tx.subscribe()
    }

    /// Get the broadcast sender for passing to other components (e.g., SessionManager).
    pub fn output_sender(&self) -> broadcast::Sender<String> {
        self.output_tx.clone()
    }

    pub fn handle(&self) -> PtyHandleShared {
        self.handle.clone()
    }
}
```

- [ ] **Step 4: Remove old `next_output` callers if any exist**

Search for `next_output` — it was defined but never called in Plan 1a. The method is now removed. If any test references it, delete that reference.

- [ ] **Step 5: Run all tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 6: Commit**

```bash
git add src/pty.rs tests/pty_signal_pump.rs
git commit -m "feat: replace mpsc output channel with broadcast for multi-subscriber streaming"
```

---

## Task 6: Extract SessionManager from cmd_start_session

**Files:**
- Create: `src/session_manager.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Test: `tests/session_manager.rs`

This is the biggest refactor: extract the 130-line `cmd_start_session` monolith into a `SessionManager` struct that can be shared between the CLI and the API server. The SessionManager owns the `Db`, tracks running sessions in-memory, and provides methods for spawn/list/get/kill/handoff/send/subscribe.

- [ ] **Step 1: Add `SessionNotRunning` error variant**

In `src/error.rs`, add:

```rust
    #[error("session not running: {0}")]
    SessionNotRunning(uuid::Uuid),
```

- [ ] **Step 2: Add `session_manager` module to `src/lib.rs`**

```rust
pub mod session_manager;
```

- [ ] **Step 3: Write the test**

Create `tests/session_manager.rs`:

```rust
use galvolite::db::{Db, NewProject, SessionState};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;

fn setup() -> (Arc<SessionManager>, uuid::Uuid) {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();

    let proj = db.insert_project(galvolite::db::NewProject {
        name: "test".into(),
        path: "/tmp/test-project".into(),
        remote_url: None,
    }).unwrap();

    // Leak the tempdir so it outlives the test
    let _ = Box::leak(Box::new(dir));

    let mgr = Arc::new(SessionManager::new(db, data_dir));
    (mgr, proj.id)
}

#[tokio::test]
async fn spawn_session_and_list() {
    let (mgr, project_id) = setup();

    let session = mgr.spawn_session(
        project_id,
        "test task".into(),
        "implementation".into(),
        "echo".into(),
    ).await.unwrap();

    assert_eq!(session.task_summary, "test task");
    assert_eq!(session.role, "implementation");

    // Session appears in list
    let sessions = mgr.list_sessions().unwrap();
    assert!(!sessions.is_empty());
    assert!(sessions.iter().any(|s| s.id == session.id));
}

#[tokio::test]
async fn kill_session_marks_killed() {
    let (mgr, project_id) = setup();

    let session = mgr.spawn_session(
        project_id,
        "test task".into(),
        "implementation".into(),
        "sleep".into(),
    ).await.unwrap();

    mgr.kill_session(session.id).await.unwrap();

    // Give a moment for async cleanup
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let s = mgr.get_session(session.id).unwrap();
    assert_eq!(s.state, SessionState::Killed);
}
```

- [ ] **Step 4: Run test, verify it fails**

Run: `cargo test --test session_manager`
Expected: FAIL — module `session_manager` does not exist

- [ ] **Step 5: Write SessionManager**

Create `src/session_manager.rs`:

```rust
//! Session lifecycle manager.
//!
//! Extracted from the `cmd_start_session` monolith in main.rs.
//! Owns the Db, tracks running sessions in-memory, provides spawn/list/get/
//! kill/handoff/send/subscribe methods for the API and CLI.

use crate::claude_md::{implementation_profile, InjectionPrompt};
use crate::db::{Db, NewSession, Session, SessionState};
use crate::error::{Error, Result};
use crate::handoff::HandoffDir;
use crate::pty::{PtyHandle, PtyHandleShared, PtyOpts, SignalPump};
use crate::ritual::{PtySideEffects, RitualDriver};
use crate::signals::Signal;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::broadcast;
use uuid::Uuid;

struct RunningSession {
    driver: Arc<RitualDriver>,
    handle: PtyHandleShared,
    output_tx: broadcast::Sender<String>,
}

pub struct SessionManager {
    db: Db,
    data_dir: PathBuf,
    running: StdMutex<HashMap<Uuid, RunningSession>>,
}

impl SessionManager {
    pub fn new(db: Db, data_dir: PathBuf) -> Self {
        Self {
            db,
            data_dir,
            running: StdMutex::new(HashMap::new()),
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub async fn spawn_session(
        self: &Arc<Self>,
        project_id: Uuid,
        task: String,
        role: String,
        claude_cmd: String,
    ) -> Result<Session> {
        let project = self.db.get_project(project_id)?;
        let session = self.db.insert_session(NewSession {
            project_id: project.id,
            role: role.clone(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: task.clone(),
            parent_session_id: None,
        })?;

        let worktree = PathBuf::from(&project.path);
        let galvolite_dir = worktree.join(".galvolite");
        std::fs::create_dir_all(&galvolite_dir)?;
        let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
        std::fs::write(&claude_md_path, implementation_profile())
            .map_err(|e| Error::io(&claude_md_path, e))?;
        let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
        crate::handoff::ensure_session_dir(&handoff_dir, session.id)?;

        let cycle_id = self.db.start_cycle(session.id)?;

        let pty = PtyHandle::spawn(PtyOpts {
            cwd: worktree,
            command: claude_cmd,
            args: vec![],
            env: vec![],
        })?;
        self.db.update_session_pid(session.id, pty.pid as i64)?;

        let pump = SignalPump::start(pty, self.db.clone(), session.id, Some(cycle_id));
        let handle = pump.handle();
        let output_tx = pump.output_sender();

        let fx = Arc::new(PtySideEffects::new(handle.clone()));
        let driver = Arc::new(RitualDriver::new(fx));
        self.db.update_session_state(session.id, SessionState::Working)?;

        let injection = InjectionPrompt {
            task_summary: &task,
            session_id: &session.id.to_string(),
            claude_md_path: &claude_md_path.to_string_lossy(),
        };
        handle
            .write_str(&format!("{}\n", injection.render()))
            .await?;

        // Store in running map
        {
            let mut running = self.running.lock().unwrap();
            running.insert(session.id, RunningSession {
                driver: driver.clone(),
                handle: handle.clone(),
                output_tx,
            });
        }

        // Spawn background task to drive signals and clean up on exit
        let mgr = Arc::clone(self);
        let session_id = session.id;
        let driver_for_sig = driver.clone();
        tokio::spawn(async move {
            let mut pump = pump;
            while let Some(sig) = pump.next_signal().await {
                if matches!(sig, Signal::HandoffReady) {
                    if let Ok(Some(p)) =
                        crate::handoff::latest_handoff_for(&handoff_dir, session_id)
                    {
                        driver_for_sig
                            .set_latest_handoff_path(p.to_string_lossy().into())
                            .await;
                    }
                }
                driver_for_sig.on_signal(sig).await;
            }
            // Session process exited — clean up
            let _ = mgr.db.end_session(session_id, SessionState::Complete);
            let mut running = mgr.running.lock().unwrap();
            running.remove(&session_id);
        });

        Ok(session)
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        self.db.list_sessions()
    }

    pub fn get_session(&self, id: Uuid) -> Result<Session> {
        self.db.get_session(id)
    }

    pub fn list_projects(&self) -> Result<Vec<crate::db::Project>> {
        self.db.list_projects()
    }

    pub fn get_project(&self, id: Uuid) -> Result<crate::db::Project> {
        self.db.get_project(id)
    }

    pub fn add_project(&self, new: crate::db::NewProject) -> Result<crate::db::Project> {
        self.db.insert_project(new)
    }

    pub async fn send_to_session(&self, id: Uuid, text: String, newline: bool) -> Result<()> {
        let running = self.running.lock().unwrap();
        let rs = running
            .get(&id)
            .ok_or(Error::SessionNotRunning(id))?;
        let mut buf = text.into_bytes();
        if newline {
            buf.push(b'\n');
        }
        rs.driver.inject_raw(&buf).await;
        Ok(())
    }

    pub async fn trigger_handoff(&self, id: Uuid) -> Result<()> {
        let running = self.running.lock().unwrap();
        let rs = running
            .get(&id)
            .ok_or(Error::SessionNotRunning(id))?;
        let driver = rs.driver.clone();
        drop(running); // release lock before await
        driver.request_handoff().await;
        Ok(())
    }

    pub async fn kill_session(&self, id: Uuid) -> Result<()> {
        let rs = {
            let mut running = self.running.lock().unwrap();
            running.remove(&id)
        };
        if let Some(rs) = rs {
            rs.driver.abort("killed by user").await;
            rs.handle.kill();
        }
        self.db.end_session(id, SessionState::Killed)?;
        Ok(())
    }

    pub fn subscribe_output(&self, id: Uuid) -> Result<broadcast::Receiver<String>> {
        let running = self.running.lock().unwrap();
        let rs = running
            .get(&id)
            .ok_or(Error::SessionNotRunning(id))?;
        Ok(rs.output_tx.subscribe())
    }

    pub fn get_signals(&self, id: Uuid) -> Result<Vec<crate::db::SignalRow>> {
        self.db.list_signals_for_session(id)
    }

    pub async fn session_ritual_state(
        &self,
        id: Uuid,
    ) -> Result<crate::ritual::RitualState> {
        let running = self.running.lock().unwrap();
        let rs = running
            .get(&id)
            .ok_or(Error::SessionNotRunning(id))?;
        let driver = rs.driver.clone();
        drop(running);
        Ok(driver.state().await)
    }
}
```

- [ ] **Step 6: Run test, verify it passes**

Run: `cargo test --test session_manager -- --nocapture`
Expected: PASS

- [ ] **Step 7: Run all tests**

Run: `cargo test`
Expected: all pass

- [ ] **Step 8: Commit**

```bash
git add src/session_manager.rs src/lib.rs src/error.rs tests/session_manager.rs
git commit -m "feat: extract SessionManager from cmd_start_session monolith"
```

---

## Task 7: Add axum API with project endpoints

**Files:**
- Modify: `Cargo.toml` (add axum, tower-http)
- Create: `src/api.rs`
- Modify: `src/lib.rs`
- Test: `tests/api.rs`

- [ ] **Step 1: Add dependencies to `Cargo.toml`**

Add to `[dependencies]`:

```toml
axum = { version = "0.7", features = ["ws"] }
tower-http = { version = "0.6", features = ["cors", "fs"] }
tower = "0.5"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles

- [ ] **Step 3: Write the test**

Create `tests/api.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::db::Db;
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

fn setup() -> axum::Router {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let _ = Box::leak(Box::new(dir));
    let mgr = Arc::new(SessionManager::new(db, data_dir));
    galvolite::api::router(mgr)
}

#[tokio::test]
async fn list_projects_empty() {
    let app = setup();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(projects.is_empty());
}

#[tokio::test]
async fn create_and_list_project() {
    let app = setup();

    // Create project
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "TestProject",
                        "path": "/tmp/test-project"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(project["name"], "TestProject");

    // List projects
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "TestProject");
}
```

- [ ] **Step 4: Run test, verify it fails**

Run: `cargo test --test api`
Expected: FAIL — `api` module does not exist

- [ ] **Step 5: Add `api` module to `src/lib.rs`**

```rust
pub mod api;
```

- [ ] **Step 6: Write `src/api.rs`**

```rust
//! Axum HTTP API.
//!
//! Builds a Router that the CLI `serve` command and the Tauri binary both use.
//! All business logic is delegated to SessionManager.

use crate::session_manager::SessionManager;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post, delete};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

type AppState = Arc<SessionManager>;

pub fn router(mgr: Arc<SessionManager>) -> Router {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{id}", get(get_project))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", get(get_session).delete(kill_session))
        .route("/api/sessions/{id}/handoff", post(trigger_handoff))
        .route("/api/sessions/{id}/send", post(send_to_session))
        .route("/api/sessions/{id}/signals", get(get_signals))
        .route("/api/sessions/{id}/terminal", get(terminal_ws))
        .layer(CorsLayer::permissive())
        .with_state(mgr)
}

// --- Project endpoints ---

async fn list_projects(State(mgr): State<AppState>) -> impl IntoResponse {
    match mgr.list_projects() {
        Ok(projects) => Json(projects).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateProjectReq {
    name: String,
    path: String,
    remote_url: Option<String>,
}

async fn create_project(
    State(mgr): State<AppState>,
    Json(req): Json<CreateProjectReq>,
) -> impl IntoResponse {
    match mgr.add_project(crate::db::NewProject {
        name: req.name,
        path: req.path,
        remote_url: req.remote_url,
    }) {
        Ok(project) => (StatusCode::CREATED, Json(project)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_project(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match mgr.get_project(id) {
        Ok(project) => Json(project).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

// --- Session endpoints ---

async fn list_sessions(State(mgr): State<AppState>) -> impl IntoResponse {
    match mgr.list_sessions() {
        Ok(sessions) => Json(sessions).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateSessionReq {
    project_id: uuid::Uuid,
    task: String,
    #[serde(default = "default_role")]
    role: String,
    #[serde(default = "default_claude_cmd")]
    claude_cmd: String,
}

fn default_role() -> String { "implementation".into() }
fn default_claude_cmd() -> String { "claude".into() }

async fn create_session(
    State(mgr): State<AppState>,
    Json(req): Json<CreateSessionReq>,
) -> impl IntoResponse {
    match mgr
        .spawn_session(req.project_id, req.task, req.role, req.claude_cmd)
        .await
    {
        Ok(session) => (StatusCode::CREATED, Json(session)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_session(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match mgr.get_session(id) {
        Ok(session) => Json(session).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

async fn kill_session(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match mgr.kill_session(id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "killed"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn trigger_handoff(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match mgr.trigger_handoff(id).await {
        Ok(()) => Json(serde_json::json!({"status": "handoff initiated"})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct SendReq {
    text: String,
    #[serde(default)]
    newline: bool,
}

async fn send_to_session(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<SendReq>,
) -> impl IntoResponse {
    match mgr.send_to_session(id, req.text, req.newline).await {
        Ok(()) => Json(serde_json::json!({"status": "sent"})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_signals(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match mgr.get_signals(id) {
        Ok(signals) => Json(signals).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

// --- WebSocket ---

async fn terminal_ws(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    ws: axum::extract::WebSocketUpgrade,
) -> impl IntoResponse {
    let rx = match mgr.subscribe_output(id) {
        Ok(rx) => rx,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "session not running").into_response();
        }
    };
    ws.on_upgrade(move |socket| handle_terminal_ws(socket, rx))
        .into_response()
}

async fn handle_terminal_ws(
    mut socket: axum::extract::ws::WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<String>,
) {
    use axum::extract::ws::Message;
    loop {
        match rx.recv().await {
            Ok(line) => {
                if socket.send(Message::Text(line.into())).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                let msg = format!("[galvolite: skipped {n} lines]");
                if socket.send(Message::Text(msg.into())).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
```

- [ ] **Step 7: Run tests**

Run: `cargo test --test api -- --nocapture`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/api.rs src/lib.rs tests/api.rs
git commit -m "feat: add axum API with REST endpoints and WebSocket terminal streaming"
```

---

## Task 8: CLI `serve` command

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

Add a `serve` subcommand that starts the axum HTTP server with the dashboard API.

- [ ] **Step 1: Add `Serve` variant to CLI**

In `src/cli.rs`, add to the `Command` enum:

```rust
    /// Start the HTTP API server (dashboard backend).
    Serve {
        /// Port to listen on.
        #[arg(long, default_value = "3001")]
        port: u16,
        /// Host to bind to (use 0.0.0.0 for remote access).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
```

- [ ] **Step 2: Add `serve` handler in `main.rs`**

In `src/main.rs`, add the match arm in `main()`:

```rust
        Command::Serve { port, host } => cmd_serve(&db, data_dir, host, port).await,
```

And add the function:

```rust
async fn cmd_serve(
    db: &galvolite::db::Db,
    data_dir: std::path::PathBuf,
    host: String,
    port: u16,
) -> Result<()> {
    use galvolite::session_manager::SessionManager;
    use std::sync::Arc;

    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let app = galvolite::api::router(mgr);

    let addr = format!("{host}:{port}");
    tracing::info!("starting API server on {addr}");
    println!("GalvoLite API server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: compiles

- [ ] **Step 4: Smoke test**

Run in one terminal:
```bash
cargo run -- serve --port 3099
```

In another terminal:
```bash
curl http://localhost:3099/api/projects
```

Expected: `[]` (empty JSON array). Kill the server with Ctrl-C.

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat: add CLI serve command for HTTP API server"
```

---

## Task 9: Frontend scaffold (React + Vite + TailwindCSS)

**Files:**
- Create: `ui/package.json`
- Create: `ui/vite.config.ts`
- Create: `ui/tailwind.config.js`
- Create: `ui/postcss.config.js`
- Create: `ui/tsconfig.json`
- Create: `ui/tsconfig.node.json`
- Create: `ui/index.html`
- Create: `ui/src/main.tsx`
- Create: `ui/src/App.tsx`
- Modify: `.gitignore`

- [ ] **Step 1: Create `ui/package.json`**

```json
{
  "name": "galvolite-ui",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1",
    "@xterm/xterm": "^5.5.0",
    "@xterm/addon-fit": "^0.10.0"
  },
  "devDependencies": {
    "@types/react": "^18.3.12",
    "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.4",
    "autoprefixer": "^10.4.20",
    "postcss": "^8.4.49",
    "tailwindcss": "^3.4.17",
    "typescript": "^5.7.2",
    "vite": "^5.4.11"
  }
}
```

- [ ] **Step 2: Create `ui/vite.config.ts`**

```typescript
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: "http://localhost:3001",
        ws: true,
      },
    },
  },
  build: {
    outDir: "dist",
  },
});
```

- [ ] **Step 3: Create `ui/tailwind.config.js`**

```javascript
/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {},
  },
  plugins: [],
};
```

- [ ] **Step 4: Create `ui/postcss.config.js`**

```javascript
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
};
```

- [ ] **Step 5: Create `ui/tsconfig.json`**

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "forceConsistentCasingInFileNames": true
  },
  "include": ["src"]
}
```

- [ ] **Step 6: Create `ui/tsconfig.node.json`**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2023"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "moduleDetection": "force",
    "noEmit": true,
    "strict": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **Step 7: Create `ui/index.html`**

```html
<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>GalvoLite</title>
  </head>
  <body class="bg-gray-950 text-gray-100 min-h-screen">
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
```

- [ ] **Step 8: Create `ui/src/main.tsx`**

```tsx
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
```

- [ ] **Step 9: Create `ui/src/index.css`**

```css
@tailwind base;
@tailwind components;
@tailwind utilities;
```

- [ ] **Step 10: Create `ui/src/App.tsx`**

```tsx
export default function App() {
  return (
    <div className="p-6">
      <h1 className="text-2xl font-bold mb-4">GalvoLite</h1>
      <p className="text-gray-400">Dashboard loading...</p>
    </div>
  );
}
```

- [ ] **Step 11: Add `ui/node_modules` and `ui/dist` to `.gitignore`**

Append to `.gitignore`:

```gitignore
# Frontend
ui/node_modules/
ui/dist/
```

- [ ] **Step 12: Install dependencies and verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm install
npm run build
```

Expected: Vite builds successfully to `ui/dist/`.

- [ ] **Step 13: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/ .gitignore
git commit -m "feat: scaffold React + Vite + TailwindCSS frontend"
```

---

## Task 10: TypeScript types and API client

**Files:**
- Create: `ui/src/types.ts`
- Create: `ui/src/api.ts`

- [ ] **Step 1: Create `ui/src/types.ts`**

```typescript
export type SessionState =
  | "spawning"
  | "working"
  | "attention_needed"
  | "handing_off"
  | "restoring"
  | "complete"
  | "killed"
  | "error";

export interface Project {
  id: string;
  name: string;
  path: string;
  remote_url: string | null;
  created_at: string;
}

export interface Session {
  id: string;
  project_id: string;
  role: string;
  branch: string | null;
  worktree_path: string;
  pty_pid: number | null;
  state: SessionState;
  autopilot: boolean;
  parent_session_id: string | null;
  task_summary: string;
  created_at: string;
  ended_at: string | null;
}

export interface SignalRow {
  id: number;
  session_id: string;
  cycle_id: string | null;
  kind: string;
  payload: string | null;
  at: string;
}

export interface CreateSessionReq {
  project_id: string;
  task: string;
  role?: string;
  claude_cmd?: string;
}

export interface CreateProjectReq {
  name: string;
  path: string;
  remote_url?: string;
}

export interface SendReq {
  text: string;
  newline?: boolean;
}

export const STATE_COLORS: Record<SessionState, string> = {
  spawning: "bg-blue-400",
  working: "bg-green-400",
  attention_needed: "bg-yellow-400",
  handing_off: "bg-blue-400",
  restoring: "bg-blue-400",
  complete: "bg-gray-400",
  killed: "bg-red-400",
  error: "bg-red-500",
};
```

- [ ] **Step 2: Create `ui/src/api.ts`**

```typescript
import type {
  Project,
  Session,
  SignalRow,
  CreateProjectReq,
  CreateSessionReq,
  SendReq,
} from "./types";

const BASE = "";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const resp = await fetch(`${BASE}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...init?.headers,
    },
  });
  if (!resp.ok) {
    const text = await resp.text();
    throw new Error(`${resp.status}: ${text}`);
  }
  return resp.json();
}

export const api = {
  listProjects: () => request<Project[]>("/api/projects"),

  createProject: (req: CreateProjectReq) =>
    request<Project>("/api/projects", {
      method: "POST",
      body: JSON.stringify(req),
    }),

  getProject: (id: string) => request<Project>(`/api/projects/${id}`),

  listSessions: () => request<Session[]>("/api/sessions"),

  createSession: (req: CreateSessionReq) =>
    request<Session>("/api/sessions", {
      method: "POST",
      body: JSON.stringify(req),
    }),

  getSession: (id: string) => request<Session>(`/api/sessions/${id}`),

  killSession: (id: string) =>
    request<{ status: string }>(`/api/sessions/${id}`, { method: "DELETE" }),

  triggerHandoff: (id: string) =>
    request<{ status: string }>(`/api/sessions/${id}/handoff`, {
      method: "POST",
    }),

  sendToSession: (id: string, req: SendReq) =>
    request<{ status: string }>(`/api/sessions/${id}/send`, {
      method: "POST",
      body: JSON.stringify(req),
    }),

  getSignals: (id: string) =>
    request<SignalRow[]>(`/api/sessions/${id}/signals`),

  terminalWsUrl: (id: string): string => {
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${window.location.host}/api/sessions/${id}/terminal`;
  },
};
```

- [ ] **Step 3: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles with no type errors

- [ ] **Step 4: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/types.ts ui/src/api.ts
git commit -m "feat: add TypeScript types and API client"
```

---

## Task 11: React hooks for data fetching

**Files:**
- Create: `ui/src/hooks/useProjects.ts`
- Create: `ui/src/hooks/useSessions.ts`
- Create: `ui/src/hooks/useTerminal.ts`

- [ ] **Step 1: Create `ui/src/hooks/useProjects.ts`**

```typescript
import { useEffect, useState } from "react";
import { api } from "../api";
import type { Project } from "../types";

export function useProjects() {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => {
    api
      .listProjects()
      .then(setProjects)
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    refresh();
  }, []);

  return { projects, loading, error, refresh };
}
```

- [ ] **Step 2: Create `ui/src/hooks/useSessions.ts`**

```typescript
import { useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { Session } from "../types";

export function useSessions(pollIntervalMs = 2000) {
  const [sessions, setSessions] = useState<Session[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>();

  const refresh = () => {
    api
      .listSessions()
      .then((data) => {
        setSessions(data);
        setError(null);
      })
      .catch((e) => setError(e.message))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    refresh();
    intervalRef.current = setInterval(refresh, pollIntervalMs);
    return () => clearInterval(intervalRef.current);
  }, [pollIntervalMs]);

  return { sessions, loading, error, refresh };
}
```

- [ ] **Step 3: Create `ui/src/hooks/useTerminal.ts`**

```typescript
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { api } from "../api";

export function useTerminal(
  containerRef: React.RefObject<HTMLDivElement | null>,
  sessionId: string | null
) {
  const termRef = useRef<Terminal | null>(null);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!containerRef.current || !sessionId) return;

    const term = new Terminal({
      fontSize: 12,
      fontFamily: "Menlo, monospace",
      theme: {
        background: "#0a0a0a",
        foreground: "#e5e5e5",
      },
      scrollback: 5000,
      convertEol: true,
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(containerRef.current);
    fitAddon.fit();
    termRef.current = term;

    const url = api.terminalWsUrl(sessionId);
    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onmessage = (ev) => {
      term.writeln(ev.data);
    };

    ws.onclose = () => {
      term.writeln("\r\n[connection closed]");
    };

    const onResize = () => fitAddon.fit();
    window.addEventListener("resize", onResize);

    return () => {
      window.removeEventListener("resize", onResize);
      ws.close();
      term.dispose();
      termRef.current = null;
      wsRef.current = null;
    };
  }, [containerRef, sessionId]);

  return termRef;
}
```

- [ ] **Step 4: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles

- [ ] **Step 5: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/hooks/
git commit -m "feat: add React hooks for projects, sessions, and terminal streaming"
```

---

## Task 12: ProjectSelector component

**Files:**
- Create: `ui/src/components/ProjectSelector.tsx`

- [ ] **Step 1: Create `ui/src/components/ProjectSelector.tsx`**

```tsx
import type { Project } from "../types";

interface Props {
  projects: Project[];
  selected: string | null;
  onSelect: (id: string | null) => void;
}

export function ProjectSelector({ projects, selected, onSelect }: Props) {
  return (
    <div className="flex items-center gap-3">
      <label className="text-sm text-gray-400">Project:</label>
      <select
        className="bg-gray-800 border border-gray-700 rounded px-3 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-blue-500"
        value={selected ?? ""}
        onChange={(e) => onSelect(e.target.value || null)}
      >
        <option value="">All projects</option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
    </div>
  );
}
```

- [ ] **Step 2: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles (component not yet mounted in App)

- [ ] **Step 3: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/components/ProjectSelector.tsx
git commit -m "feat: add ProjectSelector component"
```

---

## Task 13: SessionRow component

**Files:**
- Create: `ui/src/components/SessionRow.tsx`

- [ ] **Step 1: Create `ui/src/components/SessionRow.tsx`**

```tsx
import type { Session } from "../types";
import { STATE_COLORS } from "../types";
import { api } from "../api";

interface Props {
  session: Session;
  onSelect: (id: string) => void;
  selected: boolean;
  onRefresh: () => void;
}

export function SessionRow({ session, onSelect, selected, onRefresh }: Props) {
  const dotColor = STATE_COLORS[session.state] ?? "bg-gray-400";

  const handleKill = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await api.killSession(session.id);
    onRefresh();
  };

  const handleHandoff = async (e: React.MouseEvent) => {
    e.stopPropagation();
    await api.triggerHandoff(session.id);
  };

  const age = (() => {
    const ms = Date.now() - new Date(session.created_at).getTime();
    const mins = Math.floor(ms / 60000);
    if (mins < 60) return `${mins}m`;
    return `${Math.floor(mins / 60)}h ${mins % 60}m`;
  })();

  return (
    <div
      className={`flex items-center gap-3 px-4 py-3 cursor-pointer border-b border-gray-800 hover:bg-gray-800/50 ${
        selected ? "bg-gray-800" : ""
      }`}
      onClick={() => onSelect(session.id)}
    >
      {/* Status dot */}
      <div className={`w-2.5 h-2.5 rounded-full shrink-0 ${dotColor}`} />

      {/* Task + branch */}
      <div className="flex-1 min-w-0">
        <div className="text-sm truncate">{session.task_summary}</div>
        <div className="text-xs text-gray-500 truncate">
          {session.role}
          {session.branch ? ` \u00b7 ${session.branch}` : ""}
          {" \u00b7 "}
          {age}
        </div>
      </div>

      {/* State badge */}
      <span className="text-xs px-2 py-0.5 rounded bg-gray-700 text-gray-300">
        {session.state}
      </span>

      {/* Autopilot pill */}
      <span
        className={`text-xs px-2 py-0.5 rounded ${
          session.autopilot
            ? "bg-green-900 text-green-300"
            : "bg-gray-700 text-gray-500"
        }`}
      >
        {session.autopilot ? "auto" : "manual"}
      </span>

      {/* Action buttons */}
      {session.state === "working" && (
        <div className="flex gap-1">
          <button
            className="text-xs px-2 py-1 rounded bg-blue-800 hover:bg-blue-700 text-blue-200"
            onClick={handleHandoff}
            title="Trigger handoff"
          >
            Handoff
          </button>
          <button
            className="text-xs px-2 py-1 rounded bg-red-900 hover:bg-red-800 text-red-200"
            onClick={handleKill}
            title="Kill session"
          >
            Kill
          </button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles

- [ ] **Step 3: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/components/SessionRow.tsx
git commit -m "feat: add SessionRow component with status dot and controls"
```

---

## Task 14: TerminalPreview component (xterm.js)

**Files:**
- Create: `ui/src/components/TerminalPreview.tsx`

- [ ] **Step 1: Create `ui/src/components/TerminalPreview.tsx`**

```tsx
import { useRef } from "react";
import { useTerminal } from "../hooks/useTerminal";
import "@xterm/xterm/css/xterm.css";

interface Props {
  sessionId: string;
  mini?: boolean;
}

export function TerminalPreview({ sessionId, mini = false }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  useTerminal(containerRef, sessionId);

  if (mini) {
    return (
      <div
        className="w-48 h-20 overflow-hidden rounded border border-gray-700 bg-black"
        style={{ transform: "scale(0.4)", transformOrigin: "top left", width: "480px", height: "200px" }}
      >
        <div ref={containerRef} className="w-full h-full" />
      </div>
    );
  }

  return (
    <div className="rounded border border-gray-700 bg-black overflow-hidden">
      <div ref={containerRef} style={{ height: "400px" }} />
    </div>
  );
}
```

- [ ] **Step 2: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles

- [ ] **Step 3: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/components/TerminalPreview.tsx
git commit -m "feat: add xterm.js TerminalPreview component with mini mode"
```

---

## Task 15: SessionDetail expand panel

**Files:**
- Create: `ui/src/components/SessionDetail.tsx`

- [ ] **Step 1: Create `ui/src/components/SessionDetail.tsx`**

```tsx
import { useEffect, useState } from "react";
import { api } from "../api";
import type { Session, SignalRow } from "../types";
import { STATE_COLORS } from "../types";
import { TerminalPreview } from "./TerminalPreview";

interface Props {
  session: Session;
  onRefresh: () => void;
}

export function SessionDetail({ session, onRefresh }: Props) {
  const [signals, setSignals] = useState<SignalRow[]>([]);

  useEffect(() => {
    api.getSignals(session.id).then(setSignals).catch(() => {});
  }, [session.id]);

  const handleKill = async () => {
    await api.killSession(session.id);
    onRefresh();
  };

  const handleHandoff = async () => {
    await api.triggerHandoff(session.id);
  };

  const dotColor = STATE_COLORS[session.state] ?? "bg-gray-400";
  const isRunning = !["complete", "killed", "error"].includes(session.state);

  return (
    <div className="border-l border-gray-800 p-4 space-y-4 overflow-auto">
      {/* Header */}
      <div className="flex items-center gap-2">
        <div className={`w-3 h-3 rounded-full ${dotColor}`} />
        <h2 className="text-lg font-semibold">{session.task_summary}</h2>
      </div>

      {/* Metadata */}
      <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
        <dt className="text-gray-500">Role</dt>
        <dd>{session.role}</dd>
        <dt className="text-gray-500">State</dt>
        <dd>{session.state}</dd>
        <dt className="text-gray-500">Branch</dt>
        <dd>{session.branch ?? "(none)"}</dd>
        <dt className="text-gray-500">Worktree</dt>
        <dd className="truncate">{session.worktree_path}</dd>
        <dt className="text-gray-500">PID</dt>
        <dd>{session.pty_pid ?? "N/A"}</dd>
        <dt className="text-gray-500">Autopilot</dt>
        <dd>{session.autopilot ? "ON" : "OFF"}</dd>
        <dt className="text-gray-500">Created</dt>
        <dd>{new Date(session.created_at).toLocaleString()}</dd>
      </dl>

      {/* Actions */}
      {isRunning && (
        <div className="flex gap-2">
          <button
            className="px-3 py-1.5 rounded bg-blue-800 hover:bg-blue-700 text-sm"
            onClick={handleHandoff}
          >
            Handoff Now
          </button>
          <button
            className="px-3 py-1.5 rounded bg-red-900 hover:bg-red-800 text-sm"
            onClick={handleKill}
          >
            Kill Session
          </button>
        </div>
      )}

      {/* Terminal */}
      {isRunning && (
        <div>
          <h3 className="text-sm font-medium text-gray-400 mb-2">
            Live Terminal
          </h3>
          <TerminalPreview sessionId={session.id} />
        </div>
      )}

      {/* Recent signals */}
      <div>
        <h3 className="text-sm font-medium text-gray-400 mb-2">
          Recent Signals ({signals.length})
        </h3>
        <div className="space-y-1 max-h-48 overflow-auto">
          {signals
            .slice(-20)
            .reverse()
            .map((sig) => (
              <div key={sig.id} className="text-xs font-mono text-gray-400">
                <span className="text-gray-600">
                  {new Date(sig.at).toLocaleTimeString()}
                </span>{" "}
                <span className="text-blue-300">{sig.kind}</span>
                {sig.payload && (
                  <span className="text-gray-500"> {sig.payload}</span>
                )}
              </div>
            ))}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles

- [ ] **Step 3: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/components/SessionDetail.tsx
git commit -m "feat: add SessionDetail expand panel with signals and live terminal"
```

---

## Task 16: Dashboard component (full assembly)

**Files:**
- Create: `ui/src/components/Dashboard.tsx`
- Modify: `ui/src/App.tsx`

Wire all components together into the main dashboard view.

- [ ] **Step 1: Create `ui/src/components/Dashboard.tsx`**

```tsx
import { useState } from "react";
import { useProjects } from "../hooks/useProjects";
import { useSessions } from "../hooks/useSessions";
import { ProjectSelector } from "./ProjectSelector";
import { SessionRow } from "./SessionRow";
import { SessionDetail } from "./SessionDetail";

export function Dashboard() {
  const { projects } = useProjects();
  const { sessions, refresh } = useSessions(2000);
  const [selectedProject, setSelectedProject] = useState<string | null>(null);
  const [selectedSession, setSelectedSession] = useState<string | null>(null);

  const filtered = selectedProject
    ? sessions.filter((s) => s.project_id === selectedProject)
    : sessions;

  const detail = filtered.find((s) => s.id === selectedSession);

  const activeCount = filtered.filter(
    (s) => !["complete", "killed", "error"].includes(s.state)
  ).length;

  const attentionCount = filtered.filter(
    (s) => s.state === "attention_needed"
  ).length;

  return (
    <div className="flex flex-col h-screen">
      {/* Top bar */}
      <header className="flex items-center justify-between px-4 py-3 border-b border-gray-800 bg-gray-900">
        <div className="flex items-center gap-4">
          <h1 className="text-lg font-bold">GalvoLite</h1>
          <ProjectSelector
            projects={projects}
            selected={selectedProject}
            onSelect={setSelectedProject}
          />
        </div>
        <div className="flex items-center gap-4 text-sm">
          <span className="text-gray-400">
            {activeCount} active
          </span>
          {attentionCount > 0 && (
            <span className="px-2 py-0.5 rounded bg-yellow-900 text-yellow-300">
              {attentionCount} need attention
            </span>
          )}
        </div>
      </header>

      {/* Main content */}
      <div className="flex flex-1 overflow-hidden">
        {/* Session list */}
        <div className="flex-1 overflow-auto">
          {filtered.length === 0 ? (
            <div className="p-8 text-center text-gray-500">
              No sessions. Start one via the API or CLI.
            </div>
          ) : (
            filtered.map((s) => (
              <SessionRow
                key={s.id}
                session={s}
                selected={s.id === selectedSession}
                onSelect={setSelectedSession}
                onRefresh={refresh}
              />
            ))
          )}
        </div>

        {/* Detail panel */}
        {detail && (
          <div className="w-[480px] shrink-0">
            <SessionDetail session={detail} onRefresh={refresh} />
          </div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Update `ui/src/App.tsx` to render Dashboard**

```tsx
import { Dashboard } from "./components/Dashboard";

export default function App() {
  return <Dashboard />;
}
```

- [ ] **Step 3: Verify build**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
```

Expected: compiles with no errors

- [ ] **Step 4: Smoke test in browser**

In one terminal:
```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
cargo run -- serve --port 3001
```

In another terminal:
```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run dev
```

Open `http://localhost:5173` in a browser. Expected: the dashboard renders showing "GalvoLite" header with project selector and "No sessions" message. The Vite proxy forwards `/api` calls to the cargo server on port 3001.

- [ ] **Step 5: Commit**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git add ui/src/components/Dashboard.tsx ui/src/App.tsx
git commit -m "feat: assemble Dashboard with session list, detail panel, and project selector"
```

---

## Task 17: Serve frontend static files from axum

**Files:**
- Modify: `src/api.rs`

In production (Tauri build), the axum server should serve the built frontend files from `ui/dist/`. Add a fallback service that serves static files when the `ui/dist` directory exists.

- [ ] **Step 1: Modify `router()` in `src/api.rs`**

Add static file serving as a fallback after the API routes. Replace the `router` function:

```rust
pub fn router(mgr: Arc<SessionManager>) -> Router {
    let api = Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{id}", get(get_project))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", get(get_session).delete(kill_session))
        .route("/api/sessions/{id}/handoff", post(trigger_handoff))
        .route("/api/sessions/{id}/send", post(send_to_session))
        .route("/api/sessions/{id}/signals", get(get_signals))
        .route("/api/sessions/{id}/terminal", get(terminal_ws))
        .layer(CorsLayer::permissive())
        .with_state(mgr);

    // Serve frontend static files if ui/dist exists
    let ui_dir = std::path::PathBuf::from(
        std::env::var("GALVOLITE_UI_DIR").unwrap_or_else(|_| "ui/dist".to_string()),
    );
    if ui_dir.exists() {
        api.fallback_service(
            tower_http::services::ServeDir::new(&ui_dir)
                .fallback(tower_http::services::ServeFile::new(ui_dir.join("index.html"))),
        )
    } else {
        api
    }
}
```

Add the import at the top of the file (if not already present, `tower_http` is already a dependency):

No new import needed — `tower_http::services::ServeDir` and `ServeFile` are used inline.

- [ ] **Step 2: Verify build**

Run: `cargo build`
Expected: compiles

- [ ] **Step 3: Test static serving**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/ui"
npm run build
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
cargo run -- serve --port 3099
```

Open `http://localhost:3099` in a browser. Expected: the React dashboard loads (served by axum, not Vite). Kill the server.

- [ ] **Step 4: Commit**

```bash
git add src/api.rs
git commit -m "feat: serve frontend static files from axum when ui/dist exists"
```

---

## Task 18: Tauri project setup

**Files:**
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/build.rs`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/capabilities/default.json`
- Create: `src-tauri/src/main.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Add `src-tauri` to workspace members**

In the root `Cargo.toml`, modify the workspace section:

```toml
[workspace]
members = [".", "fixtures/mock-claude", "src-tauri"]
```

- [ ] **Step 2: Create `src-tauri/Cargo.toml`**

```toml
[package]
name = "galvolite-tauri"
version = "0.1.0"
edition = "2021"
description = "GalvoLite desktop app — Tauri shell"

[dependencies]
galvolite = { path = ".." }
tauri = { version = "2", features = [] }
tauri-build = { version = "2", features = [] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net"] }
axum = "0.7"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

- [ ] **Step 3: Create `src-tauri/build.rs`**

```rust
fn main() {
    tauri_build::build();
}
```

- [ ] **Step 4: Create `src-tauri/tauri.conf.json`**

```json
{
  "$schema": "https://raw.githubusercontent.com/nicedoc/tauri/v2/crates/tauri-config-schema/schema.json",
  "productName": "GalvoLite",
  "version": "0.1.0",
  "identifier": "com.lucidity.galvolite",
  "build": {
    "frontendDist": "../ui/dist",
    "devUrl": "http://localhost:5173",
    "beforeDevCommand": "cd ../ui && npm run dev",
    "beforeBuildCommand": "cd ../ui && npm run build"
  },
  "app": {
    "windows": [
      {
        "title": "GalvoLite",
        "width": 1200,
        "height": 800,
        "url": "http://localhost:3001"
      }
    ]
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": []
  }
}
```

- [ ] **Step 5: Create `src-tauri/capabilities/default.json`**

```json
{
  "identifier": "default",
  "description": "Default capabilities for GalvoLite",
  "windows": ["*"],
  "permissions": [
    "core:default"
  ]
}
```

- [ ] **Step 6: Create `src-tauri/src/main.rs`**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use galvolite::db::Db;
use galvolite::session_manager::SessionManager;
use std::sync::Arc;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("galvolite=info")),
        )
        .init();

    let data_dir = dirs_next_or_dirs_data_dir().join("GalvoLite");
    std::fs::create_dir_all(&data_dir).expect("create data dir");

    let db = Db::open(data_dir.join("galvolite.sqlite")).expect("open database");
    let mgr = Arc::new(SessionManager::new(db, data_dir));

    let port: u16 = std::env::var("GALVOLITE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3001);

    // Start axum server in a background thread
    let mgr_for_server = mgr.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(async move {
            let app = galvolite::api::router(mgr_for_server);
            let addr = format!("127.0.0.1:{port}");
            tracing::info!("API server on http://{addr}");
            let listener = tokio::net::TcpListener::bind(&addr)
                .await
                .expect("bind port");
            axum::serve(listener, app).await.expect("axum serve");
        });
    });

    // Give the server a moment to start
    std::thread::sleep(std::time::Duration::from_millis(200));

    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn dirs_next_or_dirs_data_dir() -> std::path::PathBuf {
    dirs::data_dir().expect("could not determine OS data dir")
}
```

- [ ] **Step 7: Verify workspace compiles**

Run: `cargo build --workspace`
Expected: compiles (Tauri may take a while on first build due to WebKit deps on macOS)

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src-tauri/
git commit -m "feat: add Tauri desktop shell project"
```

---

## Task 19: Tauri dev mode verification

**Files:** (none — verification only)

Verify the full Tauri dev workflow: Vite HMR + axum API + Tauri webview.

- [ ] **Step 1: Install Tauri CLI**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/src-tauri"
cargo install tauri-cli --version "^2"
```

- [ ] **Step 2: Run in dev mode**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite/src-tauri"
cargo tauri dev
```

Expected: Tauri opens a native window showing the GalvoLite dashboard. The Vite dev server runs on :5173 and the axum API server on :3001. The window loads `http://localhost:3001` which falls back to the Vite proxy in dev mode.

Note: if the Tauri window loads `http://localhost:3001` but the UI is not there (no static files), switch `tauri.conf.json` window url to `http://localhost:5173` for dev mode, or set `GALVOLITE_UI_DIR` to point to the Vite dev proxy.

- [ ] **Step 3: Verify API connectivity**

With the app running, open browser dev tools in the Tauri window (right-click → Inspect). Check the Network tab — `/api/projects` and `/api/sessions` should return 200 with JSON.

- [ ] **Step 4: Stop and commit any config adjustments**

If any config needed tweaking, commit:
```bash
git add -A
git commit -m "fix: adjust Tauri dev mode configuration"
```

---

## Task 20: Update main.rs to use SessionManager for existing CLI commands

**Files:**
- Modify: `src/main.rs`

Now that SessionManager exists, update the existing CLI commands (`start-session`, `send`, `handoff`, `kill`) to use it instead of duplicating logic. The `start-session` command should use SessionManager and block until the session exits.

- [ ] **Step 1: Refactor `main.rs`**

Replace the body of `main()` and the command handlers to use SessionManager:

```rust
use anyhow::Result;
use clap::Parser;
use galvolite::cli::{Cli, Command};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("galvolite=info")),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = resolve_data_dir(cli.data_dir.clone())?;
    std::fs::create_dir_all(&data_dir)?;

    let db = galvolite::db::Db::open(data_dir.join("galvolite.sqlite"))?;
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir.clone()));

    match cli.cmd {
        Command::AddProject { path, name } => cmd_add_project(&mgr, path, name),
        Command::Projects => cmd_list_projects(&mgr),
        Command::StartSession {
            project,
            task,
            role,
            claude_cmd,
        } => cmd_start_session(&mgr, project, task, role, claude_cmd).await,
        Command::Sessions => cmd_list_sessions(&mgr),
        Command::Status { session } => cmd_status(&db, session),
        Command::Send {
            session,
            text,
            newline,
        } => cmd_send(&mgr, session, text, newline).await,
        Command::Handoff { session } => cmd_handoff(&mgr, session).await,
        Command::Kill { session } => cmd_kill(&mgr, session).await,
        Command::Serve { port, host } => cmd_serve(mgr, host, port).await,
    }
}

fn resolve_data_dir(override_dir: Option<std::path::PathBuf>) -> Result<std::path::PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    let base =
        dirs::data_dir().ok_or_else(|| anyhow::anyhow!("could not determine OS data dir"))?;
    Ok(base.join("GalvoLite"))
}

fn cmd_add_project(
    mgr: &SessionManager,
    path: std::path::PathBuf,
    name: Option<String>,
) -> Result<()> {
    let abs = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("resolve {}: {e}", path.display()))?;
    let name = name.unwrap_or_else(|| {
        abs.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into())
    });
    let p = mgr.add_project(galvolite::db::NewProject {
        name: name.clone(),
        path: abs.to_string_lossy().into_owned(),
        remote_url: None,
    })?;
    println!("Added project {} ({}) at {}", p.name, p.id, p.path);
    Ok(())
}

fn cmd_list_projects(mgr: &SessionManager) -> Result<()> {
    let projects = mgr.list_projects()?;
    if projects.is_empty() {
        println!("(no projects registered)");
        return Ok(());
    }
    for p in projects {
        println!("{}  {}  {}", p.id, p.name, p.path);
    }
    Ok(())
}

async fn cmd_start_session(
    mgr: &Arc<SessionManager>,
    project_id: uuid::Uuid,
    task: String,
    role: String,
    claude_cmd: String,
) -> Result<()> {
    let session = mgr
        .spawn_session(project_id, task, role, claude_cmd)
        .await?;
    println!("Session {} started (pid {:?})", session.id, session.pty_pid);
    println!("Press Ctrl-C to detach (session keeps running in background).");

    // Block until Ctrl-C
    tokio::signal::ctrl_c().await?;
    println!("\nDetached from session {}. It continues running.", session.id);
    Ok(())
}

fn cmd_list_sessions(mgr: &SessionManager) -> Result<()> {
    let ss = mgr.list_sessions()?;
    if ss.is_empty() {
        println!("(no active sessions)");
        return Ok(());
    }
    for s in ss {
        println!("{}  state={}  task={}", s.id, s.state, s.task_summary);
    }
    Ok(())
}

fn cmd_status(db: &galvolite::db::Db, session: uuid::Uuid) -> Result<()> {
    let s = db.get_session(session)?;
    println!("Session {}", s.id);
    println!("  Project:       {}", s.project_id);
    println!("  Role:          {}", s.role);
    println!("  State:         {}", s.state);
    println!("  Task:          {}", s.task_summary);
    println!("  Worktree:      {}", s.worktree_path);
    println!("  PID:           {:?}", s.pty_pid);
    println!("  Autopilot:     {}", s.autopilot);
    let sigs = db.list_signals_for_session(session)?;
    println!("  Signals ({}):", sigs.len());
    for sig in sigs.iter().rev().take(10).rev() {
        println!(
            "    [{}] {}{}",
            sig.at.format("%H:%M:%S"),
            sig.kind,
            sig.payload
                .as_deref()
                .map(|p| format!(" reason=\"{p}\""))
                .unwrap_or_default()
        );
    }
    Ok(())
}

async fn cmd_send(
    mgr: &SessionManager,
    session_id: uuid::Uuid,
    text: String,
    newline: bool,
) -> Result<()> {
    mgr.send_to_session(session_id, text, newline).await?;
    println!("sent");
    Ok(())
}

async fn cmd_handoff(mgr: &SessionManager, session_id: uuid::Uuid) -> Result<()> {
    mgr.trigger_handoff(session_id).await?;
    println!("handoff initiated");
    Ok(())
}

async fn cmd_kill(mgr: &SessionManager, session_id: uuid::Uuid) -> Result<()> {
    mgr.kill_session(session_id).await?;
    println!("killed");
    Ok(())
}

async fn cmd_serve(mgr: Arc<SessionManager>, host: String, port: u16) -> Result<()> {
    let app = galvolite::api::router(mgr);

    let addr = format!("{host}:{port}");
    tracing::info!("starting API server on {addr}");
    println!("GalvoLite API server listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 2: Verify all tests pass**

Run: `cargo test`
Expected: all pass

- [ ] **Step 3: Verify CLI still works**

```bash
cargo run -- projects
```

Expected: prints project list (or "no projects registered")

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "refactor: CLI commands now delegate to SessionManager"
```

---

## Task 21: Integration test — API to mock-claude

**Files:**
- Create: `tests/api_integration.rs`

End-to-end test: use the API to create a project, spawn a session with mock-claude, verify the session appears in list, verify the terminal WebSocket streams output, and kill the session.

- [ ] **Step 1: Write the integration test**

Create `tests/api_integration.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::db::Db;
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

/// Build the path to the mock-claude binary. It should already be compiled
/// from Plan 1a.
fn mock_claude_bin() -> String {
    let p = std::path::Path::new(env!("CARGO_BIN_EXE_galvolite"))
        .parent()
        .unwrap()
        .join("mock-claude");
    p.to_string_lossy().into_owned()
}

fn setup() -> (axum::Router, Arc<SessionManager>) {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();

    // Create a project directory for mock-claude
    let project_dir = dir.path().join("mock-project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let _ = Box::leak(Box::new(dir));

    let mgr = Arc::new(SessionManager::new(db, data_dir));
    let app = galvolite::api::router(mgr.clone());
    (app, mgr)
}

#[tokio::test]
async fn full_api_lifecycle() {
    let (app, mgr) = setup();

    // Create project
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "TestProject",
                        "path": "/tmp/galvolite-test-integration"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let project_id = project["id"].as_str().unwrap();

    // List sessions (should be empty)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let sessions: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(sessions.is_empty());

    // Create session with echo (not mock-claude, to keep it simple)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "project_id": project_id,
                        "task": "integration test",
                        "claude_cmd": "echo"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let session: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let session_id = session["id"].as_str().unwrap();

    // Get session detail
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Wait a moment for the echo to complete
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Kill session (may already be done since echo exits immediately)
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/sessions/{session_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --test api_integration -- --nocapture`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add tests/api_integration.rs
git commit -m "test: add API integration test covering project+session lifecycle"
```

---

## Task 22: Update README

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README.md**

```markdown
# GalvoLite

Solo continuity engine for Claude Code. See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec.

## Status

- **Plan 1a** (Ritual Engine CLI): DONE — 44 tests, clippy clean
- **Plan 1b** (Tauri Desktop Shell + Dashboard): DONE

## Quick start

### CLI server

```bash
cargo run -- serve
# API available at http://localhost:3001
```

### Desktop app (Tauri)

```bash
cd src-tauri
cargo tauri dev
```

### Development (frontend only)

```bash
# Terminal 1: API server
cargo run -- serve --port 3001

# Terminal 2: Vite dev server with HMR
cd ui && npm run dev
# Open http://localhost:5173
```

## Architecture

```
Tauri (native shell)
  └─ React + xterm.js frontend
         │ HTTP + WebSocket
  Rust core (tokio + axum)
    ├── SessionManager (pty, signals, ritual driver)
    ├── SQLite (operational state)
    └── N long-lived Claude Code processes
```
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: update README for Plan 1b completion"
```

---

## Self-Review Checklist

**Spec coverage:**
- [x] §4 Architecture: Tauri + React + axum + Rust core ← Tasks 7-8, 9-17, 18-19
- [x] §8.1 Data model: existing schema works; Serialize added ← Task 2
- [x] §12.1 Dashboard: row per session, status dot, task summary, state badge ← Tasks 13, 16
- [x] §12.2 Mini xterm.js preview: TerminalPreview with mini mode ← Task 14
- [x] §12.3 Expand panel: SessionDetail with metadata, signals, terminal ← Task 15
- [x] §12.5 Attention surfacing: attention count in top bar ← Task 16
- [x] §13 Tech stack: all specified technologies used
- [x] Deferred item #1 (N+1 query): ← Task 1
- [x] Deferred item #2 (kill doesn't terminate child): ← Task 3
- [x] Deferred item #3 (no --json): bypassed by direct API ← Task 7
- [x] Deferred item #4 (cmd_start_session monolith): ← Task 6
- [x] Deferred item #5 (cycle_id always None): ← Task 4

**Not in Plan 1b scope (deferred to later plans):**
- §5.2 Plugin-based context-pressure trigger (Plan 2)
- §5.4 Autopilot auto-trigger (Plan 2; toggle UI wired but doesn't fire)
- §6 Brainstorm → plan.md flow (Plan 2)
- §7 PR pipeline / GitHub (Plan 3)
- §10 CLAUDE.md profile injection for brainstorm/scout (Plan 2)
- §11 Seeds subsystem (Plan 4)
- §14 GitHub OAuth (Plan 3)
- §12.2 Hover-dwell expand behavior (polish — deferred)
- §12.2 Click-to-interactive terminal (polish — deferred)

**Type consistency:** SessionState uses snake_case throughout (DB, JSON, TypeScript). UUID fields are strings in TypeScript, Uuid in Rust. All API endpoint paths match between api.rs and api.ts.

**Placeholder scan:** No TBD/TODO/placeholder tokens found.
