//! Plan 8 — recommendations DB CRUD with content_hash dedupe.

use crate::db::Db;
use crate::error::{Error, Result};
use chrono::{DateTime, Utc};
use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecStatus { Pending, Acted, Dismissed, Deferred }

impl RecStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending", Self::Acted => "acted",
            Self::Dismissed => "dismissed", Self::Deferred => "deferred",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewRec {
    pub project_id: Uuid,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub rationale: String,
    pub evidence_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Recommendation {
    pub id: Uuid,
    pub project_id: Uuid,
    pub kind: String,
    pub severity: String,
    pub title: String,
    pub body: String,
    pub rationale: String,
    pub evidence_json: serde_json::Value,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub acted_at: Option<DateTime<Utc>>,
    pub deferred_until: Option<DateTime<Utc>>,
}

pub fn content_hash(kind: &str, title: &str, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(kind.as_bytes()); h.update(b"|");
    h.update(title.as_bytes()); h.update(b"|");
    h.update(body.as_bytes());
    hex::encode(h.finalize())
}

/// Insert a recommendation. Returns `Ok(Some(id))` on insert, `Ok(None)` if
/// `content_hash` already present (dedupe).
pub fn insert(db: &Db, n: NewRec) -> Result<Option<Uuid>> {
    let hash = content_hash(&n.kind, &n.title, &n.body);
    let id = Uuid::new_v4();
    let now = Utc::now();
    let conn = db.pool().get()?;
    let rows = conn.execute(
        "INSERT OR IGNORE INTO recommendations
         (id, project_id, kind, severity, title, body, rationale, evidence_json,
          status, created_at, content_hash)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,?10)",
        params![
            id.to_string(), n.project_id.to_string(), n.kind, n.severity,
            n.title, n.body, n.rationale,
            serde_json::to_string(&n.evidence_json).map_err(|e| Error::Config(e.to_string()))?,
            now.to_rfc3339(), hash,
        ],
    )?;
    Ok(if rows == 0 { None } else { Some(id) })
}

pub fn list_by_status(db: &Db, project_id: Uuid, status: RecStatus) -> Result<Vec<Recommendation>> {
    let conn = db.pool().get()?;
    let mut stmt = conn.prepare(
        "SELECT id, project_id, kind, severity, title, body, rationale,
                evidence_json, status, created_at, acted_at, deferred_until
         FROM recommendations WHERE project_id=?1 AND status=?2
         ORDER BY created_at DESC")?;
    let rows = stmt.query_map(params![project_id.to_string(), status.as_str()], row_to_rec)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get(db: &Db, id: Uuid) -> Result<Recommendation> {
    let conn = db.pool().get()?;
    conn.query_row(
        "SELECT id, project_id, kind, severity, title, body, rationale,
                evidence_json, status, created_at, acted_at, deferred_until
         FROM recommendations WHERE id=?1",
        params![id.to_string()], row_to_rec,
    ).map_err(Error::Db)
}

pub fn set_status(db: &Db, id: Uuid, status: RecStatus) -> Result<()> {
    let conn = db.pool().get()?;
    let acted_at = if matches!(status, RecStatus::Acted) {
        Some(Utc::now().to_rfc3339())
    } else { None };
    conn.execute(
        "UPDATE recommendations SET status=?1, acted_at=COALESCE(?2, acted_at) WHERE id=?3",
        params![status.as_str(), acted_at, id.to_string()],
    )?;
    Ok(())
}

pub fn defer(db: &Db, id: Uuid, until: DateTime<Utc>) -> Result<()> {
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE recommendations SET status='deferred', deferred_until=?1 WHERE id=?2",
        params![until.to_rfc3339(), id.to_string()],
    )?;
    Ok(())
}

pub fn resurface_expired(db: &Db, now: DateTime<Utc>) -> Result<usize> {
    let conn = db.pool().get()?;
    let n = conn.execute(
        "UPDATE recommendations SET status='pending', deferred_until=NULL
         WHERE status='deferred' AND deferred_until IS NOT NULL AND deferred_until <= ?1",
        params![now.to_rfc3339()],
    )?;
    Ok(n)
}

/// Update rationale (used by Better-explanation Kimi response).
pub fn update_rationale(db: &Db, id: Uuid, rationale: &str) -> Result<()> {
    let conn = db.pool().get()?;
    conn.execute(
        "UPDATE recommendations SET rationale=?1 WHERE id=?2",
        params![rationale, id.to_string()],
    )?;
    Ok(())
}

fn row_to_rec(r: &rusqlite::Row) -> rusqlite::Result<Recommendation> {
    Ok(Recommendation {
        id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap(),
        project_id: Uuid::parse_str(&r.get::<_, String>(1)?).unwrap(),
        kind: r.get(2)?,
        severity: r.get(3)?,
        title: r.get(4)?,
        body: r.get(5)?,
        rationale: r.get(6)?,
        evidence_json: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or(serde_json::Value::Null),
        status: r.get(8)?,
        created_at: DateTime::parse_from_rfc3339(&r.get::<_, String>(9)?)
            .unwrap().with_timezone(&Utc),
        acted_at: r.get::<_, Option<String>>(10)?.and_then(|s|
            DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
        deferred_until: r.get::<_, Option<String>>(11)?.and_then(|s|
            DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&Utc))),
    })
}
