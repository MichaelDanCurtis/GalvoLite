use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use galvolite::api::router;
use galvolite::db::{Db, NewProject};
use galvolite::session_manager::SessionManager;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

async fn setup() -> (axum::Router, Arc<SessionManager>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Db::open(dir.path().join("t.db")).unwrap();
    let mgr = Arc::new(SessionManager::new(db, dir.path().to_path_buf()));
    (router(mgr.clone()), mgr, dir)
}

#[tokio::test]
async fn put_autopilot_true_flips_flag() {
    let (app, mgr, _dir) = setup().await;
    let project = mgr
        .add_project(NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let session = mgr
        .spawn_session(project.id, "t".into(), "implementation".into(), "echo".into(), vec![])
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/sessions/{}/autopilot", session.id))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = to_bytes(resp.into_body(), 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["autopilot"], true);
    assert!(mgr.get_session(session.id).unwrap().autopilot);
}

#[tokio::test]
async fn put_autopilot_unknown_session_404() {
    let (app, _mgr, _dir) = setup().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/sessions/{}/autopilot", uuid::Uuid::new_v4()))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"enabled":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
