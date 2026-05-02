mod common;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use chrono::Utc;
use galvolite::decisions::{record as record_decision, Decision};
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn list_decisions_returns_recorded_entries() {
    let app = common::test_app();
    let project = app.mgr.add_project(galvolite::db::NewProject {
        name: "p".into(),
        path: app._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    record_decision(app._dir.path(), &Decision {
        session_id: Uuid::new_v4(),
        asked_at: Utc::now(),
        question: "Use SQLite?".into(),
        answer: "Yes".into(),
        choices: None,
    }).unwrap();

    let resp = app.router.clone().oneshot(
        Request::builder()
            .uri(format!("/api/projects/{}/decisions", project.id))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["question"].as_str().unwrap(), "Use SQLite?");
    assert_eq!(arr[0]["answer"].as_str().unwrap(), "Yes");
}

#[tokio::test]
async fn list_decisions_returns_empty_when_no_dir() {
    let app = common::test_app();
    let project = app.mgr.add_project(galvolite::db::NewProject {
        name: "p".into(),
        path: app._dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();
    let resp = app.router.clone().oneshot(
        Request::builder()
            .uri(format!("/api/projects/{}/decisions", project.id))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v.as_array().unwrap().is_empty());
}
