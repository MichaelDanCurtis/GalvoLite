//! Sub-plan 7 Task 13: GET /api/projects/:id/plan endpoint.

mod common;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn returns_404_when_no_plan_md() {
    let app = common::test_app();
    let project = app.mgr.add_project(galvolite::db::NewProject {
        name: "p".into(),
        path: app._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let resp = app.router.clone().oneshot(
        Request::builder()
            .uri(format!("/api/projects/{}/plan", project.id))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn returns_markdown_when_plan_md_present() {
    let app = common::test_app();
    let project = app.mgr.add_project(galvolite::db::NewProject {
        name: "p".into(),
        path: app._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let dir = app._dir.path().join(".galvolite");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("plan.md"), "## Parts\n- 1\n").unwrap();

    let resp = app.router.clone().oneshot(
        Request::builder()
            .uri(format!("/api/projects/{}/plan", project.id))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["markdown"].as_str().unwrap(), "## Parts\n- 1\n");
}
