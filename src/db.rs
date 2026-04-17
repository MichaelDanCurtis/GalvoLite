use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

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
