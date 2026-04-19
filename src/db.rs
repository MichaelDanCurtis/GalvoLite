use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::Serialize;
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

pub use crate::state::SessionState;

const SCHEMA_V1: &str = include_str!("../migrations/0001_initial.sql");
const SCHEMA_V2: &str = include_str!("../migrations/0002_hooks_mcp_foundations.sql");

/// (version, sql_body). Order matters: applied lowest-to-highest. Each entry
/// is applied iff the version is not present in `schema_version`.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, SCHEMA_V2),
];

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

        // Bootstrap: schema_version itself is created by 0001_initial.sql, so the
        // very first call needs to run that file unconditionally before we can
        // query for applied versions.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                 version    INTEGER NOT NULL PRIMARY KEY,
                 applied_at TEXT NOT NULL
             );",
        )?;

        for (version, body) in MIGRATIONS {
            let already: Option<i64> = conn
                .query_row(
                    "SELECT version FROM schema_version WHERE version = ?1",
                    params![*version],
                    |r| r.get(0),
                )
                .ok();
            if already.is_some() {
                continue;
            }
            let tx = conn.transaction()?;
            tx.execute_batch(body)?;
            tx.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)",
                params![*version, Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        Ok(())
    }
}

// ---------- helpers ----------

fn parse_uuid(s: &str) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(e),
        )
    })
}

fn parse_datetime(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
}

// ---------- types ----------

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone)]
pub struct NewSession {
    pub project_id: Uuid,
    pub role: String,
    pub branch: Option<String>,
    pub worktree_path: String,
    pub task_summary: String,
    pub parent_session_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize)]
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
                    id: parse_uuid(&r.get::<_, String>(0)?)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    remote_url: r.get(3)?,
                    created_at: parse_datetime(&r.get::<_, String>(4)?)?,
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
                    id: parse_uuid(&r.get::<_, String>(0)?)?,
                    name: r.get(1)?,
                    path: r.get(2)?,
                    remote_url: r.get(3)?,
                    created_at: parse_datetime(&r.get::<_, String>(4)?)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Cascade-delete a project: every signal, cycle, session row and finally
    /// the project row, inside a single transaction.
    ///
    /// Returns `Error::ProjectNotFound` if the id isn't present.
    pub fn delete_project_cascade(&self, id: uuid::Uuid) -> Result<()> {
        use rusqlite::OptionalExtension;
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;

        // Verify the project exists first so we can return a clean 404.
        let exists: bool = tx
            .query_row(
                "SELECT 1 FROM projects WHERE id = ?1",
                params![id.to_string()],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        if !exists {
            return Err(Error::ProjectNotFound(id));
        }

        tx.execute(
            "DELETE FROM signals WHERE session_id IN
                (SELECT id FROM sessions WHERE project_id = ?1)",
            params![id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM cycles WHERE session_id IN
                (SELECT id FROM sessions WHERE project_id = ?1)",
            params![id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM sessions WHERE project_id = ?1",
            params![id.to_string()],
        )?;
        tx.execute("DELETE FROM projects WHERE id = ?1", params![id.to_string()])?;

        tx.commit()?;
        Ok(())
    }

    pub fn insert_session(&self, new: NewSession) -> Result<Session> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let state = SessionState::Starting;
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
                let state_str = r.get::<_, String>(6)?;
                let state = SessionState::from_str(&state_str).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        6,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e}"))),
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

    /// Mark all non-terminal sessions (`ended_at IS NULL`) as `Error`.
    /// Called on startup to reconcile rows left in `Working`/`Spawning`/
    /// `AttentionNeeded`/etc. by an abrupt server exit. Returns the number
    /// of rows updated.
    pub fn reconcile_orphaned_sessions(&self) -> Result<usize> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET state = ?1, ended_at = ?2 WHERE ended_at IS NULL",
            params![SessionState::Error.to_string(), Utc::now().to_rfc3339()],
        )?;
        Ok(rows)
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

    /// Set the `autopilot` flag on a session row.
    /// Returns `Error::SessionNotFound` if no row was updated.
    pub fn set_session_autopilot(&self, id: uuid::Uuid, enabled: bool) -> Result<()> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET autopilot = ?1 WHERE id = ?2",
            params![enabled as i64, id.to_string()],
        )?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }

    /// Replace the `role` column on a session row. Used by SignalPump when
    /// it observes a `[ROLE:SET role="..."]` signal so the UI can reflect
    /// mid-session role transitions.
    /// Returns `Error::SessionNotFound` if no row was updated.
    pub fn set_session_role(&self, id: Uuid, role: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET role = ?1 WHERE id = ?2",
            params![role, id.to_string()],
        )?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
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

    /// Conditional end: only writes if the session is not already ended
    /// (i.e. `ended_at IS NULL`). Returns `true` if a row was updated,
    /// `false` if the session was already ended (or does not exist).
    ///
    /// This prevents lost-update races where a background completer and a
    /// user-initiated kill both try to write a terminal state — whichever
    /// writes first wins, and the other becomes a no-op.
    pub fn end_session_if_unended(&self, id: Uuid, final_state: SessionState) -> Result<bool> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET state = ?1, ended_at = ?2
             WHERE id = ?3 AND ended_at IS NULL",
            params![final_state.to_string(), Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(rows > 0)
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

    /// Conditional end: only writes if the cycle is not already ended.
    /// Returns `true` if a row was updated, `false` if already ended
    /// (or does not exist). See [`Db::end_session_if_unended`].
    pub fn end_cycle_if_unended(
        &self,
        cycle_id: Uuid,
        handoff_path: Option<&str>,
    ) -> Result<bool> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE cycles SET ended_at = ?1, handoff_path = ?2
             WHERE id = ?3 AND ended_at IS NULL",
            params![Utc::now().to_rfc3339(), handoff_path, cycle_id.to_string()],
        )?;
        Ok(rows > 0)
    }

    /// Append one row to the `events` log inside a caller-supplied transaction.
    /// Returns `(rowid, deduped)` where `deduped = true` if `dedup_key` was
    /// already present (no row was written).
    ///
    /// This is the low-level helper. Callers must use [`crate::state::apply_event`]
    /// for state-changing events so the events row and the sessions update land
    /// inside the same transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn append_event_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
        source: &str,
        kind: &str,
        payload_json: &str,
        prev_state: Option<&str>,
        next_state: Option<&str>,
        dedup_key: Option<&str>,
    ) -> Result<(i64, bool)> {
        if let Some(key) = dedup_key {
            use rusqlite::OptionalExtension;
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM events WHERE dedup_key = ?1",
                    params![key],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok((id, true));
            }
        }
        tx.execute(
            "INSERT INTO events
                (session_id, source, kind, payload_json, prev_state, next_state, dedup_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session_id.to_string(),
                source,
                kind,
                payload_json,
                prev_state,
                next_state,
                dedup_key,
            ],
        )?;
        Ok((tx.last_insert_rowid(), false))
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
                let cycle = match r.get::<_, Option<String>>(2)? {
                    Some(s) => Some(parse_uuid(&s)?),
                    None => None,
                };
                Ok(SignalRow {
                    id: r.get(0)?,
                    session_id: parse_uuid(&r.get::<_, String>(1)?)?,
                    cycle_id: cycle,
                    kind: r.get(3)?,
                    payload: r.get(4)?,
                    at: parse_datetime(&r.get::<_, String>(5)?)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
