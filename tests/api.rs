use axum::body::Body;
use axum::http::{Request, StatusCode};
use galvolite::db::Db;
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

fn setup() -> axum::Router {
    let dir = tempdir().unwrap();
    let data_dir = dir.path().to_path_buf();
    let db = Db::open(data_dir.join("test.sqlite")).unwrap();
    // Keep tempdir alive for the life of the process; test-only leak.
    let _ = Box::leak(Box::new(dir));
    let mgr = Arc::new(SessionManager::new(db, data_dir));
    galvolite::api::router(mgr)
}

#[tokio::test]
async fn list_projects_empty() {
    let app = setup();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(projects.is_empty());
}

#[tokio::test]
async fn create_and_list_project() {
    let app = setup();

    // Create project
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "name": "TestProject",
                        "path": "/tmp/test-project"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(project["name"], "TestProject");

    // List projects
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/projects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "TestProject");
}

#[tokio::test]
async fn not_found_session_returns_404() {
    let app = setup();
    let id = uuid::Uuid::new_v4();
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // `Db::get_session` returns `Error::SessionNotFound` for a missing row,
    // which `err_to_response` maps to 404. Body check distinguishes
    // handler-404 from axum's fallback-404 (empty body) — guards against
    // route-syntax regressions.
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(
        text.contains("session not found"),
        "expected handler-404 body, got: {text:?}"
    );
}

#[tokio::test]
async fn internal_error_body_is_opaque() {
    let app = setup();
    // Create a project, then POST the same path again. `path` has a UNIQUE
    // constraint in the schema, so the second insert returns
    // `rusqlite::Error` (SqliteFailure) -> `Error::Db(...)` -> the 500
    // catchall in `err_to_response`. The body must be opaque (not the raw
    // SQL error, which would leak "UNIQUE constraint failed: projects.path").
    let body_json = serde_json::to_string(&serde_json::json!({
        "name": "dup",
        "path": "/tmp/dup-project-opaque",
    }))
    .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(body_json.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("content-type", "application/json")
                .body(Body::from(body_json))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert_eq!(text, "internal server error");
    // Must not leak DB/SQL internals
    assert!(!text.to_lowercase().contains("sql"));
    assert!(!text.to_lowercase().contains("rusqlite"));
    assert!(!text.to_lowercase().contains("unique"));
}

#[tokio::test]
async fn kill_nonexistent_session_returns_404_or_409() {
    let app = setup();
    let id = uuid::Uuid::new_v4();
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Observed: 404 NOT FOUND.
    // `SessionManager::kill_session` on an unknown id reaches
    // `db.end_session_if_unended`, which does `UPDATE ... WHERE id = ?` with
    // zero affected rows and returns `Error::SessionNotFound`. `err_to_response`
    // maps that to 404. If the row existed but was already ended,
    // kill_session would return `SessionNotRunning` → 409, so we accept both.
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::CONFLICT,
        "unexpected status: {}",
        resp.status()
    );
}
