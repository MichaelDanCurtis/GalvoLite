//! Contract tests for POST /hooks/ingest.
//!
//! Uses the same `tower::ServiceExt::oneshot` pattern as existing API tests
//! (see tests/api_integration.rs) — no live socket needed. The handler
//! receives a `Bytes` body so we can probe shape errors directly.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::hooks::types::{HookAction, HookIngestRequest, HookIngestResponse, HookKind};
use galvolite::session_manager::SessionManager;
use serde_json::json;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

struct App {
    router: axum::Router,
    db: Db,
}

fn setup() -> App {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    let _ = Box::leak(Box::new(dir));
    let mgr = Arc::new(SessionManager::new(db.clone(), data_dir));
    let router = galvolite::api::router(mgr);
    App { router, db }
}

fn seed_session(db: &Db) -> (uuid::Uuid, String) {
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
            role: "implementation".into(),
            branch: None,
            worktree_path: path,
            task_summary: "t".into(),
            parent_session_id: None,
        })
        .unwrap();
    let token = format!("tok-{}", uuid::Uuid::new_v4());
    {
        let conn = db.pool().get().unwrap();
        conn.execute(
            "UPDATE sessions SET api_token = ?1 WHERE id = ?2",
            rusqlite::params![token, s.id.to_string()],
        )
        .unwrap();
    }
    (s.id, token)
}

fn post_ingest(
    app: &axum::Router,
    token: Option<&str>,
    body: Vec<u8>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/hooks/ingest")
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("X-Galvolite-Token", t);
    }
    let _ = app;
    b.body(Body::from(body)).unwrap()
}

fn minimal_envelope(session_id: &str) -> Vec<u8> {
    serde_json::to_vec(&HookIngestRequest {
        session_id: session_id.into(),
        hook: HookKind::SessionStart,
        ts: "2026-04-18T00:00:00Z".into(),
        event_id: format!("evt-{}", uuid::Uuid::new_v4()),
        payload: json!({}),
    })
    .unwrap()
}

#[tokio::test]
async fn rejects_missing_token() {
    let app = setup();
    let body = minimal_envelope("00000000-0000-0000-0000-000000000000");
    let resp = app
        .router
        .clone()
        .oneshot(post_ingest(&app.router, None, body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_bad_token() {
    let app = setup();
    let body = minimal_envelope("00000000-0000-0000-0000-000000000000");
    let resp = app
        .router
        .clone()
        .oneshot(post_ingest(&app.router, Some("wrong"), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_unknown_session_token() {
    let app = setup();
    // No session seeded → no api_token rows → any token fails.
    let body = minimal_envelope("11111111-1111-1111-1111-111111111111");
    let resp = app
        .router
        .clone()
        .oneshot(post_ingest(&app.router, Some("any"), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn rejects_malformed_envelope() {
    let app = setup();
    let (_id, token) = seed_session(&app.db);
    let resp = app
        .router
        .clone()
        .oneshot(post_ingest(&app.router, Some(&token), b"{not json".to_vec()))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn accepts_valid_session_writes_event_returns_proceed() {
    let app = setup();
    let (id, token) = seed_session(&app.db);
    let body = serde_json::to_vec(&HookIngestRequest {
        session_id: id.to_string(),
        hook: HookKind::SessionStart,
        ts: "2026-04-18T00:00:00Z".into(),
        event_id: "evt-1".into(),
        payload: json!({"is_resume": false}),
    })
    .unwrap();
    let resp = app
        .router
        .clone()
        .oneshot(post_ingest(&app.router, Some(&token), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    let parsed: HookIngestResponse = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.action, HookAction::Proceed);

    // Event row landed.
    let conn = app.db.pool().get().unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'hook.SessionStart'",
            [id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn rejects_envelope_session_token_mismatch() {
    let app = setup();
    let (_, token_a) = seed_session(&app.db);
    let (id_b, _) = seed_session(&app.db);
    let body = serde_json::to_vec(&HookIngestRequest {
        session_id: id_b.to_string(),
        hook: HookKind::SessionStart,
        ts: "2026-04-18T00:00:00Z".into(),
        event_id: "evt-2".into(),
        payload: json!({}),
    })
    .unwrap();
    let resp = app
        .router
        .clone()
        .oneshot(post_ingest(&app.router, Some(&token_a), body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
