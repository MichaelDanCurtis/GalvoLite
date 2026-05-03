//! Plan 8 Task 22 — full lifecycle integration test.
//!
//! Plan drift fix: plan uses fictional `spawn_test_app`/`create_project_in_temp_repo`/
//! `spawn_orchestrator`/`app.post`. Real helpers: `test_app_with_bus()` +
//! `add_project` + `spawn_session` + inline oneshot POST helper (see
//! `tests/api_recommendations_act.rs`).
mod common;

use axum::body::Body;
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
    Command::new("git").args(["add", "."]).current_dir(d).status().unwrap();
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
async fn full_lifecycle_pending_acted_deferred_dismissed_resurfaced() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "lc".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    // Top-level session acts as orchestrator per find_orchestrator_for_project.
    let _orch = app
        .mgr
        .spawn_session(
            project.id,
            "root".into(),
            "implementation".into(),
            "/bin/cat".into(),
            vec![],
        )
        .await
        .unwrap();

    // Insert + Make.
    let r = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k".into(), severity: "warn".into(),
        title: "T".into(), body: "B".into(), rationale: "R".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    let r1 = post_json(
        &app,
        &format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r], "action": "make"}),
    ).await;
    assert_eq!(r1.status(), 200);
    assert_eq!(recommendations::get(app.mgr.db(), r).unwrap().status, "acted");

    // Insert another, defer 0h.
    let r2 = recommendations::insert(app.mgr.db(), NewRec {
        project_id: project.id, kind: "k2".into(), severity: "info".into(),
        title: "T2".into(), body: "B2".into(), rationale: "R2".into(),
        evidence_json: serde_json::json!([]),
    }).unwrap().unwrap();
    post_json(
        &app,
        &format!("/api/projects/{}/recommendations/defer", project.id),
        serde_json::json!({"rec_id": r2, "hours": 0}),
    ).await;

    // Sweep — 0h-deferred resurfaces immediately.
    let n = recommendations::resurface_expired(
        app.mgr.db(),
        chrono::Utc::now() + chrono::Duration::seconds(1),
    ).unwrap();
    assert_eq!(n, 1);
    assert_eq!(recommendations::get(app.mgr.db(), r2).unwrap().status, "pending");

    // Dismiss it.
    post_json(
        &app,
        &format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r2], "action": "dismiss"}),
    ).await;
    assert_eq!(recommendations::get(app.mgr.db(), r2).unwrap().status, "dismissed");
}
