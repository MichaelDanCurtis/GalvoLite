//! Shared helpers for integration tests that need a live axum app + DB.
//!
//! Each helper returns a fresh `tempdir`-backed setup; tests should not share
//! state between runs.
#![allow(dead_code)]

use galvolite::db::{Db, NewProject, NewSession};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

pub struct TestApp {
    pub app: axum::Router,
    pub mgr: Arc<SessionManager>,
    pub db: Db,
    /// Held to keep the temp dir alive for the duration of the test.
    pub _dir: TempDir,
}

/// Build a fresh axum app + SessionManager backed by a tempdir SQLite.
pub fn test_app() -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let app = galvolite::api::router(mgr.clone());
    TestApp {
        app,
        mgr,
        db,
        _dir: dir,
    }
}

/// Create a project + session row directly in the DB (no PTY spawn) and
/// stamp it with `api_token`. Returns the session id and the token.
pub fn seed_session(db: &Db, project_dir: &std::path::Path) -> (Uuid, String) {
    std::fs::create_dir_all(project_dir).unwrap();
    let project = db
        .insert_project(NewProject {
            name: "test-proj".into(),
            path: project_dir.to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = db
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "test".into(),
            parent_session_id: None,
        })
        .unwrap();
    let token = format!("tok_{}", Uuid::new_v4().simple());
    db.set_api_token(session.id, &token).unwrap();
    (session.id, token)
}
