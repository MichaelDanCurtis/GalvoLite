use axum::body::Body;
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
    let app = router(mgr.clone());
    (app, mgr, dir)
}

#[tokio::test]
async fn delete_project_returns_204_and_purges() {
    let (app, mgr, _dir) = setup().await;
    let project = mgr
        .add_project(NewProject {
            name: "p".into(),
            path: tempfile::tempdir().unwrap().path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects/{}", project.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(mgr.get_project(project.id).is_err());
}

#[tokio::test]
async fn delete_project_unknown_returns_404() {
    let (app, _mgr, _dir) = setup().await;
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/projects/{}", uuid::Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
