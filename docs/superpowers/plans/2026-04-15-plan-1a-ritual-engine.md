# Plan 1a — Ritual Engine (CLI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust CLI that can spawn a Claude Code process, parse signals from its output, and drive the save→/clear→restore ritual end-to-end — proving the highest-risk assumption of the GalvoLite product before any UI work.

**Architecture:** Single Rust binary. One long-lived pty per session. Per-session blocking threads (reader + writer) bridge to async tokio via mpsc channels. SQLite stores projects, sessions, cycles, and signal history. Ritual driver is a state machine reacting to signals + user-triggered events. End-to-end testable against a mock-claude binary fixture that emits scripted signals.

**Tech Stack:** Rust 2021, tokio, portable-pty, rusqlite (with r2d2 pool), clap v4, tracing, thiserror, anyhow, uuid, serde, tempfile (test).

**Scope boundary:**
- **IN:** Git repo init, Rust project, SQLite schema, signal parser, pty wrapper, CLAUDE.md injection, handoff file I/O, ritual state machine, CLI (add-project/start-session/send/handoff/status/kill), mock-claude fixture, integration test.
- **OUT:** Tauri (plan 1b), UI (1b), plugin (plan 2), autopilot (plan 2), brainstorm/plan.md (plan 3), multi-worktree spawn (plan 3), GitHub (plan 4), Copilot (plan 5), seeds (plan 6), remote auth (plan 8).

**Reference:** [GalvoLite design spec](../specs/2026-04-15-galvolite-design.md) — especially §5 (continuity loop), §8 (data model), §10.2 (Implementation profile).

---

## File Structure

Target layout at end of plan:

```
GalvoLite/
├── .git/                                   # task 1
├── .gitignore                              # task 1
├── Cargo.toml                              # task 2
├── Cargo.lock
├── README.md                               # task 1 (minimal stub)
├── docs/superpowers/
│   ├── specs/2026-04-15-galvolite-design.md  (already exists)
│   └── plans/2026-04-15-plan-1a-ritual-engine.md  (this file)
├── files/                                  (existing reference PRDs — leave in place)
├── migrations/
│   └── 0001_initial.sql                    # task 5
├── src/
│   ├── main.rs                             # task 3, filled in tasks 17-21
│   ├── lib.rs                              # task 3
│   ├── error.rs                            # task 4
│   ├── db.rs                               # tasks 5-7
│   ├── signals.rs                          # tasks 8-9
│   ├── claude_md.rs                        # task 10
│   ├── pty.rs                              # tasks 11-12
│   ├── handoff.rs                          # task 13
│   ├── ritual.rs                           # tasks 14-16
│   └── cli.rs                              # task 17
├── tests/
│   ├── signals.rs                          # task 8 (parser tests)
│   ├── db.rs                               # task 7
│   ├── handoff.rs                          # task 13
│   └── ritual_end_to_end.rs                # task 23
└── fixtures/
    └── mock-claude/                        # task 22
        ├── Cargo.toml
        └── src/main.rs
```

Responsibilities:
- `signals.rs` — pure parsing, no I/O. Easiest to TDD first.
- `db.rs` — schema migration + typed CRUD. Sync rusqlite wrapped in `tokio::task::spawn_blocking` from async callers.
- `pty.rs` — spawn child process under a pty; reader/writer bridge threads; emit line-buffered output.
- `claude_md.rs` — profile template + inject-before-first-prompt helper.
- `handoff.rs` — path resolution, file read/write, validation ("handoff file exists and contains Continuation Prompt section").
- `ritual.rs` — state machine + orchestrator. Subscribes to parsed signals from a session's pty, reacts.
- `cli.rs` — clap command definitions, dispatchers.
- `main.rs` — bootstrap: init tracing, open DB, dispatch CLI command.

---

## Task 1: Initialize git repo and workspace scaffolding

**Files:**
- Create: `.gitignore`
- Create: `README.md`

- [ ] **Step 1: Initialize git repo in-place**

```bash
cd "/Volumes/Main External/Development/lucidity/GalvoLite"
git init
git branch -M main
```

Expected: `Initialized empty Git repository in .../GalvoLite/.git/`

- [ ] **Step 2: Create `.gitignore`**

Contents:

```gitignore
# Rust
/target
Cargo.lock.bak
**/*.rs.bk

# macOS
.DS_Store

# Environment
.env
.env.local

# GalvoLite runtime data (not source)
.galvolite/worktrees/
.galvolite/handoffs/
# (we commit .galvolite/plan.md and .galvolite/seeds/ selectively; those do not apply in this repo)

# IDE
.idea/
.vscode/
*.swp
```

- [ ] **Step 3: Create minimal `README.md`**

```markdown
# GalvoLite

Solo continuity engine for Claude Code. See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec.

Plan 1a (this branch) delivers the ritual engine as a CLI. Tauri + dashboard come in plan 1b.

## Status

Plan 1a in progress.
```

- [ ] **Step 4: Commit**

```bash
git add .gitignore README.md docs/ files/
git commit -m "chore: initialize repo with spec, plan 1a, and ignore rules"
```

Expected: a single commit on `main`. Verify with `git log --oneline`.

---

## Task 2: Create Cargo project and declare dependencies

**Files:**
- Create: `Cargo.toml`

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "galvolite"
version = "0.1.0"
edition = "2021"
description = "Solo continuity engine for Claude Code — ritual engine CLI"
license = "MIT OR Apache-2.0"

[dependencies]
anyhow = "1.0"
thiserror = "1.0"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "fs", "io-util", "signal"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
rusqlite = { version = "0.31", features = ["bundled", "uuid", "chrono"] }
r2d2 = "0.8"
r2d2_sqlite = "0.24"
portable-pty = "0.8"
chrono = { version = "0.4", features = ["serde"] }
dirs = "5"
once_cell = "1"

[dev-dependencies]
tempfile = "3"
assert_cmd = "2"
predicates = "3"

[[bin]]
name = "galvolite"
path = "src/main.rs"

[lib]
name = "galvolite"
path = "src/lib.rs"
```

- [ ] **Step 2: Verify it compiles (with empty src)**

```bash
mkdir -p src
echo "fn main() {}" > src/main.rs
echo "" > src/lib.rs
cargo check
```

Expected: compiles successfully, downloads all deps. May take 1-2 minutes on first run.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock src/
git commit -m "chore: cargo project skeleton with ritual-engine deps"
```

---

## Task 3: Module skeleton

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`
- Create: `src/error.rs`, `src/db.rs`, `src/signals.rs`, `src/claude_md.rs`, `src/pty.rs`, `src/handoff.rs`, `src/ritual.rs`, `src/cli.rs`

- [ ] **Step 1: Write `src/lib.rs`**

```rust
//! GalvoLite ritual engine — library crate.
//!
//! See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec
//! and `docs/superpowers/plans/2026-04-15-plan-1a-ritual-engine.md` for this plan.

pub mod claude_md;
pub mod db;
pub mod error;
pub mod handoff;
pub mod pty;
pub mod ritual;
pub mod signals;

pub use error::{Error, Result};
```

- [ ] **Step 2: Write empty module stubs**

`src/error.rs`:
```rust
//! Library error type. Populated in task 4.
```

`src/db.rs`:
```rust
//! SQLite schema + CRUD. Populated in tasks 5-7.
```

`src/signals.rs`:
```rust
//! Signal protocol parsing. Populated in tasks 8-9.
```

`src/claude_md.rs`:
```rust
//! CLAUDE.md profile injection. Populated in task 10.
```

`src/pty.rs`:
```rust
//! Pty wrapper + bridge threads. Populated in tasks 11-12.
```

`src/handoff.rs`:
```rust
//! Handoff file format + I/O. Populated in task 13.
```

`src/ritual.rs`:
```rust
//! Ritual state machine + driver. Populated in tasks 14-16.
```

`src/cli.rs`:
```rust
//! Clap command definitions. Populated in task 17.
```

- [ ] **Step 3: Write `src/main.rs`**

```rust
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("galvolite=info")),
        )
        .init();

    tracing::info!("galvolite starting (stub — CLI wired in task 17)");
    Ok(())
}
```

- [ ] **Step 4: Build and run**

```bash
cargo build
./target/debug/galvolite
```

Expected: one log line `galvolite starting (stub...)`.

- [ ] **Step 5: Commit**

```bash
git add src/
git commit -m "chore: module skeleton"
```

---

## Task 4: Error type

**Files:**
- Modify: `src/error.rs`

- [ ] **Step 1: Write failing test**

Create `tests/error.rs`:

```rust
use galvolite::error::Error;

#[test]
fn error_display_contains_context() {
    let err = Error::SignalParse("bad line".to_string());
    let s = format!("{err}");
    assert!(s.contains("signal parse"), "unexpected display: {s}");
    assert!(s.contains("bad line"), "unexpected display: {s}");
}
```

- [ ] **Step 2: Run test — expect compile failure**

```bash
cargo test --test error
```

Expected: `error[E0432]: unresolved import 'galvolite::error::Error'` or similar — `Error` does not yet exist.

- [ ] **Step 3: Implement error type**

`src/error.rs`:
```rust
use std::path::PathBuf;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("signal parse failed: {0}")]
    SignalParse(String),

    #[error("pty error: {0}")]
    Pty(String),

    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("db pool error: {0}")]
    DbPool(#[from] r2d2::Error),

    #[error("io: {path} — {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("handoff invalid: {0}")]
    HandoffInvalid(String),

    #[error("session not found: {0}")]
    SessionNotFound(uuid::Uuid),

    #[error("ritual state error: {0}")]
    Ritual(String),

    #[error("config: {0}")]
    Config(String),
}

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }
}
```

- [ ] **Step 4: Run test — expect pass**

```bash
cargo test --test error
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/error.rs tests/error.rs
git commit -m "feat(error): typed library error"
```

---

## Task 5: SQLite schema migration

**Files:**
- Create: `migrations/0001_initial.sql`

- [ ] **Step 1: Write schema**

```sql
-- GalvoLite ritual-engine schema v0001
-- All IDs are TEXT (UUID v4). Timestamps are TEXT (RFC3339).

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    remote_url TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT NOT NULL PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('implementation','brainstorm','scout','fix')),
    branch TEXT,
    worktree_path TEXT NOT NULL,
    pty_pid INTEGER,
    state TEXT NOT NULL,
    autopilot INTEGER NOT NULL DEFAULT 0,
    parent_session_id TEXT REFERENCES sessions(id),
    task_summary TEXT NOT NULL,
    created_at TEXT NOT NULL,
    ended_at TEXT
);

CREATE TABLE IF NOT EXISTS cycles (
    id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    handoff_path TEXT,
    UNIQUE(session_id, ordinal)
);

CREATE TABLE IF NOT EXISTS signals (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    cycle_id TEXT REFERENCES cycles(id),
    kind TEXT NOT NULL,
    payload TEXT,
    at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_project_id ON sessions(project_id);
CREATE INDEX IF NOT EXISTS idx_cycles_session_id ON cycles(session_id);
CREATE INDEX IF NOT EXISTS idx_signals_session_id ON signals(session_id);
CREATE INDEX IF NOT EXISTS idx_signals_at ON signals(at);
```

- [ ] **Step 2: Commit**

```bash
git add migrations/0001_initial.sql
git commit -m "feat(db): initial schema"
```

---

## Task 6: Database connection pool + migration runner

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Write failing test**

Create `tests/db_init.rs`:

```rust
use galvolite::db::Db;
use tempfile::TempDir;

#[test]
fn new_db_applies_schema() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");

    let db = Db::open(&db_path).expect("open db");

    let conn = db.pool().get().unwrap();
    let version: i64 = conn
        .query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("query schema_version");
    assert_eq!(version, 1);
}

#[test]
fn reopening_db_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");

    let _ = galvolite::db::Db::open(&db_path).unwrap();
    let _ = galvolite::db::Db::open(&db_path).unwrap();
    // no panic = pass
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test db_init
```

Expected: compile error — `Db::open` not found.

- [ ] **Step 3: Implement `Db::open`**

`src/db.rs`:
```rust
use crate::error::{Error, Result};
use chrono::Utc;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use std::path::Path;

const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");

#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

impl Db {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error::io(parent, e))?;
            }
        }

        let manager = SqliteConnectionManager::file(path.as_ref())
            .with_init(|c| {
                c.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            });
        let pool = Pool::builder().max_size(8).build(manager)?;

        let db = Db { pool };
        db.migrate()?;
        Ok(db)
    }

    pub fn pool(&self) -> &Pool<SqliteConnectionManager> {
        &self.pool
    }

    fn migrate(&self) -> Result<()> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;

        tx.execute_batch(SCHEMA_V1)?;

        let already_applied: Option<i64> = tx
            .query_row(
                "SELECT version FROM schema_version WHERE version = 1",
                [],
                |r| r.get(0),
            )
            .ok();

        if already_applied.is_none() {
            tx.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                params![1i64, Utc::now().to_rfc3339()],
            )?;
        }

        tx.commit()?;
        Ok(())
    }
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test db_init
```

Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/db_init.rs
git commit -m "feat(db): pooled sqlite with idempotent migration"
```

---

## Task 7: Typed CRUD for projects, sessions, cycles, signals

**Files:**
- Modify: `src/db.rs`

- [ ] **Step 1: Write failing tests**

Add to `tests/db_init.rs` (or create `tests/db_crud.rs`):

```rust
use chrono::Utc;
use galvolite::db::{Db, NewProject, NewSession, Project, Session, SessionState};
use tempfile::TempDir;
use uuid::Uuid;

fn open_db() -> (TempDir, Db) {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    (tmp, db)
}

#[test]
fn insert_and_fetch_project() {
    let (_tmp, db) = open_db();
    let p = db.insert_project(NewProject {
        name: "Galvo".into(),
        path: "/tmp/galvo".into(),
        remote_url: None,
    }).unwrap();

    let fetched: Project = db.get_project(p.id).unwrap();
    assert_eq!(fetched.name, "Galvo");
    assert_eq!(fetched.path, "/tmp/galvo");
}

#[test]
fn insert_session_starts_in_spawning() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();

    let s = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: Some("main".into()),
        worktree_path: "/tmp/p".into(),
        task_summary: "test".into(),
        parent_session_id: None,
    }).unwrap();

    assert_eq!(s.state, SessionState::Spawning);
    let fetched = db.get_session(s.id).unwrap();
    assert_eq!(fetched.task_summary, "test");
}

#[test]
fn update_session_state_persists() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();
    let s = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: "/tmp/p".into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    db.update_session_state(s.id, SessionState::Working).unwrap();
    let fetched: Session = db.get_session(s.id).unwrap();
    assert_eq!(fetched.state, SessionState::Working);
}

#[test]
fn record_signal_and_list() {
    let (_tmp, db) = open_db();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: "/tmp/p".into(), remote_url: None,
    }).unwrap();
    let s = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: "/tmp/p".into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    db.record_signal(s.id, None, "STATUS:WORKING", None).unwrap();
    db.record_signal(s.id, None, "HANDOFF:READY", None).unwrap();

    let signals = db.list_signals_for_session(s.id).unwrap();
    assert_eq!(signals.len(), 2);
    assert_eq!(signals[0].kind, "STATUS:WORKING");
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test db_crud
```

Expected: all missing types/methods.

- [ ] **Step 3: Implement CRUD**

Append to `src/db.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

// ---------- types ----------

#[derive(Debug, Clone)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub remote_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewProject {
    pub name: String,
    pub path: String,
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Spawning => "spawning",
            Self::Working => "working",
            Self::AttentionNeeded => "attention_needed",
            Self::HandingOff => "handing_off",
            Self::Restoring => "restoring",
            Self::Complete => "complete",
            Self::Killed => "killed",
            Self::Error => "error",
        };
        f.write_str(s)
    }
}

impl FromStr for SessionState {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "spawning" => Self::Spawning,
            "working" => Self::Working,
            "attention_needed" => Self::AttentionNeeded,
            "handing_off" => Self::HandingOff,
            "restoring" => Self::Restoring,
            "complete" => Self::Complete,
            "killed" => Self::Killed,
            "error" => Self::Error,
            other => return Err(Error::Config(format!("unknown session state: {other}"))),
        })
    }
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct NewSession {
    pub project_id: Uuid,
    pub role: String,
    pub branch: Option<String>,
    pub worktree_path: String,
    pub task_summary: String,
    pub parent_session_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct SignalRow {
    pub id: i64,
    pub session_id: Uuid,
    pub cycle_id: Option<Uuid>,
    pub kind: String,
    pub payload: Option<String>,
    pub at: DateTime<Utc>,
}

// ---------- impl ----------

impl Db {
    pub fn insert_project(&self, new: NewProject) -> Result<Project> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO projects (id, name, path, remote_url, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id.to_string(),
                new.name,
                new.path,
                new.remote_url,
                now.to_rfc3339()
            ],
        )?;
        Ok(Project {
            id,
            name: new.name,
            path: new.path,
            remote_url: new.remote_url,
            created_at: now,
        })
    }

    pub fn get_project(&self, id: Uuid) -> Result<Project> {
        let conn = self.pool.get()?;
        let row = conn.query_row(
            "SELECT id, name, path, remote_url, created_at
             FROM projects WHERE id = ?1",
            params![id.to_string()],
            |r| {
                Ok(Project {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                    name: r.get(1)?,
                    path: r.get(2)?,
                    remote_url: r.get(3)?,
                    created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(4)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            },
        )?;
        Ok(row)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, path, remote_url, created_at FROM projects ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Project {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                    name: r.get(1)?,
                    path: r.get(2)?,
                    remote_url: r.get(3)?,
                    created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(4)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_session(&self, new: NewSession) -> Result<Session> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let state = SessionState::Spawning;
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO sessions
             (id, project_id, role, branch, worktree_path, pty_pid, state, autopilot,
              parent_session_id, task_summary, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                id.to_string(),
                new.project_id.to_string(),
                new.role,
                new.branch,
                new.worktree_path,
                Option::<i64>::None,
                state.to_string(),
                0i64,
                new.parent_session_id.map(|u| u.to_string()),
                new.task_summary,
                now.to_rfc3339(),
            ],
        )?;

        Ok(Session {
            id,
            project_id: new.project_id,
            role: new.role,
            branch: new.branch,
            worktree_path: new.worktree_path,
            pty_pid: None,
            state,
            autopilot: false,
            parent_session_id: new.parent_session_id,
            task_summary: new.task_summary,
            created_at: now,
            ended_at: None,
        })
    }

    pub fn get_session(&self, id: Uuid) -> Result<Session> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, project_id, role, branch, worktree_path, pty_pid, state,
                    autopilot, parent_session_id, task_summary, created_at, ended_at
             FROM sessions WHERE id = ?1",
            params![id.to_string()],
            |r| {
                Ok(Session {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
                    project_id: Uuid::parse_str(&r.get::<_, String>(1)?).unwrap(),
                    role: r.get(2)?,
                    branch: r.get(3)?,
                    worktree_path: r.get(4)?,
                    pty_pid: r.get(5)?,
                    state: SessionState::from_str(&r.get::<_, String>(6)?).unwrap(),
                    autopilot: r.get::<_, i64>(7)? != 0,
                    parent_session_id: r
                        .get::<_, Option<String>>(8)?
                        .map(|s| Uuid::parse_str(&s).unwrap()),
                    task_summary: r.get(9)?,
                    created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(10)?)
                        .unwrap()
                        .with_timezone(&Utc),
                    ended_at: r
                        .get::<_, Option<String>>(11)?
                        .map(|s| DateTime::parse_from_rfc3339(&s).unwrap().with_timezone(&Utc)),
                })
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Error::SessionNotFound(id),
            other => Error::Db(other),
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM sessions WHERE ended_at IS NULL ORDER BY created_at ASC",
        )?;
        let ids = stmt
            .query_map([], |r| {
                Ok(Uuid::parse_str(&r.get::<_, String>(0)?).unwrap())
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.get_session(id)).collect()
    }

    pub fn update_session_state(&self, id: Uuid, state: SessionState) -> Result<()> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET state = ?1 WHERE id = ?2",
            params![state.to_string(), id.to_string()],
        )?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }

    pub fn update_session_pid(&self, id: Uuid, pid: i64) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sessions SET pty_pid = ?1 WHERE id = ?2",
            params![pid, id.to_string()],
        )?;
        Ok(())
    }

    pub fn end_session(&self, id: Uuid, final_state: SessionState) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sessions SET state = ?1, ended_at = ?2 WHERE id = ?3",
            params![final_state.to_string(), Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(())
    }

    pub fn start_cycle(&self, session_id: Uuid) -> Result<Uuid> {
        let conn = self.pool.get()?;
        let ord: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(ordinal), 0) FROM cycles WHERE session_id = ?1",
                params![session_id.to_string()],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO cycles (id, session_id, ordinal, started_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                id.to_string(),
                session_id.to_string(),
                ord + 1,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(id)
    }

    pub fn end_cycle(&self, cycle_id: Uuid, handoff_path: Option<&str>) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE cycles SET ended_at = ?1, handoff_path = ?2 WHERE id = ?3",
            params![Utc::now().to_rfc3339(), handoff_path, cycle_id.to_string()],
        )?;
        Ok(())
    }

    pub fn record_signal(
        &self,
        session_id: Uuid,
        cycle_id: Option<Uuid>,
        kind: &str,
        payload: Option<&str>,
    ) -> Result<i64> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO signals (session_id, cycle_id, kind, payload, at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session_id.to_string(),
                cycle_id.map(|u| u.to_string()),
                kind,
                payload,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_signals_for_session(&self, session_id: Uuid) -> Result<Vec<SignalRow>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, cycle_id, kind, payload, at
             FROM signals WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string()], |r| {
                Ok(SignalRow {
                    id: r.get(0)?,
                    session_id: Uuid::parse_str(&r.get::<_, String>(1)?).unwrap(),
                    cycle_id: r
                        .get::<_, Option<String>>(2)?
                        .map(|s| Uuid::parse_str(&s).unwrap()),
                    kind: r.get(3)?,
                    payload: r.get(4)?,
                    at: DateTime::parse_from_rfc3339(&r.get::<_, String>(5)?)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test db_crud
```

Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/db.rs tests/db_crud.rs
git commit -m "feat(db): project/session/cycle/signal CRUD"
```

---

## Task 8: Signal parser

**Files:**
- Modify: `src/signals.rs`
- Create: `tests/signals.rs`

- [ ] **Step 1: Write failing tests**

`tests/signals.rs`:
```rust
use galvolite::signals::{parse_signal, Signal};

#[test]
fn parses_status_working() {
    assert_eq!(parse_signal("[STATUS:WORKING]"), Some(Signal::StatusWorking));
}

#[test]
fn parses_status_complete() {
    assert_eq!(parse_signal("[STATUS:COMPLETE]"), Some(Signal::StatusComplete));
}

#[test]
fn parses_handoff_ready() {
    assert_eq!(parse_signal("[HANDOFF:READY]"), Some(Signal::HandoffReady));
}

#[test]
fn parses_handoff_blocked_with_reason() {
    let s = parse_signal(r#"[HANDOFF:BLOCKED reason="file missing"]"#);
    assert_eq!(s, Some(Signal::HandoffBlocked { reason: "file missing".into() }));
}

#[test]
fn parses_attention_needed() {
    let s = parse_signal(r#"[ATTENTION:NEEDED reason="ambiguous"]"#);
    assert_eq!(s, Some(Signal::AttentionNeeded { reason: "ambiguous".into() }));
}

#[test]
fn parses_attention_context() {
    assert_eq!(parse_signal("[ATTENTION:CONTEXT]"), Some(Signal::AttentionContext));
}

#[test]
fn parses_plan_ready() {
    assert_eq!(parse_signal("[PLAN:READY]"), Some(Signal::PlanReady));
}

#[test]
fn parses_seed_new() {
    let s = parse_signal(r#"[SEED:NEW id="SEED-007"]"#);
    assert_eq!(s, Some(Signal::SeedNew { id: "SEED-007".into() }));
}

#[test]
fn ignores_non_signal_lines() {
    assert_eq!(parse_signal("regular output"), None);
    assert_eq!(parse_signal(""), None);
    assert_eq!(parse_signal("[NOT:A:SIGNAL]"), None);
}

#[test]
fn requires_signal_to_be_alone_on_line() {
    assert_eq!(parse_signal("prefix [STATUS:WORKING] suffix"), None);
    assert_eq!(parse_signal("  [STATUS:WORKING]"), Some(Signal::StatusWorking),
        "leading whitespace is ok");
    assert_eq!(parse_signal("[STATUS:WORKING]  "), Some(Signal::StatusWorking),
        "trailing whitespace is ok");
}

#[test]
fn reason_handles_escaped_quote() {
    let s = parse_signal(r#"[ATTENTION:NEEDED reason="he said \"hi\""]"#);
    assert_eq!(s, Some(Signal::AttentionNeeded { reason: r#"he said "hi""#.into() }));
}

#[test]
fn signal_kind_string_for_db() {
    assert_eq!(Signal::StatusWorking.kind(), "STATUS:WORKING");
    assert_eq!(Signal::HandoffReady.kind(), "HANDOFF:READY");
    assert_eq!(
        Signal::AttentionNeeded { reason: "x".into() }.kind(),
        "ATTENTION:NEEDED"
    );
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test signals
```

Expected: compile failures; `Signal` and `parse_signal` not yet defined.

- [ ] **Step 3: Implement parser**

`src/signals.rs`:
```rust
//! Signal protocol parsing.
//!
//! Signals are single-line markers Claude emits. See design spec §8.3 / Appendix A.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    StatusWorking,
    StatusComplete,
    AttentionNeeded { reason: String },
    AttentionContext,
    HandoffReady,
    HandoffBlocked { reason: String },
    PlanReady,
    SeedNew { id: String },
}

impl Signal {
    /// Stable string used for DB storage (kind column).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::StatusWorking => "STATUS:WORKING",
            Self::StatusComplete => "STATUS:COMPLETE",
            Self::AttentionNeeded { .. } => "ATTENTION:NEEDED",
            Self::AttentionContext => "ATTENTION:CONTEXT",
            Self::HandoffReady => "HANDOFF:READY",
            Self::HandoffBlocked { .. } => "HANDOFF:BLOCKED",
            Self::PlanReady => "PLAN:READY",
            Self::SeedNew { .. } => "SEED:NEW",
        }
    }

    pub fn payload(&self) -> Option<String> {
        match self {
            Self::AttentionNeeded { reason } | Self::HandoffBlocked { reason } => {
                Some(reason.clone())
            }
            Self::SeedNew { id } => Some(id.clone()),
            _ => None,
        }
    }
}

/// Parse a single line. Returns `Some(Signal)` iff the line, after trimming,
/// is exactly a well-formed signal. Any extra non-whitespace content → None.
pub fn parse_signal(line: &str) -> Option<Signal> {
    let line = line.trim();
    if !line.starts_with('[') || !line.ends_with(']') {
        return None;
    }
    let inner = &line[1..line.len() - 1];

    // Fast path: parameterless signals.
    match inner {
        "STATUS:WORKING" => return Some(Signal::StatusWorking),
        "STATUS:COMPLETE" => return Some(Signal::StatusComplete),
        "ATTENTION:CONTEXT" => return Some(Signal::AttentionContext),
        "HANDOFF:READY" => return Some(Signal::HandoffReady),
        "PLAN:READY" => return Some(Signal::PlanReady),
        _ => {}
    }

    // Parameterized signals: "<KIND> <key>=\"<value>\""
    let (kind, rest) = inner.split_once(' ')?;
    let rest = rest.trim();

    match kind {
        "ATTENTION:NEEDED" => {
            let reason = extract_quoted(rest, "reason")?;
            Some(Signal::AttentionNeeded { reason })
        }
        "HANDOFF:BLOCKED" => {
            let reason = extract_quoted(rest, "reason")?;
            Some(Signal::HandoffBlocked { reason })
        }
        "SEED:NEW" => {
            let id = extract_quoted(rest, "id")?;
            Some(Signal::SeedNew { id })
        }
        _ => None,
    }
}

/// Extract the value of `<key>="<value>"` from `s`. Supports backslash-escaped quotes.
fn extract_quoted(s: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=\"");
    let rest = s.strip_prefix(&prefix)?;
    let mut out = String::with_capacity(rest.len());
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let next = chars.next()?;
                out.push(next);
            }
            '"' => {
                // Expect end-of-string (maybe trailing space).
                let tail: String = chars.collect();
                if tail.trim().is_empty() {
                    return Some(out);
                }
                return None;
            }
            other => out.push(other),
        }
    }
    None
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test signals
```

Expected: 12 passed.

- [ ] **Step 5: Commit**

```bash
git add src/signals.rs tests/signals.rs
git commit -m "feat(signals): protocol parser"
```

---

## Task 9: Streaming line buffer (partial-read safe)

**Files:**
- Modify: `src/signals.rs`

- [ ] **Step 1: Write failing tests**

Append to `tests/signals.rs`:

```rust
use galvolite::signals::LineBuffer;

#[test]
fn line_buffer_accumulates_partial_reads() {
    let mut buf = LineBuffer::new();
    let a = buf.feed("hello");
    assert!(a.is_empty());
    let b = buf.feed(" world\n");
    assert_eq!(b, vec!["hello world"]);
}

#[test]
fn line_buffer_multiple_lines_in_one_feed() {
    let mut buf = LineBuffer::new();
    let lines = buf.feed("a\nb\nc\n");
    assert_eq!(lines, vec!["a", "b", "c"]);
}

#[test]
fn line_buffer_handles_crlf() {
    let mut buf = LineBuffer::new();
    let lines = buf.feed("one\r\ntwo\r\n");
    assert_eq!(lines, vec!["one", "two"]);
}

#[test]
fn line_buffer_keeps_trailing_incomplete_line() {
    let mut buf = LineBuffer::new();
    let a = buf.feed("done\nstill here");
    assert_eq!(a, vec!["done"]);
    let b = buf.feed("\n");
    assert_eq!(b, vec!["still here"]);
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test signals
```

Expected: 4 new tests fail to compile.

- [ ] **Step 3: Implement `LineBuffer`**

Append to `src/signals.rs`:

```rust
/// Buffers byte-stream chunks and yields complete lines.
/// Handles partial reads and CRLF line endings.
#[derive(Debug, Default)]
pub struct LineBuffer {
    pending: String,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk. Returns complete lines (in order). Incomplete trailing
    /// content is retained for the next call.
    pub fn feed(&mut self, chunk: &str) -> Vec<String> {
        self.pending.push_str(chunk);
        let mut out = Vec::new();

        while let Some(nl) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=nl).collect();
            let trimmed = line.trim_end_matches(|c| c == '\n' || c == '\r');
            out.push(trimmed.to_string());
        }
        out
    }

    pub fn pending(&self) -> &str {
        &self.pending
    }
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test signals
```

Expected: 16 passed.

- [ ] **Step 5: Commit**

```bash
git add src/signals.rs tests/signals.rs
git commit -m "feat(signals): line buffer for partial reads"
```

---

## Task 10: CLAUDE.md Implementation profile + injection

**Files:**
- Modify: `src/claude_md.rs`

- [ ] **Step 1: Write failing test**

Create `tests/claude_md.rs`:

```rust
use galvolite::claude_md::{implementation_profile, InjectionPrompt};

#[test]
fn implementation_profile_has_required_sections() {
    let md = implementation_profile();
    assert!(md.contains("# CLAUDE.md"));
    assert!(md.contains("[STATUS:WORKING]"));
    assert!(md.contains("[STATUS:COMPLETE]"));
    assert!(md.contains("[HANDOFF:READY]"));
    assert!(md.contains("[ATTENTION:NEEDED"));
    assert!(md.contains("[ATTENTION:CONTEXT]"));
    assert!(md.contains(".galvolite/handoffs"));
    assert!(md.contains("Continuation Prompt"));
}

#[test]
fn injection_prompt_references_profile_and_task() {
    let p = InjectionPrompt {
        task_summary: "Build the thing",
        session_id: "abc-123",
        claude_md_path: "/tmp/CLAUDE.md",
    };
    let out = p.render();
    assert!(out.contains("Build the thing"));
    assert!(out.contains("abc-123"));
    assert!(out.contains("/tmp/CLAUDE.md"));
    assert!(out.contains("Read"), "prompt must instruct Claude to read the profile");
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test claude_md
```

Expected: compile failure.

- [ ] **Step 3: Implement**

`src/claude_md.rs`:
```rust
//! CLAUDE.md profile templates + injection-prompt rendering.
//!
//! At session start, GalvoLite:
//!   1. Writes the appropriate profile to the session's worktree as `CLAUDE.md`
//!      (or a unique path inside `.galvolite/` for test isolation).
//!   2. Sends an initial prompt telling Claude to read the profile and begin.

/// The Implementation profile. See design spec §10.2.
pub fn implementation_profile() -> &'static str {
    IMPL_PROFILE
}

const IMPL_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Implementation Session

You are a Claude Code session managed by GalvoLite. Follow this contract:

## Signals

Emit these on their own line (no surrounding text, no code fences):

- `[STATUS:WORKING]` — heartbeat every 10-15 tool calls or after a meaningful subtask.
- `[STATUS:COMPLETE]` — task done, tests pass, branch pushed, working tree clean.
- `[ATTENTION:NEEDED reason="<brief>"]` — needs human input. Provide context on subsequent lines.
- `[ATTENTION:CONTEXT]` — you notice your context is getting large; handoff soon.
- `[HANDOFF:READY]` — after writing the handoff file and validating it.
- `[HANDOFF:BLOCKED reason="<brief>"]` — cannot complete the handoff.

## Handoff Protocol

When the harness tells you "Handoff time":

1. Commit any work-in-progress with a `wip:` prefix.
2. Append a timestamped entry to `.galvolite/handoffs/<session-id>/<ISO8601>.md`.
   Include these sections, each filled (no placeholders):
   - Task Assignment
   - Current State (file paths, line numbers)
   - Decisions Made (with rejected alternatives and why)
   - Modified Files
   - Open Questions / Blockers
   - Next Steps (numbered)
   - Critical Context
   - Git State
   - **Continuation Prompt** — a verbose, self-contained briefing for a fresh
     instance. The next cycle reads ONLY this prompt. Err on the side of more
     context. Include exact file paths and line numbers, decisions, rejected
     alternatives, the specific next step, and anything non-obvious.
3. Emit `[HANDOFF:READY]` on its own line.
4. Wait. The harness will send `/clear` and then a restore prompt.

## Restore Protocol

When you receive a prompt pointing you at a handoff file, read it and resume
from the Continuation Prompt section. You have no memory of prior cycles —
trust only the handoff file and the project state.

## Scope Rules

- Stay within your assigned task and worktree.
- If you need to modify files outside your assignment, emit `[ATTENTION:NEEDED]`.
- Commit frequently. Use conventional commit prefixes (feat, fix, chore, test, docs, refactor).

## Do Not

- Emit signals inside code blocks or quoted text.
- Modify this file.
- Create PRs yourself (the harness handles it).
- Abbreviate handoff content. The next cycle's success depends on its quality.
"#;

#[derive(Debug, Clone)]
pub struct InjectionPrompt<'a> {
    pub task_summary: &'a str,
    pub session_id: &'a str,
    pub claude_md_path: &'a str,
}

impl<'a> InjectionPrompt<'a> {
    pub fn render(&self) -> String {
        format!(
            "You are GalvoLite session {session}. Read `{path}` for your behavioral contract, \
             then begin this task:\n\n{task}\n\nEmit `[STATUS:WORKING]` once you've read the \
             profile and understood the task.",
            session = self.session_id,
            path = self.claude_md_path,
            task = self.task_summary,
        )
    }
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test claude_md
```

Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/claude_md.rs tests/claude_md.rs
git commit -m "feat(claude_md): implementation profile + injection prompt"
```

---

## Task 11: Pty wrapper — spawn, read, write, kill

**Files:**
- Modify: `src/pty.rs`

- [ ] **Step 1: Write failing test**

Create `tests/pty_basic.rs`:

```rust
use galvolite::pty::{PtyHandle, PtyOpts};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn spawn_echo_and_read_stdout() {
    let opts = PtyOpts {
        cwd: std::env::temp_dir(),
        command: "/bin/echo".to_string(),
        args: vec!["hello from pty".into()],
        env: vec![],
    };
    let mut handle = PtyHandle::spawn(opts).expect("spawn");

    // Collect all lines until the child exits or we time out.
    let mut collected = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while let Ok(Some(line)) = timeout(Duration::from_millis(500), handle.next_line()).await {
        collected.push_str(&line);
        collected.push('\n');
        if tokio::time::Instant::now() >= deadline {
            break;
        }
    }

    assert!(collected.contains("hello from pty"), "stdout: {collected:?}");
}

#[tokio::test]
async fn write_to_pty_reaches_child() {
    // cat echoes stdin to stdout.
    let opts = PtyOpts {
        cwd: std::env::temp_dir(),
        command: "/bin/cat".to_string(),
        args: vec![],
        env: vec![],
    };
    let mut handle = PtyHandle::spawn(opts).expect("spawn");
    handle.write(b"ping\n").await.expect("write");

    let line = timeout(Duration::from_secs(2), handle.next_line())
        .await
        .expect("timeout")
        .expect("line");
    assert!(line.contains("ping"), "got: {line:?}");

    handle.kill().await.expect("kill");
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test pty_basic
```

Expected: compile errors for `PtyHandle`, `PtyOpts`, etc.

- [ ] **Step 3: Implement**

`src/pty.rs`:
```rust
//! Pty wrapper. Each session owns one long-lived child process.
//!
//! Architecture:
//!   - `portable_pty` spawns a child under a master pty.
//!   - A dedicated blocking thread reads bytes from the pty master and pushes
//!     them through a `LineBuffer`, emitting complete lines into an mpsc channel.
//!   - A second blocking thread receives byte chunks on an mpsc channel and
//!     writes them to the pty master (stdin).
//!   - The async world interacts solely via those channels + a kill flag.

use crate::error::{Error, Result};
use crate::signals::LineBuffer;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct PtyOpts {
    pub cwd: std::path::PathBuf,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct PtyHandle {
    pub pid: u32,
    stdout_rx: mpsc::Receiver<String>,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_flag: Arc<AtomicBool>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    pub fn spawn(opts: PtyOpts) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::Pty(format!("openpty: {e}")))?;

        let mut cmd = CommandBuilder::new(&opts.command);
        for a in &opts.args {
            cmd.arg(a);
        }
        cmd.cwd(&opts.cwd);
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::Pty(format!("spawn: {e}")))?;
        let pid = child.process_id().unwrap_or(0);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::Pty(format!("clone_reader: {e}")))?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::Pty(format!("take_writer: {e}")))?;

        let kill_flag = Arc::new(AtomicBool::new(false));

        // Reader thread
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(256);
        let kill_reader = kill_flag.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut line_buf = LineBuffer::new();
            while !kill_reader.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        let lines = line_buf.feed(&chunk);
                        for line in lines {
                            if stdout_tx.blocking_send(line).is_err() {
                                return;
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        // Writer thread
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        std::thread::spawn(move || {
            while let Some(bytes) = stdin_rx.blocking_recv() {
                if writer.write_all(&bytes).is_err() {
                    return;
                }
                let _ = writer.flush();
            }
        });

        Ok(Self {
            pid,
            stdout_rx,
            stdin_tx,
            kill_flag,
            child,
        })
    }

    /// Next complete line of stdout, or `None` if the stream has ended.
    pub async fn next_line(&mut self) -> Option<String> {
        self.stdout_rx.recv().await
    }

    /// Write bytes to the child's stdin (via pty master).
    pub async fn write(&self, bytes: &[u8]) -> Result<()> {
        self.stdin_tx
            .send(bytes.to_vec())
            .await
            .map_err(|_| Error::Pty("stdin channel closed".into()))
    }

    pub async fn write_str(&self, s: &str) -> Result<()> {
        self.write(s.as_bytes()).await
    }

    pub async fn kill(&mut self) -> Result<()> {
        self.kill_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        Ok(())
    }
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        self.kill_flag.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
    }
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test pty_basic
```

Expected: 2 passed. (Test may be slow first time; allow 10s.)

- [ ] **Step 5: Commit**

```bash
git add src/pty.rs tests/pty_basic.rs
git commit -m "feat(pty): spawn/read/write/kill via portable-pty"
```

---

## Task 12: Signal event pump — pty output → parsed signals → DB

**Files:**
- Modify: `src/pty.rs`

- [ ] **Step 1: Write failing test**

Create `tests/pty_signal_pump.rs`:

```rust
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use galvolite::signals::Signal;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;

#[tokio::test]
async fn signals_emitted_on_dedicated_channel_and_persisted() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let proj = db.insert_project(NewProject {
        name: "p".into(), path: tmp.path().to_string_lossy().into(), remote_url: None,
    }).unwrap();
    let sess = db.insert_session(NewSession {
        project_id: proj.id, role: "implementation".into(),
        branch: None, worktree_path: tmp.path().to_string_lossy().into(),
        task_summary: "t".into(), parent_session_id: None,
    }).unwrap();

    // Use `printf` to emit a signal line, a status line, then exit.
    let handle = PtyHandle::spawn(PtyOpts {
        cwd: tmp.path().to_path_buf(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), "printf '[STATUS:WORKING]\\n[HANDOFF:READY]\\nregular output\\n'".into()],
        env: vec![],
    }).unwrap();

    let mut pump = SignalPump::start(handle, db.clone(), sess.id);

    // Collect 2 signals.
    let mut got = Vec::new();
    for _ in 0..2 {
        let s = timeout(Duration::from_secs(3), pump.next_signal())
            .await
            .expect("timeout")
            .expect("signal");
        got.push(s);
    }

    assert_eq!(got, vec![Signal::StatusWorking, Signal::HandoffReady]);

    // Persisted in DB
    let rows = db.list_signals_for_session(sess.id).unwrap();
    let kinds: Vec<String> = rows.iter().map(|r| r.kind.clone()).collect();
    assert!(kinds.contains(&"STATUS:WORKING".to_string()));
    assert!(kinds.contains(&"HANDOFF:READY".to_string()));
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test pty_signal_pump
```

Expected: missing `SignalPump` type.

- [ ] **Step 3: Implement `SignalPump`**

Append to `src/pty.rs`:

```rust
use crate::db::Db;
use crate::signals::{parse_signal, Signal};
use uuid::Uuid;

/// Wraps a `PtyHandle`. Splits the stdout line stream into:
///   * a channel of human-readable output lines (for display)
///   * a channel of parsed signals (for the ritual driver)
/// Signals are persisted to the `signals` table as they arrive.
pub struct SignalPump {
    signal_rx: mpsc::Receiver<Signal>,
    output_rx: mpsc::Receiver<String>,
    handle: PtyHandleShared,
    _task: tokio::task::JoinHandle<()>,
}

/// A clonable "write + kill" view of the underlying pty for the ritual driver.
#[derive(Clone)]
pub struct PtyHandleShared {
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_flag: Arc<AtomicBool>,
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
    }
}

impl SignalPump {
    pub fn start(mut pty: PtyHandle, db: Db, session_id: Uuid) -> Self {
        let (signal_tx, signal_rx) = mpsc::channel::<Signal>(64);
        let (output_tx, output_rx) = mpsc::channel::<String>(256);
        let shared = PtyHandleShared {
            stdin_tx: pty.stdin_tx.clone(),
            kill_flag: pty.kill_flag.clone(),
        };

        let task = tokio::spawn(async move {
            while let Some(line) = pty.next_line().await {
                if let Some(sig) = parse_signal(&line) {
                    let _ = db.record_signal(
                        session_id,
                        None,
                        sig.kind(),
                        sig.payload().as_deref(),
                    );
                    if signal_tx.send(sig).await.is_err() {
                        break;
                    }
                } else if output_tx.send(line).await.is_err() {
                    break;
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

Also promote the two private fields on `PtyHandle` (`stdin_tx`, `kill_flag`) from private to `pub(crate)` so `SignalPump::start` can read them:

In `src/pty.rs`, change:
```rust
pub struct PtyHandle {
    pub pid: u32,
    stdout_rx: mpsc::Receiver<String>,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_flag: Arc<AtomicBool>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}
```
to:
```rust
pub struct PtyHandle {
    pub pid: u32,
    stdout_rx: mpsc::Receiver<String>,
    pub(crate) stdin_tx: mpsc::Sender<Vec<u8>>,
    pub(crate) kill_flag: Arc<AtomicBool>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test pty_signal_pump
cargo test --test pty_basic  # regression check
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/pty.rs tests/pty_signal_pump.rs
git commit -m "feat(pty): signal pump with DB persistence and shared write handle"
```

---

## Task 13: Handoff file format, path resolution, validation

**Files:**
- Modify: `src/handoff.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/handoff.rs`:

```rust
use chrono::{TimeZone, Utc};
use galvolite::handoff::{handoff_path, validate_handoff_content, HandoffDir};
use tempfile::TempDir;

#[test]
fn path_is_under_session_subdir() {
    let tmp = TempDir::new().unwrap();
    let dir = HandoffDir::new(tmp.path().to_path_buf());

    let session_id = uuid::Uuid::new_v4();
    let when = Utc.with_ymd_and_hms(2026, 4, 15, 14, 32, 0).unwrap();

    let p = handoff_path(&dir, session_id, when);
    let s = p.to_string_lossy();

    assert!(s.contains(&format!("{}", session_id)));
    assert!(s.ends_with("2026-04-15T14-32-00.md"));
    assert!(s.starts_with(&tmp.path().to_string_lossy().to_string()));
}

#[test]
fn validate_accepts_well_formed_handoff() {
    let body = r#"# Handoff: 2026-04-15 14:32

## Task Assignment
Build the thing.

## Current State
Files touched: src/foo.rs:10-42

## Decisions Made
- Chose X over Y because …

## Modified Files
- src/foo.rs

## Open Questions / Blockers
None.

## Next Steps
1. Wire up bar

## Critical Context
- The legacy foo hates X

## Git State
- Branch: main
- Last commit: abc1234

## Continuation Prompt
You are session XYZ. Read .galvolite/handoffs/XYZ/prev.md. Resume by …
"#;
    assert!(validate_handoff_content(body).is_ok());
}

#[test]
fn validate_rejects_missing_continuation_prompt() {
    let body = "# Handoff\n## Task Assignment\nfoo\n";
    let err = validate_handoff_content(body).unwrap_err();
    assert!(format!("{err}").contains("Continuation Prompt"));
}

#[test]
fn validate_rejects_placeholder_content() {
    let body = r#"# Handoff

## Task Assignment
TODO

## Continuation Prompt
TBD
"#;
    let err = validate_handoff_content(body).unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("TODO") || s.contains("TBD") || s.contains("placeholder"));
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test handoff
```

Expected: compile failure.

- [ ] **Step 3: Implement**

`src/handoff.rs`:
```rust
//! Handoff files live at `<root>/<session-id>/<YYYY-MM-DDTHH-MM-SS>.md`.
//! Root is typically `<project>/.galvolite/handoffs/` but tests inject a temp root.

use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct HandoffDir(PathBuf);

impl HandoffDir {
    pub fn new(root: PathBuf) -> Self {
        Self(root)
    }

    pub fn root(&self) -> &Path {
        &self.0
    }
}

pub fn handoff_path(dir: &HandoffDir, session_id: Uuid, when: DateTime<Utc>) -> PathBuf {
    let fname = when.format("%Y-%m-%dT%H-%M-%S.md").to_string();
    dir.root().join(session_id.to_string()).join(fname)
}

pub fn ensure_session_dir(dir: &HandoffDir, session_id: Uuid) -> Result<PathBuf> {
    let p = dir.root().join(session_id.to_string());
    std::fs::create_dir_all(&p).map_err(|e| Error::io(&p, e))?;
    Ok(p)
}

pub fn read_handoff(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

pub fn latest_handoff_for(dir: &HandoffDir, session_id: Uuid) -> Result<Option<PathBuf>> {
    let session_dir = dir.root().join(session_id.to_string());
    if !session_dir.exists() {
        return Ok(None);
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&session_dir)
        .map_err(|e| Error::io(&session_dir, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    entries.sort();
    Ok(entries.into_iter().last())
}

pub const REQUIRED_SECTIONS: &[&str] = &[
    "Task Assignment",
    "Current State",
    "Decisions Made",
    "Modified Files",
    "Open Questions",
    "Next Steps",
    "Critical Context",
    "Git State",
    "Continuation Prompt",
];

const PLACEHOLDER_TOKENS: &[&str] = &["TBD", "TODO", "<fill in>", "XXX"];

pub fn validate_handoff_content(body: &str) -> Result<()> {
    // Required sections
    for section in REQUIRED_SECTIONS {
        let header = format!("## {section}");
        if !body.contains(&header) {
            return Err(Error::HandoffInvalid(format!(
                "missing required section: {section}"
            )));
        }
    }

    // Continuation Prompt must be non-empty
    if let Some(idx) = body.find("## Continuation Prompt") {
        let rest = &body[idx + "## Continuation Prompt".len()..];
        let next_section = rest.find("\n## ").unwrap_or(rest.len());
        let content = rest[..next_section].trim();
        if content.is_empty() {
            return Err(Error::HandoffInvalid(
                "Continuation Prompt is empty".into(),
            ));
        }
        if content.len() < 50 {
            return Err(Error::HandoffInvalid(
                "Continuation Prompt is too short (<50 chars) to be self-contained".into(),
            ));
        }
    }

    // Placeholder scan
    for tok in PLACEHOLDER_TOKENS {
        if body.contains(tok) {
            return Err(Error::HandoffInvalid(format!(
                "contains placeholder token '{tok}'"
            )));
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test handoff
```

Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
git add src/handoff.rs tests/handoff.rs
git commit -m "feat(handoff): path resolution + structural validation"
```

---

## Task 14: Ritual state machine (pure, no I/O)

**Files:**
- Modify: `src/ritual.rs`

- [ ] **Step 1: Write failing test**

Create `tests/ritual_state.rs`:

```rust
use galvolite::ritual::{RitualEvent, RitualMachine, RitualState};
use galvolite::signals::Signal;

#[test]
fn starts_in_working_after_initial_status() {
    let mut m = RitualMachine::new();
    assert_eq!(m.state(), &RitualState::Idle);
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    assert_eq!(m.state(), &RitualState::Working);
}

#[test]
fn request_handoff_transitions_to_awaiting_ready() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    assert_eq!(m.state(), &RitualState::AwaitingHandoffReady);
}

#[test]
fn handoff_ready_transitions_to_clearing() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffReady));
    assert_eq!(m.state(), &RitualState::Clearing);
}

#[test]
fn clear_sent_transitions_to_restoring() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffReady));
    m.on_event(RitualEvent::ClearSent);
    assert_eq!(m.state(), &RitualState::Restoring);
}

#[test]
fn restore_prompt_sent_returns_to_working() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffReady));
    m.on_event(RitualEvent::ClearSent);
    m.on_event(RitualEvent::RestorePromptSent);
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    assert_eq!(m.state(), &RitualState::Working);
}

#[test]
fn handoff_blocked_goes_to_error() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffBlocked { reason: "no disk".into() }));
    match m.state() {
        RitualState::Error(_) => {}
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn attention_needed_transitions_to_attention() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::Signal(Signal::AttentionNeeded { reason: "?".into() }));
    assert_eq!(m.state(), &RitualState::Attention);
}

#[test]
fn status_complete_transitions_to_complete() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::Signal(Signal::StatusComplete));
    assert_eq!(m.state(), &RitualState::Complete);
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test ritual_state
```

Expected: missing types.

- [ ] **Step 3: Implement**

`src/ritual.rs`:
```rust
//! Ritual state machine + driver.
//!
//! The state machine (`RitualMachine`) is pure: events in, state out. It
//! emits no I/O. The driver (task 15) observes state transitions and performs
//! side effects (write prompt, send /clear, write handoff, etc.).

use crate::signals::Signal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RitualState {
    /// Newly constructed, no signals seen yet.
    Idle,
    /// Normal operation.
    Working,
    /// User (or autopilot) has requested a handoff; waiting for `[HANDOFF:READY]`.
    AwaitingHandoffReady,
    /// Handoff validated; about to send `/clear`.
    Clearing,
    /// `/clear` sent; about to send restore prompt.
    Restoring,
    /// Claude emitted `[ATTENTION:NEEDED ...]`. User must respond.
    Attention,
    /// `[STATUS:COMPLETE]` received. Terminal (session ready for PR step).
    Complete,
    /// Something went wrong; human attention required.
    Error(String),
}

#[derive(Debug, Clone)]
pub enum RitualEvent {
    Signal(Signal),
    /// User (or autopilot) asks to initiate the ritual.
    RequestHandoff,
    /// Driver has sent `/clear`.
    ClearSent,
    /// Driver has sent the restore prompt.
    RestorePromptSent,
    /// User supplied the response Claude was waiting for.
    AttentionResolved,
    /// Fatal abort; state machine terminates.
    Abort(String),
}

#[derive(Debug, Clone)]
pub struct RitualMachine {
    state: RitualState,
}

impl RitualMachine {
    pub fn new() -> Self {
        Self { state: RitualState::Idle }
    }

    pub fn state(&self) -> &RitualState {
        &self.state
    }

    pub fn on_event(&mut self, ev: RitualEvent) {
        self.state = match (&self.state, ev) {
            // First heartbeat lifts us out of Idle.
            (RitualState::Idle, RitualEvent::Signal(Signal::StatusWorking)) => {
                RitualState::Working
            }

            // Handoff request path
            (RitualState::Working, RitualEvent::RequestHandoff) => {
                RitualState::AwaitingHandoffReady
            }
            (RitualState::AwaitingHandoffReady, RitualEvent::Signal(Signal::HandoffReady)) => {
                RitualState::Clearing
            }
            (
                RitualState::AwaitingHandoffReady,
                RitualEvent::Signal(Signal::HandoffBlocked { reason }),
            ) => RitualState::Error(format!("handoff blocked: {reason}")),
            (RitualState::Clearing, RitualEvent::ClearSent) => RitualState::Restoring,
            (RitualState::Restoring, RitualEvent::RestorePromptSent) => {
                // Wait for next StatusWorking from the restored cycle.
                RitualState::AwaitingHandoffReady  // placeholder; will be overridden below
            }

            // Restored cycle heartbeat
            (RitualState::Restoring, RitualEvent::Signal(Signal::StatusWorking)) => {
                RitualState::Working
            }

            // Attention path
            (_, RitualEvent::Signal(Signal::AttentionNeeded { .. })) => RitualState::Attention,
            (RitualState::Attention, RitualEvent::AttentionResolved) => RitualState::Working,
            (_, RitualEvent::Signal(Signal::AttentionContext)) => {
                // Safety-net self-signal: treated as an implicit handoff request iff Working.
                match self.state {
                    RitualState::Working => RitualState::AwaitingHandoffReady,
                    _ => self.state.clone(),
                }
            }

            // Completion
            (_, RitualEvent::Signal(Signal::StatusComplete)) => RitualState::Complete,

            // Abort
            (_, RitualEvent::Abort(reason)) => RitualState::Error(reason),

            // Ignored signals (PlanReady, SeedNew) in this scope
            (state, _) => state.clone(),
        };

        // Post-step: after RestorePromptSent we are waiting for next heartbeat.
        // The match above produced AwaitingHandoffReady as a sentinel; fix it.
        if matches!(self.state, RitualState::AwaitingHandoffReady)
            && matches!(
                self.last_event_was_restore_prompt_sent,
                Some(true)
            )
        {
            self.state = RitualState::Restoring;
        }
        self.last_event_was_restore_prompt_sent = None;
    }
}
```

Hmm — that's a buggy pattern. Replace the implementation block with this cleaner one (no hidden flags):

```rust
impl RitualMachine {
    pub fn new() -> Self {
        Self { state: RitualState::Idle }
    }

    pub fn state(&self) -> &RitualState {
        &self.state
    }

    pub fn on_event(&mut self, ev: RitualEvent) {
        use RitualEvent as E;
        use RitualState as S;
        use Signal as Sg;

        let next = match (self.state.clone(), ev) {
            (S::Idle, E::Signal(Sg::StatusWorking)) => S::Working,

            (S::Working, E::RequestHandoff) => S::AwaitingHandoffReady,

            (S::AwaitingHandoffReady, E::Signal(Sg::HandoffReady)) => S::Clearing,
            (S::AwaitingHandoffReady, E::Signal(Sg::HandoffBlocked { reason })) => {
                S::Error(format!("handoff blocked: {reason}"))
            }

            (S::Clearing, E::ClearSent) => S::Restoring,
            (S::Restoring, E::RestorePromptSent) => S::Restoring,
            (S::Restoring, E::Signal(Sg::StatusWorking)) => S::Working,

            (_, E::Signal(Sg::AttentionNeeded { .. })) => S::Attention,
            (S::Attention, E::AttentionResolved) => S::Working,

            (S::Working, E::Signal(Sg::AttentionContext)) => S::AwaitingHandoffReady,

            (_, E::Signal(Sg::StatusComplete)) => S::Complete,
            (_, E::Abort(r)) => S::Error(r),

            (s, _) => s,
        };
        self.state = next;
    }
}
```

(Discard the buggy block. The final code for Task 14 is the cleaner one directly above.)

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test ritual_state
```

Expected: 8 passed.

- [ ] **Step 5: Commit**

```bash
git add src/ritual.rs tests/ritual_state.rs
git commit -m "feat(ritual): pure state machine"
```

---

## Task 15: Ritual driver — ties machine + pty + handoff together

**Files:**
- Modify: `src/ritual.rs`

- [ ] **Step 1: Write failing test (end-to-end driver)**

The real end-to-end test lives in task 23 (with mock-claude). For task 15 we need a narrow unit-style test that proves the driver sequences side effects in order.

Create `tests/ritual_driver.rs`:

```rust
use galvolite::ritual::{DriverCommand, MockSideEffects, RitualDriver};
use galvolite::signals::Signal;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn full_ritual_sequence_issues_expected_commands() {
    // Mock side effects: writes command trace to a channel.
    let (trace_tx, mut trace_rx) = tokio::sync::mpsc::channel::<DriverCommand>(64);
    let mock = MockSideEffects::new(trace_tx);
    let driver = RitualDriver::new_for_test(mock);

    // Prime with a working heartbeat.
    driver.on_signal(Signal::StatusWorking).await;

    // User asks for handoff.
    driver.request_handoff().await;
    // Driver should now have injected the "Handoff time" prompt.
    let c1 = timeout(Duration::from_millis(500), trace_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(c1, DriverCommand::InjectPrompt(ref s) if s.contains("Handoff time")));

    // Claude writes file, emits HANDOFF:READY.
    driver.on_signal(Signal::HandoffReady).await;

    // Driver should send /clear keystroke.
    let c2 = timeout(Duration::from_millis(500), trace_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(c2, DriverCommand::SendClear));

    // Then the restore prompt.
    let c3 = timeout(Duration::from_millis(500), trace_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(c3, DriverCommand::InjectPrompt(ref s) if s.contains("Read")));
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test ritual_driver
```

Expected: missing `RitualDriver`, `DriverCommand`, `MockSideEffects`.

- [ ] **Step 3: Implement driver with side-effect abstraction**

Append to `src/ritual.rs`:

```rust
use tokio::sync::{mpsc, Mutex};
use std::sync::Arc;

/// Side effects the driver needs. Abstracted so tests can observe without a real pty.
#[async_trait::async_trait]
pub trait SideEffects: Send + Sync + 'static {
    async fn inject_prompt(&self, prompt: &str);
    async fn send_clear(&self);
}

#[derive(Debug, Clone)]
pub enum DriverCommand {
    InjectPrompt(String),
    SendClear,
}

/// Test double: records commands to a channel.
pub struct MockSideEffects {
    tx: mpsc::Sender<DriverCommand>,
}

impl MockSideEffects {
    pub fn new(tx: mpsc::Sender<DriverCommand>) -> Self {
        Self { tx }
    }
}

#[async_trait::async_trait]
impl SideEffects for MockSideEffects {
    async fn inject_prompt(&self, prompt: &str) {
        let _ = self.tx.send(DriverCommand::InjectPrompt(prompt.to_string())).await;
    }
    async fn send_clear(&self) {
        let _ = self.tx.send(DriverCommand::SendClear).await;
    }
}

pub struct RitualDriver {
    machine: Arc<Mutex<RitualMachine>>,
    fx: Arc<dyn SideEffects>,
    latest_handoff_path: Arc<Mutex<Option<String>>>,
}

impl RitualDriver {
    pub fn new(fx: Arc<dyn SideEffects>) -> Self {
        Self {
            machine: Arc::new(Mutex::new(RitualMachine::new())),
            fx,
            latest_handoff_path: Arc::new(Mutex::new(None)),
        }
    }

    /// Test-only constructor.
    pub fn new_for_test(fx: MockSideEffects) -> Self {
        Self::new(Arc::new(fx))
    }

    pub async fn state(&self) -> RitualState {
        self.machine.lock().await.state().clone()
    }

    pub async fn set_latest_handoff_path(&self, path: String) {
        *self.latest_handoff_path.lock().await = Some(path);
    }

    pub async fn request_handoff(&self) {
        let before = self.machine.lock().await.state().clone();
        if !matches!(before, RitualState::Working) {
            tracing::warn!(?before, "ignoring handoff request from non-Working state");
            return;
        }
        self.machine.lock().await.on_event(RitualEvent::RequestHandoff);
        self.fx
            .inject_prompt(
                "Handoff time. Follow the Handoff Protocol in your CLAUDE.md. \
                 Write your handoff file now, then emit [HANDOFF:READY] on its own line.",
            )
            .await;
    }

    pub async fn on_signal(&self, sig: Signal) {
        let sig_clone = sig.clone();
        self.machine.lock().await.on_event(RitualEvent::Signal(sig));

        // Drive follow-up side effects based on new state.
        let state = self.machine.lock().await.state().clone();
        match (state, sig_clone) {
            (RitualState::Clearing, Signal::HandoffReady) => {
                self.fx.send_clear().await;
                self.machine.lock().await.on_event(RitualEvent::ClearSent);

                // Immediately inject restore prompt.
                let restore = self.build_restore_prompt().await;
                self.fx.inject_prompt(&restore).await;
                self.machine
                    .lock()
                    .await
                    .on_event(RitualEvent::RestorePromptSent);
            }
            _ => {}
        }
    }

    async fn build_restore_prompt(&self) -> String {
        let path = self
            .latest_handoff_path
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "<latest handoff>".to_string());
        format!(
            "Read `{path}`. You are the next cycle of the same session. \
             Resume from the Continuation Prompt section. \
             You have no memory of prior cycles — trust only the handoff file and the project state. \
             Emit [STATUS:WORKING] once you have read and understood the handoff."
        )
    }

    pub async fn resolve_attention(&self) {
        self.machine
            .lock()
            .await
            .on_event(RitualEvent::AttentionResolved);
    }

    pub async fn abort(&self, reason: &str) {
        self.machine
            .lock()
            .await
            .on_event(RitualEvent::Abort(reason.to_string()));
    }
}
```

Add `async-trait = "0.1"` to `[dependencies]` in `Cargo.toml`:

```toml
async-trait = "0.1"
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test ritual_driver
cargo test --test ritual_state  # regression
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/ritual.rs tests/ritual_driver.rs Cargo.toml Cargo.lock
git commit -m "feat(ritual): driver with side-effect trait + mock"
```

---

## Task 16: Real side-effects impl wiring pty + handoff validation

**Files:**
- Modify: `src/ritual.rs`

- [ ] **Step 1: Implement `PtySideEffects`**

Append to `src/ritual.rs`:

```rust
use crate::pty::PtyHandleShared;

/// Production side effects: forwards to a pty handle.
pub struct PtySideEffects {
    pty: PtyHandleShared,
}

impl PtySideEffects {
    pub fn new(pty: PtyHandleShared) -> Self {
        Self { pty }
    }
}

#[async_trait::async_trait]
impl SideEffects for PtySideEffects {
    async fn inject_prompt(&self, prompt: &str) {
        // Claude Code's CLI accepts input on stdin terminated by Enter.
        // We send the prompt followed by a newline to submit it.
        let mut buf = prompt.as_bytes().to_vec();
        buf.push(b'\n');
        let _ = self.pty.write(&buf).await;
    }

    async fn send_clear(&self) {
        // `/clear` is a slash command typed at the prompt.
        let _ = self.pty.write_str("/clear\n").await;
    }
}
```

- [ ] **Step 2: Add handoff validation wire-in**

Extend `RitualDriver` with a method that validates the latest handoff file on disk before proceeding past `Clearing`. Insert after the existing `request_handoff` method:

```rust
impl RitualDriver {
    /// After [HANDOFF:READY], driver calls this to confirm the file actually exists
    /// and is well-formed. If validation fails, driver aborts with an error state.
    pub async fn validate_latest_handoff(&self) -> Result<(), crate::error::Error> {
        let path = self
            .latest_handoff_path
            .lock()
            .await
            .clone()
            .ok_or_else(|| crate::error::Error::Ritual("no handoff path recorded".into()))?;
        let body = crate::handoff::read_handoff(std::path::Path::new(&path))?;
        crate::handoff::validate_handoff_content(&body)
    }
}
```

And modify `on_signal` so that it calls `validate_latest_handoff` before issuing `send_clear`. Replace the `(RitualState::Clearing, Signal::HandoffReady) => { ... }` arm with:

```rust
(RitualState::Clearing, Signal::HandoffReady) => {
    if let Err(e) = self.validate_latest_handoff().await {
        tracing::error!(error = %e, "handoff validation failed");
        self.fx
            .inject_prompt(&format!(
                "Handoff validation failed: {e}. Fix the handoff file and re-emit [HANDOFF:READY]."
            ))
            .await;
        // Revert to AwaitingHandoffReady so next HandoffReady retries.
        // (Direct state mutation is acceptable here — we own the lock.)
        let mut m = self.machine.lock().await;
        m.state = RitualState::AwaitingHandoffReady;
        return;
    }
    self.fx.send_clear().await;
    self.machine.lock().await.on_event(RitualEvent::ClearSent);
    let restore = self.build_restore_prompt().await;
    self.fx.inject_prompt(&restore).await;
    self.machine
        .lock()
        .await
        .on_event(RitualEvent::RestorePromptSent);
}
```

Make `RitualMachine.state` field visible to the driver's module — change the struct in `src/ritual.rs`:

```rust
pub struct RitualMachine {
    pub(crate) state: RitualState,
}
```

- [ ] **Step 3: Write test for validation-failure retry path**

Append to `tests/ritual_driver.rs`:

```rust
#[tokio::test]
async fn handoff_validation_failure_triggers_retry_prompt() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    // No handoff file actually written → read fails.

    let (trace_tx, mut trace_rx) = tokio::sync::mpsc::channel(64);
    let mock = MockSideEffects::new(trace_tx);
    let driver = RitualDriver::new_for_test(mock);
    driver.set_latest_handoff_path(tmp.path().join("missing.md").to_string_lossy().into()).await;

    driver.on_signal(Signal::StatusWorking).await;
    driver.request_handoff().await;
    let _prompt = trace_rx.recv().await.unwrap();

    driver.on_signal(Signal::HandoffReady).await;

    // Driver should have injected a retry prompt, NOT sent /clear.
    let next = trace_rx.recv().await.unwrap();
    match next {
        DriverCommand::InjectPrompt(s) => assert!(s.contains("validation failed")),
        other => panic!("expected retry prompt, got {other:?}"),
    }
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test ritual_driver
```

Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
git add src/ritual.rs tests/ritual_driver.rs
git commit -m "feat(ritual): pty side-effects + handoff validation retry"
```

---

## Task 17: CLI command definitions (clap)

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Define command tree**

`src/cli.rs`:
```rust
use clap::{Parser, Subcommand};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "galvolite", version, about = "Solo continuity engine for Claude Code")]
pub struct Cli {
    /// Override the app data directory (default: OS-standard).
    #[arg(long, global = true)]
    pub data_dir: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub cmd: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Register a project directory.
    AddProject {
        /// Path to the project's git repo.
        path: std::path::PathBuf,
        /// Optional human-readable name (defaults to directory name).
        #[arg(long)]
        name: Option<String>,
    },
    /// List registered projects.
    Projects,
    /// Start a new session in a project.
    StartSession {
        /// Project UUID.
        #[arg(long)]
        project: Uuid,
        /// One-line task summary.
        #[arg(long)]
        task: String,
        /// Role (implementation | brainstorm | scout | fix).
        #[arg(long, default_value = "implementation")]
        role: String,
        /// Command to spawn (default: `claude`).
        #[arg(long, default_value = "claude")]
        claude_cmd: String,
    },
    /// List active sessions.
    Sessions,
    /// Show a session's state + recent signals.
    Status {
        session: Uuid,
    },
    /// Send text to a session's stdin (no trailing newline added).
    Send {
        session: Uuid,
        /// Text to inject (use --newline to append \n).
        text: String,
        #[arg(long)]
        newline: bool,
    },
    /// Trigger the handoff ritual for a session.
    Handoff {
        session: Uuid,
    },
    /// Kill a session (sends SIGTERM and marks it killed in DB).
    Kill {
        session: Uuid,
    },
}
```

- [ ] **Step 2: Wire `main.rs`**

`src/main.rs`:
```rust
use anyhow::Result;
use clap::Parser;
use galvolite::cli::{Cli, Command};

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

    match cli.cmd {
        Command::AddProject { path, name } => cmd_add_project(&db, path, name),
        Command::Projects => cmd_list_projects(&db),
        Command::StartSession { project, task, role, claude_cmd } => {
            cmd_start_session(&db, project, task, role, claude_cmd).await
        }
        Command::Sessions => cmd_list_sessions(&db),
        Command::Status { session } => cmd_status(&db, session),
        Command::Send { session, text, newline } => {
            cmd_send(&db, session, text, newline).await
        }
        Command::Handoff { session } => cmd_handoff(&db, session).await,
        Command::Kill { session } => cmd_kill(&db, session).await,
    }
}

fn resolve_data_dir(override_dir: Option<std::path::PathBuf>) -> Result<std::path::PathBuf> {
    if let Some(d) = override_dir {
        return Ok(d);
    }
    let base = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine OS data dir"))?;
    Ok(base.join("GalvoLite"))
}

// Stubs filled in by tasks 18-21.
fn cmd_add_project(_db: &galvolite::db::Db, _path: std::path::PathBuf, _name: Option<String>) -> Result<()> {
    anyhow::bail!("not yet implemented — task 18")
}
fn cmd_list_projects(_db: &galvolite::db::Db) -> Result<()> {
    anyhow::bail!("not yet implemented — task 18")
}
async fn cmd_start_session(_db: &galvolite::db::Db, _project: uuid::Uuid, _task: String, _role: String, _claude_cmd: String) -> Result<()> {
    anyhow::bail!("not yet implemented — task 19")
}
fn cmd_list_sessions(_db: &galvolite::db::Db) -> Result<()> {
    anyhow::bail!("not yet implemented — task 19")
}
fn cmd_status(_db: &galvolite::db::Db, _session: uuid::Uuid) -> Result<()> {
    anyhow::bail!("not yet implemented — task 19")
}
async fn cmd_send(_db: &galvolite::db::Db, _session: uuid::Uuid, _text: String, _newline: bool) -> Result<()> {
    anyhow::bail!("not yet implemented — task 20")
}
async fn cmd_handoff(_db: &galvolite::db::Db, _session: uuid::Uuid) -> Result<()> {
    anyhow::bail!("not yet implemented — task 21")
}
async fn cmd_kill(_db: &galvolite::db::Db, _session: uuid::Uuid) -> Result<()> {
    anyhow::bail!("not yet implemented — task 21")
}
```

- [ ] **Step 3: Build**

```bash
cargo build
./target/debug/galvolite --help
```

Expected: help text lists all subcommands.

- [ ] **Step 4: Commit**

```bash
git add src/cli.rs src/main.rs
git commit -m "feat(cli): command definitions and argument parsing"
```

---

## Task 18: `add-project` + `projects` commands

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing end-to-end CLI test**

Create `tests/cli_projects.rs`:

```rust
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn add_and_list_project() {
    let data_dir = TempDir::new().unwrap();
    let proj_dir = TempDir::new().unwrap();

    Command::cargo_bin("galvolite")
        .unwrap()
        .args(&[
            "--data-dir", &data_dir.path().to_string_lossy(),
            "add-project", &proj_dir.path().to_string_lossy(),
            "--name", "Test Project",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added project"));

    Command::cargo_bin("galvolite")
        .unwrap()
        .args(&["--data-dir", &data_dir.path().to_string_lossy(), "projects"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Project"));
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo test --test cli_projects
```

Expected: panic — "not yet implemented".

- [ ] **Step 3: Implement commands**

Replace the stubs in `src/main.rs`:

```rust
fn cmd_add_project(db: &galvolite::db::Db, path: std::path::PathBuf, name: Option<String>) -> Result<()> {
    let abs = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("resolve {}: {e}", path.display()))?;
    let name = name.unwrap_or_else(|| {
        abs.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".into())
    });
    let p = db.insert_project(galvolite::db::NewProject {
        name: name.clone(),
        path: abs.to_string_lossy().into_owned(),
        remote_url: None,
    })?;
    println!("Added project {} ({}) at {}", p.name, p.id, p.path);
    Ok(())
}

fn cmd_list_projects(db: &galvolite::db::Db) -> Result<()> {
    let projects = db.list_projects()?;
    if projects.is_empty() {
        println!("(no projects registered)");
        return Ok(());
    }
    for p in projects {
        println!("{}  {}  {}", p.id, p.name, p.path);
    }
    Ok(())
}
```

- [ ] **Step 4: Run — expect pass**

```bash
cargo test --test cli_projects
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs tests/cli_projects.rs
git commit -m "feat(cli): add-project and projects commands"
```

---

## Task 19: `start-session`, `sessions`, `status` commands (single-process daemon model)

**Files:**
- Modify: `src/main.rs`
- Create: `src/session_registry.rs`
- Modify: `src/lib.rs`

**Background:** A CLI that spawns a pty and then exits leaves the pty orphaned. We need a persistent daemon-like process to own live sessions. For plan 1a, we take a pragmatic approach: **`start-session` blocks** — it spawns the pty and stays in the foreground, printing output to the terminal and listening for signals. Other commands (`send`, `handoff`, `status`, `kill`) communicate with the running `start-session` process via a **per-session Unix socket** at `<data-dir>/sockets/<session-id>.sock`.

This keeps plan 1a in a single process per session (no separate daemon), which is sufficient for proving the ritual end-to-end. Plan 1b's Tauri shell will replace this with an in-app session registry.

- [ ] **Step 1: Define the socket protocol**

`src/session_registry.rs`:
```rust
//! Per-session control socket.
//!
//! `start-session` listens on `<data-dir>/sockets/<session-id>.sock`.
//! Other CLI commands (`send`, `handoff`, `kill`) connect and send a single
//! JSON-line command.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlMessage {
    Send { text: String, newline: bool },
    Handoff,
    Kill,
    Status,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ControlResponse {
    Ok(String),
    Err(String),
}

pub fn socket_path(data_dir: &std::path::Path, session_id: uuid::Uuid) -> PathBuf {
    data_dir.join("sockets").join(format!("{session_id}.sock"))
}

pub async fn listen(
    path: &std::path::Path,
) -> Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| crate::error::Error::io(parent, e))?;
    }
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path).map_err(|e| crate::error::Error::io(path, e))
}

pub async fn send_command(
    sock_path: &std::path::Path,
    msg: ControlMessage,
) -> Result<ControlResponse> {
    let stream = UnixStream::connect(sock_path)
        .await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    let (reader, mut writer) = stream.into_split();
    let line = serde_json::to_string(&msg)
        .map_err(|e| crate::error::Error::Config(format!("serialize: {e}")))?;
    writer.write_all(line.as_bytes()).await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    writer.write_all(b"\n").await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    writer.shutdown().await.ok();

    let mut br = BufReader::new(reader);
    let mut buf = String::new();
    br.read_line(&mut buf).await
        .map_err(|e| crate::error::Error::io(sock_path, e))?;
    let resp: ControlResponse = serde_json::from_str(buf.trim())
        .map_err(|e| crate::error::Error::Config(format!("deserialize: {e}")))?;
    Ok(resp)
}
```

Add module to `src/lib.rs`:

```rust
pub mod session_registry;
```

- [ ] **Step 2: Implement `start-session` (the daemon mode)**

Replace `cmd_start_session` in `src/main.rs`:

```rust
async fn cmd_start_session(
    db: &galvolite::db::Db,
    project_id: uuid::Uuid,
    task: String,
    role: String,
    claude_cmd: String,
) -> Result<()> {
    use galvolite::claude_md::{implementation_profile, InjectionPrompt};
    use galvolite::db::NewSession;
    use galvolite::handoff::HandoffDir;
    use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
    use galvolite::ritual::{PtySideEffects, RitualDriver};
    use galvolite::session_registry::{listen, socket_path, ControlMessage, ControlResponse};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let project = db.get_project(project_id)?;

    let session = db.insert_session(NewSession {
        project_id: project.id,
        role: role.clone(),
        branch: None,
        worktree_path: project.path.clone(),
        task_summary: task.clone(),
        parent_session_id: None,
    })?;

    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session.id);
    let listener = listen(&sock).await.map_err(|e| anyhow::anyhow!("{e}"))?;

    let worktree = std::path::PathBuf::from(&project.path);
    let galvolite_dir = worktree.join(".galvolite");
    std::fs::create_dir_all(&galvolite_dir)?;
    let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
    std::fs::write(&claude_md_path, implementation_profile())?;
    let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
    galvolite::handoff::ensure_session_dir(&handoff_dir, session.id)?;

    tracing::info!(%session.id, "spawning claude");
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: worktree.clone(),
        command: claude_cmd,
        args: vec![],
        env: vec![],
    }).map_err(|e| anyhow::anyhow!("{e}"))?;
    db.update_session_pid(session.id, pty.pid as i64)?;
    let mut pump = SignalPump::start(pty, db.clone(), session.id);

    let fx = Arc::new(PtySideEffects::new(pump.handle()));
    let driver = Arc::new(RitualDriver::new(fx));
    db.update_session_state(session.id, galvolite::db::SessionState::Working)?;

    // Inject initial prompt.
    let injection = InjectionPrompt {
        task_summary: &task,
        session_id: &session.id.to_string(),
        claude_md_path: &claude_md_path.to_string_lossy(),
    };
    pump.handle().write_str(&format!("{}\n", injection.render())).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Session {} started. Socket: {}", session.id, sock.display());

    // Main loop: interleave pty output, signals, and control socket connections.
    let driver_for_sig = driver.clone();
    let db_for_sig = db.clone();
    let session_id_for_sig = session.id;
    let handoff_dir_for_sig = handoff_dir.clone();
    let signal_task = tokio::spawn(async move {
        while let Some(sig) = pump.next_signal().await {
            // If HandoffReady, find the latest handoff file on disk and record it.
            if matches!(sig, galvolite::signals::Signal::HandoffReady) {
                if let Ok(Some(p)) =
                    galvolite::handoff::latest_handoff_for(&handoff_dir_for_sig, session_id_for_sig)
                {
                    driver_for_sig.set_latest_handoff_path(p.to_string_lossy().into()).await;
                }
            }
            driver_for_sig.on_signal(sig).await;
        }
        tracing::info!("signal stream ended");
        let _ = db_for_sig.end_session(session_id_for_sig, galvolite::db::SessionState::Complete);
    });

    let output_task = tokio::spawn(async move {
        while let Some(line) = pump.next_output().await {
            println!("{line}");
        }
    });

    let driver_for_ctrl = driver.clone();
    let db_for_ctrl = db.clone();
    let control_task = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(e) => { tracing::error!(?e, "accept"); continue; }
            };
            let driver = driver_for_ctrl.clone();
            let db = db_for_ctrl.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = stream.into_split();
                let mut br = BufReader::new(reader);
                let mut line = String::new();
                if br.read_line(&mut line).await.is_err() { return; }
                let msg: ControlMessage = match serde_json::from_str(line.trim()) {
                    Ok(m) => m,
                    Err(e) => {
                        let _ = respond(&mut writer, ControlResponse::Err(format!("{e}"))).await;
                        return;
                    }
                };
                let resp = handle_control(&db, &driver, session_id_for_sig, msg).await;
                let _ = respond(&mut writer, resp).await;
            });
        }
    });

    // Block until signal_task finishes (session ends) or SIGINT.
    tokio::select! {
        _ = signal_task => {},
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("received ctrl-c, shutting down");
            let _ = db.end_session(session.id, galvolite::db::SessionState::Killed);
        }
    }
    output_task.abort();
    control_task.abort();
    let _ = std::fs::remove_file(&sock);
    Ok(())
}

async fn respond<W: tokio::io::AsyncWriteExt + Unpin>(
    w: &mut W,
    resp: galvolite::session_registry::ControlResponse,
) -> anyhow::Result<()> {
    let s = serde_json::to_string(&resp)?;
    w.write_all(s.as_bytes()).await?;
    w.write_all(b"\n").await?;
    w.shutdown().await?;
    Ok(())
}

async fn handle_control(
    db: &galvolite::db::Db,
    driver: &std::sync::Arc<galvolite::ritual::RitualDriver>,
    session_id: uuid::Uuid,
    msg: galvolite::session_registry::ControlMessage,
) -> galvolite::session_registry::ControlResponse {
    use galvolite::session_registry::{ControlMessage as M, ControlResponse as R};
    match msg {
        M::Send { text, newline } => {
            // Routed back out through driver's side effects.
            let mut buf = text.into_bytes();
            if newline { buf.push(b'\n'); }
            // Access pty via driver's fx — not directly exposed. Add a helper:
            driver.inject_raw(&buf).await;
            R::Ok("sent".into())
        }
        M::Handoff => {
            driver.request_handoff().await;
            R::Ok("handoff initiated".into())
        }
        M::Kill => {
            let _ = db.end_session(session_id, galvolite::db::SessionState::Killed);
            driver.abort("killed by user").await;
            R::Ok("killed".into())
        }
        M::Status => {
            let s = driver.state().await;
            R::Ok(format!("{s:?}"))
        }
    }
}
```

Add `inject_raw` to `RitualDriver` in `src/ritual.rs`:

```rust
impl RitualDriver {
    pub async fn inject_raw(&self, bytes: &[u8]) {
        // Expose a raw write path. We add a dedicated method on SideEffects
        // for this, since `inject_prompt` stringifies + newlines.
        self.fx.inject_raw(bytes).await;
    }
}
```

Extend the trait:
```rust
#[async_trait::async_trait]
pub trait SideEffects: Send + Sync + 'static {
    async fn inject_prompt(&self, prompt: &str);
    async fn send_clear(&self);
    async fn inject_raw(&self, bytes: &[u8]);
}
```

And update `PtySideEffects` and `MockSideEffects`:

```rust
#[async_trait::async_trait]
impl SideEffects for PtySideEffects {
    async fn inject_prompt(&self, prompt: &str) {
        let mut buf = prompt.as_bytes().to_vec();
        buf.push(b'\n');
        let _ = self.pty.write(&buf).await;
    }
    async fn send_clear(&self) {
        let _ = self.pty.write_str("/clear\n").await;
    }
    async fn inject_raw(&self, bytes: &[u8]) {
        let _ = self.pty.write(bytes).await;
    }
}

#[async_trait::async_trait]
impl SideEffects for MockSideEffects {
    async fn inject_prompt(&self, prompt: &str) {
        let _ = self.tx.send(DriverCommand::InjectPrompt(prompt.to_string())).await;
    }
    async fn send_clear(&self) {
        let _ = self.tx.send(DriverCommand::SendClear).await;
    }
    async fn inject_raw(&self, bytes: &[u8]) {
        let _ = self.tx.send(DriverCommand::InjectPrompt(
            String::from_utf8_lossy(bytes).to_string()
        )).await;
    }
}
```

Add the `InjectRaw` variant to `DriverCommand` if you prefer to distinguish in tests:
```rust
#[derive(Debug, Clone)]
pub enum DriverCommand {
    InjectPrompt(String),
    SendClear,
    InjectRaw(Vec<u8>),
}
```
and update `MockSideEffects::inject_raw` accordingly:
```rust
async fn inject_raw(&self, bytes: &[u8]) {
    let _ = self.tx.send(DriverCommand::InjectRaw(bytes.to_vec())).await;
}
```

Also implement `cmd_list_sessions` and `cmd_status`:

```rust
fn cmd_list_sessions(db: &galvolite::db::Db) -> Result<()> {
    let ss = db.list_sessions()?;
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
        println!("    [{}] {}{}", sig.at.format("%H:%M:%S"), sig.kind,
            sig.payload.as_deref().map(|p| format!(" reason=\"{p}\"")).unwrap_or_default());
    }
    Ok(())
}
```

- [ ] **Step 3: Build**

```bash
cargo build
```

Expected: compiles.

- [ ] **Step 4: Smoke test**

```bash
./target/debug/galvolite --data-dir /tmp/gl-test add-project /tmp
./target/debug/galvolite --data-dir /tmp/gl-test projects
./target/debug/galvolite --data-dir /tmp/gl-test sessions
```

Expected: first adds a project, second lists it, third shows "(no active sessions)".

- [ ] **Step 5: Commit**

```bash
git add src/ src/session_registry.rs
git commit -m "feat(cli): start-session daemon with control socket + sessions/status"
```

---

## Task 20: `send` command (via socket)

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement `cmd_send`**

```rust
async fn cmd_send(
    _db: &galvolite::db::Db,
    session_id: uuid::Uuid,
    text: String,
    newline: bool,
) -> Result<()> {
    use galvolite::session_registry::{send_command, socket_path, ControlMessage, ControlResponse};
    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session_id);
    let resp = send_command(&sock, ControlMessage::Send { text, newline }).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    match resp {
        ControlResponse::Ok(msg) => println!("{msg}"),
        ControlResponse::Err(msg) => anyhow::bail!("{msg}"),
    }
    Ok(())
}
```

- [ ] **Step 2: Manual smoke test**

(Requires a running `start-session`. Skip for now if no real claude; covered in task 23 integration test.)

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): send command via control socket"
```

---

## Task 21: `handoff` and `kill` commands

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement**

```rust
async fn cmd_handoff(_db: &galvolite::db::Db, session_id: uuid::Uuid) -> Result<()> {
    use galvolite::session_registry::{send_command, socket_path, ControlMessage, ControlResponse};
    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session_id);
    let resp = send_command(&sock, ControlMessage::Handoff).await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    match resp {
        ControlResponse::Ok(msg) => println!("{msg}"),
        ControlResponse::Err(msg) => anyhow::bail!("{msg}"),
    }
    Ok(())
}

async fn cmd_kill(db: &galvolite::db::Db, session_id: uuid::Uuid) -> Result<()> {
    use galvolite::session_registry::{send_command, socket_path, ControlMessage, ControlResponse};
    let data_dir = resolve_data_dir(None)?;
    let sock = socket_path(&data_dir, session_id);
    // Try socket first; fall back to direct DB mark.
    match send_command(&sock, ControlMessage::Kill).await {
        Ok(ControlResponse::Ok(msg)) => println!("{msg}"),
        Ok(ControlResponse::Err(msg)) => anyhow::bail!("{msg}"),
        Err(_) => {
            // start-session not running; just mark in DB.
            db.end_session(session_id, galvolite::db::SessionState::Killed)?;
            println!("session marked killed (daemon was not running)");
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build + smoke**

```bash
cargo build
./target/debug/galvolite --help | grep -E "handoff|kill"
```

Expected: commands listed.

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat(cli): handoff and kill commands"
```

---

## Task 22: Mock-claude test fixture

**Files:**
- Create: `fixtures/mock-claude/Cargo.toml`
- Create: `fixtures/mock-claude/src/main.rs`
- Modify: `Cargo.toml` (workspace)

- [ ] **Step 1: Convert root Cargo.toml to a workspace**

Top of `Cargo.toml`:

```toml
[workspace]
members = [".", "fixtures/mock-claude"]
```

(Keep the existing `[package]`, `[dependencies]`, etc. blocks after `[workspace]`.)

- [ ] **Step 2: Create mock-claude**

`fixtures/mock-claude/Cargo.toml`:
```toml
[package]
name = "mock-claude"
version = "0.1.0"
edition = "2021"

[dependencies]
```

`fixtures/mock-claude/src/main.rs`:
```rust
//! A scriptable stand-in for the real `claude` CLI, used by integration tests.
//!
//! Behavior:
//!   * Reads lines from stdin.
//!   * If an input line matches a trigger pattern from an internal script,
//!     emits the corresponding response lines to stdout (including signals).
//!   * Supports the `/clear` keystroke: on seeing `/clear`, resets an internal
//!     "cycle counter" and emits a new [STATUS:WORKING] after next input.
//!
//! The script is hardcoded for the Plan 1a integration test scenario:
//!
//!   cycle 1:
//!     on first "begin" prompt → emit [STATUS:WORKING]
//!     on "Handoff time" prompt → write handoff file, emit [HANDOFF:READY]
//!   after /clear:
//!     on next prompt (the restore prompt) → emit [STATUS:WORKING]
//!
//! Handoff file path comes from the GALVO_MOCK_HANDOFF_DIR env var + session id
//! extracted from the restore prompt (simple parse).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

fn main() {
    let mut cycle = 1u32;
    let mut handoff_pending = false;

    // Flush stdout often.
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    writeln!(out, "mock-claude v1 (cycle {cycle}) ready").ok();
    out.flush().ok();

    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle /clear
        if line.trim() == "/clear" {
            cycle += 1;
            writeln!(out, "(mock-claude cleared; now cycle {cycle})").ok();
            out.flush().ok();
            handoff_pending = false;
            continue;
        }

        // Begin → heartbeat
        if line.contains("begin") || line.contains("GalvoLite session") {
            writeln!(out, "mock: starting task").ok();
            writeln!(out, "[STATUS:WORKING]").ok();
            out.flush().ok();
            continue;
        }

        // Handoff trigger
        if line.contains("Handoff time") {
            // Write a minimal valid handoff file.
            let handoff_dir = std::env::var("GALVO_MOCK_HANDOFF_DIR")
                .expect("GALVO_MOCK_HANDOFF_DIR must be set");
            let session_id = std::env::var("GALVO_MOCK_SESSION_ID")
                .expect("GALVO_MOCK_SESSION_ID must be set");
            let when = chrono_like_timestamp();
            let dir = PathBuf::from(&handoff_dir).join(&session_id);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{when}.md"));
            std::fs::write(&path, MINIMAL_HANDOFF).unwrap();
            writeln!(out, "mock: wrote handoff at {}", path.display()).ok();
            writeln!(out, "[HANDOFF:READY]").ok();
            out.flush().ok();
            handoff_pending = true;
            continue;
        }

        // Restore prompt → heartbeat for new cycle
        if line.contains("Read") && line.contains(".galvolite/handoffs") || line.contains("next cycle") {
            writeln!(out, "mock: restored (cycle {cycle})").ok();
            writeln!(out, "[STATUS:WORKING]").ok();
            out.flush().ok();
            continue;
        }

        // Everything else — echo back
        let _ = handoff_pending;
        writeln!(out, "mock: echoed '{line}'").ok();
        out.flush().ok();
    }
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    format!("mock-{secs}")
}

const MINIMAL_HANDOFF: &str = r#"# Handoff: mock

## Task Assignment
Execute the integration test scenario.

## Current State
Emitted STATUS:WORKING at cycle start. Task is a mock and requires no real work.

## Decisions Made
- Chose to emit minimal handoff to satisfy validator (rejected alternative: richer body, unnecessary for test).

## Modified Files
- (none — mock)

## Open Questions / Blockers
None.

## Next Steps
1. Resume in next cycle and emit STATUS:WORKING

## Critical Context
- This is a mock. The real Claude writes much richer handoffs.

## Git State
- Branch: main
- Last commit: (mock)

## Continuation Prompt
You are the restored cycle of a GalvoLite integration test session. The previous cycle ran a mock-claude binary. Upon receiving your restore prompt, emit [STATUS:WORKING] on its own line to confirm the ritual completed. Do no other work. This prompt is intentionally long enough to pass the validator's 50-character minimum.
"#;
```

- [ ] **Step 3: Build**

```bash
cargo build -p mock-claude
ls -la target/debug/mock-claude
```

Expected: binary exists.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml fixtures/
git commit -m "test(fixture): mock-claude scriptable stand-in"
```

---

## Task 23: End-to-end ritual integration test

**Files:**
- Create: `tests/ritual_end_to_end.rs`

- [ ] **Step 1: Write the test**

```rust
//! End-to-end: spawn mock-claude via the real PtyHandle + SignalPump +
//! RitualDriver + handoff validation, trigger a handoff, and assert the
//! ritual completes losslessly.

use galvolite::claude_md::{implementation_profile, InjectionPrompt};
use galvolite::db::{Db, NewProject, NewSession, SessionState};
use galvolite::handoff::HandoffDir;
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use galvolite::ritual::{PtySideEffects, RitualDriver, RitualState};
use galvolite::signals::Signal;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

fn mock_claude_path() -> std::path::PathBuf {
    // When `cargo test` runs, binary is at target/<profile>/mock-claude.
    let exe = std::env::current_exe().unwrap();
    let mut dir = exe.parent().unwrap().to_path_buf();
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join("mock-claude")
}

#[tokio::test]
async fn full_ritual_with_mock_claude() {
    // ---------- Setup ----------
    let data_dir = TempDir::new().unwrap();
    let proj_dir = TempDir::new().unwrap();
    let db = Db::open(data_dir.path().join("t.db")).unwrap();

    let proj = db.insert_project(NewProject {
        name: "integration".into(),
        path: proj_dir.path().to_string_lossy().into(),
        remote_url: None,
    }).unwrap();

    let session = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: proj_dir.path().to_string_lossy().into(),
        task_summary: "mock integration".into(),
        parent_session_id: None,
    }).unwrap();

    // Write CLAUDE.md + prep handoff dir
    let galvolite_dir = proj_dir.path().join(".galvolite");
    std::fs::create_dir_all(&galvolite_dir).unwrap();
    let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
    std::fs::write(&claude_md_path, implementation_profile()).unwrap();
    let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
    galvolite::handoff::ensure_session_dir(&handoff_dir, session.id).unwrap();

    // ---------- Spawn mock ----------
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: proj_dir.path().to_path_buf(),
        command: mock_claude_path().to_string_lossy().into(),
        args: vec![],
        env: vec![
            ("GALVO_MOCK_HANDOFF_DIR".into(), handoff_dir.root().to_string_lossy().into()),
            ("GALVO_MOCK_SESSION_ID".into(), session.id.to_string()),
        ],
    }).expect("spawn");
    let mut pump = SignalPump::start(pty, db.clone(), session.id);
    let fx = Arc::new(PtySideEffects::new(pump.handle()));
    let driver = Arc::new(RitualDriver::new(fx));

    // ---------- Inject initial prompt ----------
    let inj = InjectionPrompt {
        task_summary: "mock integration",
        session_id: &session.id.to_string(),
        claude_md_path: &claude_md_path.to_string_lossy(),
    };
    pump.handle().write_str(&format!("{} begin\n", inj.render())).await.unwrap();

    // ---------- Wait for first StatusWorking ----------
    let sig = timeout(Duration::from_secs(5), pump.next_signal())
        .await.expect("timeout").expect("signal");
    assert_eq!(sig, Signal::StatusWorking);
    driver.on_signal(sig).await;
    assert_eq!(driver.state().await, RitualState::Working);

    // ---------- Trigger handoff ----------
    driver.request_handoff().await;
    // Driver injected handoff prompt; mock will write file + emit HandoffReady.

    let sig = timeout(Duration::from_secs(5), pump.next_signal())
        .await.expect("timeout").expect("signal");
    assert_eq!(sig, Signal::HandoffReady);

    // Look up latest handoff file and tell the driver.
    // (In production, cmd_start_session's signal task does this automatically.)
    let latest = galvolite::handoff::latest_handoff_for(&handoff_dir, session.id)
        .unwrap().expect("handoff file written");
    driver.set_latest_handoff_path(latest.to_string_lossy().into()).await;

    driver.on_signal(sig).await;

    // Driver should now be in Restoring (clear sent, restore prompt sent).
    // Wait for mock's post-restore StatusWorking.
    let sig = timeout(Duration::from_secs(5), pump.next_signal())
        .await.expect("timeout").expect("signal");
    assert_eq!(sig, Signal::StatusWorking);
    driver.on_signal(sig).await;

    // ---------- Assert: ritual completed losslessly ----------
    assert_eq!(driver.state().await, RitualState::Working);

    // Signals persisted
    let rows = db.list_signals_for_session(session.id).unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    assert!(kinds.contains(&"STATUS:WORKING"));
    assert!(kinds.contains(&"HANDOFF:READY"));
    // At least 2 StatusWorking entries (one per cycle)
    let hb_count = kinds.iter().filter(|k| **k == "STATUS:WORKING").count();
    assert!(hb_count >= 2, "expected ≥2 heartbeats, got {hb_count}");

    // Cleanup
    sleep(Duration::from_millis(50)).await;
}
```

- [ ] **Step 2: Run — expect pass**

```bash
cargo build -p mock-claude
cargo test --test ritual_end_to_end -- --nocapture
```

Expected: 1 passed. This test is the single most important check in plan 1a — it proves the save/clear/restore ritual works end-to-end.

- [ ] **Step 3: Commit**

```bash
git add tests/ritual_end_to_end.rs
git commit -m "test: end-to-end ritual with mock-claude"
```

---

## Task 24: Full suite sanity + docs

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Run the full test suite**

```bash
cargo test --workspace
```

Expected: all tests pass. If any fail, fix before proceeding.

- [ ] **Step 2: Update README with quickstart**

```markdown
# GalvoLite

Solo continuity engine for Claude Code. See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec.

## Plan 1a Status: Ritual Engine (CLI)

What works:
- Register projects, spawn long-lived Claude sessions via pty, parse signal protocol
- Drive the save→/clear→restore ritual end-to-end
- Signal + session state persisted to SQLite
- Integration test proves the ritual with a mock-claude fixture

## Quickstart

```bash
cargo build
./target/debug/galvolite --data-dir /tmp/gl add-project /path/to/your/repo --name MyProj
./target/debug/galvolite --data-dir /tmp/gl projects
./target/debug/galvolite --data-dir /tmp/gl start-session --project <UUID> --task "Refactor the foo"
# In another terminal:
./target/debug/galvolite --data-dir /tmp/gl status <SESSION-UUID>
./target/debug/galvolite --data-dir /tmp/gl send <SESSION-UUID> "some input" --newline
./target/debug/galvolite --data-dir /tmp/gl handoff <SESSION-UUID>
```

## Running tests

```bash
cargo test --workspace
```

## What's next

Plan 1b: Tauri desktop shell wrapping this ritual engine with a dashboard UI.
```

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: plan 1a quickstart in README"
```

---

## Self-Review Checklist (performed by plan author after writing)

**Spec coverage (design spec §-by-§):**
- §4 Architecture (core daemon) → tasks 6, 11, 12, 17, 19. Tauri/frontend deferred to 1b ✓
- §5 Continuity loop → tasks 13-16 (save/clear/restore mechanics). Plugin trigger deferred to plan 2 ✓
- §5.4 Autopilot → deferred to plan 2 (matches plan boundary) ✓
- §5.5 Memory/handoff split → task 13 (handoff half). Cognitive memory is Claude's own, no code needed ✓
- §6 Decomposition → deferred to plan 3 ✓
- §7 Review loop → deferred to plans 4-5 ✓
- §8 Data model → tasks 5-7 (schema + CRUD), task 8 (signal protocol), task 14 (state machine) ✓
- §9 Filesystem layout → `.galvolite/` dir with CLAUDE-session.md and handoffs/ created by tasks 13, 19 ✓
- §10.2 Implementation profile → task 10 ✓
- §10.1 Brainstorm, §10.3 Scout → deferred to plans 3, 6 ✓
- §11 Seeds → deferred to plan 6 ✓
- §12 Dashboard → deferred to plan 1b ✓
- §14 GitHub → deferred to plan 4 ✓
- §15 Onboarding → minimal (CLI flags, no OAuth) — full onboarding in plan 1b ✓

**Placeholders:** No "TBD", "TODO", "similar to Task N", or vague "add error handling". The handoff validator explicitly fails on TBD/TODO tokens.

**Type consistency:** `Signal` enum variants match across `signals.rs`, `ritual.rs`, tests. `SessionState` string form (`to_string`/`from_str`) matches DB check constraint (`spawning|working|attention_needed|handing_off|restoring|complete|killed|error`). `RitualDriver::state()` returns `RitualState` (same type exported from ritual.rs). `DriverCommand::InjectRaw` added in task 19 is consistent with the updated trait signature.

**Known limitation:** Plan 1a's `start-session` blocks in the foreground. Plan 1b will replace this with an in-process session registry owned by the Tauri app. The Unix-socket protocol designed here transitions cleanly: the Tauri app becomes the socket server; CLI commands remain the client interface.

**Platform:** Unix-only (Unix sockets, /bin/cat, /bin/echo in tests). Windows support is out of scope for plan 1a; add in a later plan.

---

*End of Plan 1a.*
