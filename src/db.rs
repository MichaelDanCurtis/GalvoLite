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
