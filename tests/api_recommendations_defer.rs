//! Plan 8 Task 15 — POST /api/projects/:id/recommendations/defer
//!
//! Two behaviors covered:
//!   1. Default 24h defer sets `status='deferred'` and a non-null
//!      `deferred_until` timestamp.
//!   2. Explicit `hours` overrides the default; the resulting
//!      `deferred_until` lies within a tight window of the requested offset.
mod common;

use axum::body::{to_bytes, Body};
use axum::http::Request;
use galvolite::db::NewProject;
use galvolite::recommendations::{self, NewRec};
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

async fn post_json(
    app: &common::TestApp,
    uri: &str,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    app.app.clone().oneshot(req).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn defer_default_24h_sets_deferred_until() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "defer".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let r = recommendations::insert(
        app.mgr.db(),
        NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "info".into(),
            title: "T".into(),
            body: "B".into(),
            rationale: "R".into(),
            evidence_json: serde_json::json!([]),
        },
    )
    .unwrap()
    .unwrap();
    let resp = post_json(
        &app,
        &format!("/api/projects/{}/recommendations/defer", project.id),
        serde_json::json!({"rec_id": r}),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    assert_eq!(row.status, "deferred");
    assert!(row.deferred_until.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn defer_with_explicit_hours_overrides_default() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "defer2".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    let r = recommendations::insert(
        app.mgr.db(),
        NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "info".into(),
            title: "T".into(),
            body: "B".into(),
            rationale: "R".into(),
            evidence_json: serde_json::json!([]),
        },
    )
    .unwrap()
    .unwrap();
    let resp = post_json(
        &app,
        &format!("/api/projects/{}/recommendations/defer", project.id),
        serde_json::json!({"rec_id": r, "hours": 1}),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let _ = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    let until = row.deferred_until.unwrap();
    let now = chrono::Utc::now();
    let delta = (until - now).num_minutes();
    assert!(
        (50..=70).contains(&delta),
        "expected ~60min, got {delta}"
    );
}
