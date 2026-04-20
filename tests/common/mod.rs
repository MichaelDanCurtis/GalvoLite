//! Shared helpers for hooks integration tests (Tasks 7–9).
//!
//! Spins up the axum router with a fresh DB, exposes seed/post helpers, and
//! provides state/counter inspectors that read directly from sqlite.

#![allow(dead_code)] // each test file uses a different subset of helpers

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use galvolite::db::{Db, NewProject, NewSession, SessionState as DbSessionState};
use galvolite::hooks::types::{HookIngestRequest, HookIngestResponse, HookKind};
use galvolite::session_manager::SessionManager;
use galvolite::state::SessionState;
use serde_json::Value;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

pub struct TestApp {
    pub router: axum::Router,
    pub db: Db,
    _tmp: TempDir,
}

pub fn test_app() -> TestApp {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let router = galvolite::api::router(mgr);
    TestApp {
        router,
        db,
        _tmp: tmp,
    }
}

pub fn seed_session_with_role(db: &Db, role: &str) -> (uuid::Uuid, String) {
    let path = format!("/tmp/galvolite-test-{}", uuid::Uuid::new_v4());
    let proj = db
        .insert_project(NewProject {
            name: "p".into(),
            path: path.clone(),
            remote_url: None,
        })
        .unwrap();
    let s = db
        .insert_session(NewSession {
            project_id: proj.id,
            role: role.into(),
            branch: None,
            worktree_path: path,
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();
    let token = format!("tok-{}", uuid::Uuid::new_v4());
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET api_token = ?1 WHERE id = ?2",
        rusqlite::params![token, s.id.to_string()],
    )
    .unwrap();
    (s.id, token)
}

pub fn seed_session_in_state(db: &Db, state: SessionState) -> (uuid::Uuid, String) {
    let (id, token) = seed_session_with_role(db, "implementation");
    set_state(db, id, state);
    (id, token)
}

pub fn set_state(db: &Db, id: uuid::Uuid, state: SessionState) {
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET state = ?1 WHERE id = ?2",
        rusqlite::params![state.to_string(), id.to_string()],
    )
    .unwrap();
}

pub fn state_of(db: &Db, id: uuid::Uuid) -> SessionState {
    let conn = db.pool().get().unwrap();
    let s: String = conn
        .query_row(
            "SELECT state FROM sessions WHERE id = ?1",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    s.parse().unwrap()
}

pub fn counters_of(db: &Db, id: uuid::Uuid) -> (i64, i64) {
    let conn = db.pool().get().unwrap();
    conn.query_row(
        "SELECT turn_count, est_tokens FROM sessions WHERE id = ?1",
        [id.to_string()],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .unwrap()
}

pub fn bump_tokens(db: &Db, id: uuid::Uuid, by: i64) {
    let conn = db.pool().get().unwrap();
    conn.execute(
        "UPDATE sessions SET est_tokens = COALESCE(est_tokens,0) + ?1 WHERE id = ?2",
        rusqlite::params![by, id.to_string()],
    )
    .unwrap();
}

pub async fn record_event(db: &Db, id: uuid::Uuid, kind: &str, payload: Value) {
    galvolite::state::apply_event(
        db,
        id,
        galvolite::state::EventInput {
            source: galvolite::state::Source::System,
            kind: kind.into(),
            payload,
            dedup_key: Some(format!("test-{kind}-{}", uuid::Uuid::new_v4())),
        },
    )
    .unwrap();
}

pub async fn post_hook(
    app: &TestApp,
    token: &str,
    session_id: &uuid::Uuid,
    hook: HookKind,
    payload: Value,
) -> HookIngestResponse {
    let body = serde_json::to_vec(&HookIngestRequest {
        session_id: session_id.to_string(),
        hook,
        ts: "2026-04-18T00:00:00Z".into(),
        event_id: format!("evt-{}", uuid::Uuid::new_v4()),
        payload,
    })
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/hooks/ingest")
        .header("content-type", "application/json")
        .header("X-Galvolite-Token", token)
        .body(Body::from(body))
        .unwrap();

    let resp = app.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    if status != StatusCode::OK {
        panic!(
            "hooks/ingest returned {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
    }
    serde_json::from_slice(&bytes).unwrap()
}

// Re-export the DB session-state alias so callers can compare against the
// state column directly when they need to (some tests want that lower-level
// view).
pub type DbState = DbSessionState;
