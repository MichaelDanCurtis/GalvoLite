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
    /// Alias of `router` — Sub-plan 3 mcp_* tests use `setup.app`.
    pub app: axum::Router,
    pub db: Db,
    /// Exposed so Sub-plan 4 tests can call `mgr.subscribe_events()` to assert
    /// on the domain-event broadcaster. The router already holds its own
    /// `Arc<SessionManager>`; this is the same Arc, cloned.
    pub mgr: Arc<SessionManager>,
    pub _dir: TempDir,
}

pub fn test_app() -> TestApp {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let router = galvolite::api::router(mgr.clone());
    TestApp {
        app: router.clone(),
        router,
        db,
        mgr,
        _dir: dir,
    }
}

/// Like [`test_app`] but constructs the `SessionManager` with an explicit
/// `EventBroadcaster` so tests can assert on `mgr.subscribe_events()`. Used
/// by Sub-plan 5 plan_step / recovery tests; will be the only constructor
/// once Task 9 lands.
pub fn test_app_with_bus() -> TestApp {
    let dir = TempDir::new().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let bus = galvolite::api::events::EventBroadcaster::new(64);
    let mgr = Arc::new(SessionManager::with_event_bus(
        db.clone(),
        data_dir,
        bus,
    ));
    let router = galvolite::api::router(mgr.clone());
    TestApp {
        app: router.clone(),
        router,
        db,
        mgr,
        _dir: dir,
    }
}

/// Bind an ephemeral 127.0.0.1 listener for tests that need a real HTTP/WS
/// surface; spawn the server and return the bound port. Mirrors what each
/// WS test was hand-rolling before.
pub async fn serve_ephemeral(app: axum::Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    port
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

// ---------------------------------------------------------------------------
// Sub-plan 3 (MCP surface) helpers — merged in during the rebase. Keep names
// distinct from the hooks-side helpers above when signatures differ.
// ---------------------------------------------------------------------------

/// Force a session into a specific state by direct UPDATE (Sub-plan 3 alias
/// of `set_state` above; both call the same SQL).
pub fn force_state(db: &Db, sid: uuid::Uuid, state: SessionState) {
    set_state(db, sid, state);
}

/// Create a project + session row at a specific path and stamp it with an
/// `api_token`. Returns the session id and the token.
pub fn seed_session(db: &Db, project_dir: &std::path::Path) -> (uuid::Uuid, String) {
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
    let token = format!("tok_{}", uuid::Uuid::new_v4().simple());
    db.set_api_token(session.id, &token).unwrap();
    (session.id, token)
}

/// Same as [`seed_session`] but additionally forces the session into the
/// given state via direct UPDATE. Distinct from `seed_session_in_state`
/// (which generates its own path).
pub fn seed_session_in_state_at(
    db: &Db,
    project_dir: &std::path::Path,
    state: SessionState,
) -> (uuid::Uuid, String) {
    let (sid, token) = seed_session(db, project_dir);
    force_state(db, sid, state);
    (sid, token)
}

/// Insert a batch of `plan_steps` rows for a session. Rows are written with
/// DB-native status values (`pending | in_progress | done | blocked`) — the
/// API helper is responsible for mapping `in_progress` → `working` on the way
/// out, so tests assert against the DB-native shape here.
///
/// Tuple shape: `(step_hash, ordinal, title, status, note)`.
pub fn seed_plan_steps(
    db: &Db,
    session_id: uuid::Uuid,
    rows: &[(&str, i64, &str, &str, Option<&str>)],
) {
    let conn = db.pool().get().unwrap();
    for (hash, ordinal, title, status, note) in rows {
        conn.execute(
            "INSERT INTO plan_steps (session_id, step_hash, ordinal, title, status, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                session_id.to_string(),
                hash,
                ordinal,
                title,
                status,
                note,
            ],
        )
        .unwrap();
    }
}

/// Post a pre-built JSON-RPC envelope to `/mcp` with the given bearer token
/// and return the parsed JSON response. Sub-plan 4 tests use this when they
/// need to hand-craft the envelope (e.g. to spawn the request on one task
/// and fulfill it from another).
pub async fn rpc(app: axum::Router, token: &str, body: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
