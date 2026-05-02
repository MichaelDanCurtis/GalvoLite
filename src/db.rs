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
const SCHEMA_V3: &str = include_str!("../migrations/0003_session_spawn.sql");
const SCHEMA_V4: &str = include_str!("../migrations/0004_overseer_executor.sql");

/// (version, sql_body). Order matters: applied lowest-to-highest. Each entry
/// is applied iff the version is not present in `schema_version`.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, SCHEMA_V1),
    (2, SCHEMA_V2),
    (3, SCHEMA_V3),
    (4, SCHEMA_V4),
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

fn parse_waiting_on(raw: Option<String>) -> Option<Vec<Uuid>> {
    let s = raw?;
    let strs: Vec<String> = serde_json::from_str(&s).ok()?;
    Some(
        strs.into_iter()
            .filter_map(|x| Uuid::parse_str(&x).ok())
            .collect(),
    )
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<Vec<Uuid>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_rejoin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_overseer_recs: Option<String>,
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
            waiting_on: None,
            pending_rejoin: None,
            terminal_status: None,
            pending_overseer_recs: None,
        })
    }

    pub fn get_session(&self, id: Uuid) -> Result<Session> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, project_id, role, branch, worktree_path, pty_pid, state,
                    autopilot, parent_session_id, task_summary, created_at, ended_at,
                    waiting_on, pending_rejoin, terminal_status, pending_overseer_recs
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
                let waiting_on = parse_waiting_on(r.get::<_, Option<String>>(12)?);
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
                    waiting_on,
                    pending_rejoin: r.get(13)?,
                    terminal_status: r.get(14)?,
                    pending_overseer_recs: r.get(15)?,
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
                    autopilot, parent_session_id, task_summary, created_at, ended_at,
                    waiting_on, pending_rejoin, terminal_status, pending_overseer_recs
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
                let waiting_on = parse_waiting_on(r.get::<_, Option<String>>(12)?);
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
                    waiting_on,
                    pending_rejoin: r.get(13)?,
                    terminal_status: r.get(14)?,
                    pending_overseer_recs: r.get(15)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Mark non-terminal sessions that never reached "PTY alive" (i.e.
    /// `pty_pid IS NULL`) as `Error` on startup. Sessions that did record a
    /// `pty_pid` are left untouched here — startup recovery
    /// (`recover_non_terminal_sessions`, spec §11.2) inspects each
    /// recorded pid and decides per-session: alive → preserve + emit
    /// `system.recovered`; dead → orphan with auto-handoff (Task 11).
    /// Returns the number of rows updated.
    pub fn reconcile_orphaned_sessions(&self) -> Result<usize> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET state = ?1, ended_at = ?2
             WHERE ended_at IS NULL AND pty_pid IS NULL",
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

    pub fn update_worktree_path(&self, sid: Uuid, path: &str) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sessions SET worktree_path = ?1 WHERE id = ?2",
            rusqlite::params![path, sid.to_string()],
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

    /// Replace the `role` column on a session row. Used by the `set_role`
    /// MCP tool so the UI can reflect mid-session role transitions.
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

    /// Set the per-session bearer token. Used at spawn time + on reattach.
    /// Returns `Error::SessionNotFound` if no row was updated.
    pub fn set_api_token(&self, id: Uuid, token: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET api_token = ?1 WHERE id = ?2",
            params![token, id.to_string()],
        )?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }

    /// Clear the per-session bearer token (revoke it). Called when a session
    /// transitions to Ended so stale curl attempts get 401.
    pub fn clear_api_token(&self, id: Uuid) -> Result<()> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sessions SET api_token = NULL WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Return `(session_id, api_token)` for every session that currently has
    /// a non-NULL token. Used by `auth::resolve_token` to do constant-time
    /// matching across all candidates.
    pub fn sessions_with_tokens(&self) -> Result<Vec<(Uuid, String)>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, api_token FROM sessions WHERE api_token IS NOT NULL",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let id_s: String = r.get(0)?;
                let token: String = r.get(1)?;
                Ok((parse_uuid(&id_s)?, token))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Update one of the markdown columns on a session row. Used by MCP tools
    /// `set_plan` and `set_seed` to persist the latest body alongside an event
    /// row. Returns `Error::SessionNotFound` if no row was updated.
    pub fn set_session_markdown(&self, id: Uuid, column: &str, body: &str) -> Result<()> {
        // Whitelist columns to keep this from being a SQLi vector.
        let col = match column {
            "plan_markdown" => "plan_markdown",
            "seed_markdown" => "seed_markdown",
            other => {
                return Err(Error::Config(format!(
                    "set_session_markdown: unknown column {other}"
                )))
            }
        };
        let conn = self.pool.get()?;
        let sql = format!("UPDATE sessions SET {col} = ?1 WHERE id = ?2");
        let rows = conn.execute(&sql, params![body, id.to_string()])?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }

    /// Update the `handoff_path` column on a session row.
    pub fn set_session_handoff_path(&self, id: Uuid, path: &str) -> Result<()> {
        let conn = self.pool.get()?;
        let rows = conn.execute(
            "UPDATE sessions SET handoff_path = ?1 WHERE id = ?2",
            params![path, id.to_string()],
        )?;
        if rows == 0 {
            return Err(Error::SessionNotFound(id));
        }
        Ok(())
    }

    /// Read the most recent `limit` event rows for a session, optionally
    /// filtered to events with `id > since`. Returned as JSON values shaped
    /// for the MCP `read_events` tool response.
    pub fn read_events_for_session(
        &self,
        session_id: Uuid,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, ts, source, kind, payload_json, prev_state, next_state
             FROM events
             WHERE session_id = ?1
               AND (?2 IS NULL OR ts > ?2)
             ORDER BY id ASC
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(
                params![session_id.to_string(), since, limit as i64],
                |r| {
                    Ok(serde_json::json!({
                        "id":         r.get::<_, i64>(0)?,
                        "ts":         r.get::<_, String>(1)?,
                        "source":     r.get::<_, String>(2)?,
                        "kind":       r.get::<_, String>(3)?,
                        "payload":    serde_json::from_str::<serde_json::Value>(
                                          &r.get::<_, String>(4)?
                                      ).unwrap_or(serde_json::Value::Null),
                        "prev_state": r.get::<_, Option<String>>(5)?,
                        "next_state": r.get::<_, Option<String>>(6)?,
                    }))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Read the most recent `limit` events for a session in **reverse
    /// chronological** order (newest first). Dashboard-facing counterpart to
    /// [`Self::read_events_for_session`] (which returns ASC for the MCP
    /// `read_events` tool). Same JSON row shape so both consumers can share
    /// UI types.
    ///
    /// Ordering is by `id DESC` — events are append-only with a monotonic
    /// primary key, so this is equivalent to "newest by insertion time"
    /// without depending on the clock-based `ts` column (events that land
    /// inside the same millisecond would otherwise tie-break nondeterministically).
    pub fn read_recent_events(
        &self,
        session_id: Uuid,
        limit: i64,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, ts, source, kind, payload_json, prev_state, next_state
             FROM events
             WHERE session_id = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string(), limit], |r| {
                Ok(serde_json::json!({
                    "id":         r.get::<_, i64>(0)?,
                    "ts":         r.get::<_, String>(1)?,
                    "source":     r.get::<_, String>(2)?,
                    "kind":       r.get::<_, String>(3)?,
                    "payload":    serde_json::from_str::<serde_json::Value>(
                                      &r.get::<_, String>(4)?
                                  ).unwrap_or(serde_json::Value::Null),
                    "prev_state": r.get::<_, Option<String>>(5)?,
                    "next_state": r.get::<_, Option<String>>(6)?,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Return `(session_id, pty_pid)` for every session in a non-terminal state
    /// (`starting | idle | working | awaiting_input | needs_attention`).
    /// Used by startup recovery (spec §11.2). Sessions without a recorded
    /// `pty_pid` are returned with `0`, which `pid_is_alive` treats as dead.
    pub fn list_non_terminal_sessions(&self) -> Result<Vec<(Uuid, i64)>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, COALESCE(pty_pid, 0) FROM sessions
             WHERE state IN ('starting','idle','working','awaiting_input','needs_attention')",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((parse_uuid(&r.get::<_, String>(0)?)?, r.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Read the structured plan steps for a session, ordered by `ordinal ASC`.
    /// Shaped for the dashboard's Plan tab: `{ hash, text, status, note }`.
    ///
    /// The DB stores `status` with `'in_progress'` per the `0002` migration's
    /// CHECK constraint, but the UI `PlanStep` type uses `'working'` — we
    /// rewrite that single value on the way out. All other statuses pass
    /// through unchanged. `note` is always emitted (as JSON `null` when the
    /// column is NULL) so consumers get a stable shape.
    pub fn read_plan_steps(&self, session_id: Uuid) -> Result<Vec<serde_json::Value>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT step_hash, title, status, note
             FROM plan_steps
             WHERE session_id = ?1
             ORDER BY ordinal ASC",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string()], |r| {
                let hash: String = r.get(0)?;
                let text: String = r.get(1)?;
                let status: String = r.get(2)?;
                let note: Option<String> = r.get(3)?;
                let status = if status == "in_progress" {
                    "working".to_string()
                } else {
                    status
                };
                Ok(serde_json::json!({
                    "hash":   hash,
                    "text":   text,
                    "status": status,
                    "note":   note,
                }))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Like `read_plan_steps` but returns `DbPlanStep` (DB-native status,
    /// keyed by `step_hash`) for use by the `apply_event_with_bus` plan
    /// diff. Order: ordinal ASC.
    pub fn read_plan_steps_raw(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<crate::plan_steps::DbPlanStep>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT step_hash, ordinal, title, status
             FROM plan_steps
             WHERE session_id = ?1
             ORDER BY ordinal ASC",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string()], |r| {
                Ok(crate::plan_steps::DbPlanStep {
                    step_hash: r.get(0)?,
                    ordinal:   r.get(1)?,
                    title:     r.get(2)?,
                    status:    r.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Read the current status of a single plan_step row, or None if
    /// the row doesn't exist. DB-native status (`in_progress`, not `working`).
    pub fn read_plan_step_status(
        &self,
        session_id: Uuid,
        step_hash: &str,
    ) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let conn = self.pool.get()?;
        Ok(conn
            .query_row(
                "SELECT status FROM plan_steps WHERE session_id = ?1 AND step_hash = ?2",
                params![session_id.to_string(), step_hash],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Upsert one `plan_steps` row inside a caller-supplied transaction.
    /// Used by `state::apply_event` for `mcp.set_plan` writes. `status` MUST
    /// be DB-native (`pending` / `in_progress` / `done` / `blocked`); see
    /// `crate::plan_steps::PlanStepStatus::as_db_str`.
    pub fn upsert_plan_step_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
        step_hash: &str,
        ordinal: i64,
        title: &str,
        status: &str,
        note: Option<&str>,
    ) -> Result<()> {
        tx.execute(
            "INSERT INTO plan_steps
                (session_id, step_hash, ordinal, title, status, note, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
             ON CONFLICT(session_id, step_hash) DO UPDATE SET
                ordinal    = excluded.ordinal,
                title      = excluded.title,
                status     = excluded.status,
                note       = excluded.note,
                updated_at = excluded.updated_at",
            params![session_id.to_string(), step_hash, ordinal, title, status, note],
        )?;
        Ok(())
    }

    /// Drop every plan_step row for a session inside a caller-supplied
    /// transaction. `set_plan` calls this before re-inserting so removed
    /// steps disappear and ordinals stay dense.
    pub fn delete_plan_steps_for_session_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
    ) -> Result<()> {
        tx.execute(
            "DELETE FROM plan_steps WHERE session_id = ?1",
            params![session_id.to_string()],
        )?;
        Ok(())
    }

    /// Update `status` (+ optional `note`) on a single plan_step row inside
    /// the given transaction. Returns the prior status, or `None` if the
    /// row didn't exist (caller treats as no-op).
    pub fn set_plan_step_status_in_tx(
        tx: &rusqlite::Transaction<'_>,
        session_id: Uuid,
        step_hash: &str,
        status: &str,
        note: Option<&str>,
    ) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        let prev: Option<String> = tx
            .query_row(
                "SELECT status FROM plan_steps
                 WHERE session_id = ?1 AND step_hash = ?2",
                params![session_id.to_string(), step_hash],
                |r| r.get(0),
            )
            .optional()?;
        if prev.is_none() {
            return Ok(None);
        }
        tx.execute(
            "UPDATE plan_steps
             SET status = ?1, note = ?2,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE session_id = ?3 AND step_hash = ?4",
            params![status, note, session_id.to_string(), step_hash],
        )?;
        Ok(prev)
    }

    /// Look up sibling sessions of a given session (same project), excluding
    /// the caller. Returns lightweight rows for the MCP `list_siblings` tool.
    pub fn list_session_siblings(
        &self,
        project_id: Uuid,
        exclude: Option<Uuid>,
    ) -> Result<Vec<serde_json::Value>> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, role, state, last_event_ts
             FROM sessions
             WHERE project_id = ?1
               AND (?2 IS NULL OR id != ?2)
             ORDER BY created_at ASC",
        )?;
        let rows = stmt
            .query_map(
                params![project_id.to_string(), exclude.map(|u| u.to_string())],
                |r| {
                    Ok(serde_json::json!({
                        "id":            r.get::<_, String>(0)?,
                        "role":          r.get::<_, String>(1)?,
                        "state":         r.get::<_, String>(2)?,
                        "last_event_ts": r.get::<_, Option<String>>(3)?,
                    }))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

}
