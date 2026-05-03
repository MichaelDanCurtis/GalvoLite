//! Plan 8 Task 13 — GET /api/projects/:id/recommendations
mod common;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;
use tower::ServiceExt;

fn init_git(d: &std::path::Path) {
    Command::new("git")
        .args(["-c", "init.defaultBranch=main", "init", "-q"])
        .current_dir(d)
        .status()
        .unwrap();
    std::fs::write(d.join("README"), "x").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(d)
        .status()
        .unwrap();
    Command::new("git")
        .args([
            "-c", "user.email=t@t",
            "-c", "user.name=t",
            "commit", "-qm", "init",
        ])
        .current_dir(d)
        .status()
        .unwrap();
}

#[tokio::test]
async fn list_returns_pending_recs_for_project() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "recs".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let id = galvolite::recommendations::insert(
        app.mgr.db(),
        galvolite::recommendations::NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "warn".into(),
            title: "t".into(),
            body: "b".into(),
            rationale: "r".into(),
            evidence_json: serde_json::json!([]),
        },
    )
    .unwrap()
    .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/projects/{}/recommendations", project.id))
        .body(Body::empty())
        .unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], id.to_string());
}

#[tokio::test]
async fn list_filters_by_status_query_param() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "recs2".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let id = galvolite::recommendations::insert(
        app.mgr.db(),
        galvolite::recommendations::NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "info".into(),
            title: "t".into(),
            body: "b".into(),
            rationale: "r".into(),
            evidence_json: serde_json::json!([]),
        },
    )
    .unwrap()
    .unwrap();
    galvolite::recommendations::set_status(
        app.mgr.db(),
        id,
        galvolite::recommendations::RecStatus::Acted,
    )
    .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/projects/{}/recommendations?status=acted",
            project.id
        ))
        .body(Body::empty())
        .unwrap();
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
}
