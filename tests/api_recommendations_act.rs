//! Plan 8 Task 14 — POST /api/projects/:id/recommendations/act
//!
//! Three behaviors covered:
//!   1. `make` appends a block (concat of all pending recs) to the
//!      orchestrator's `pending_overseer_recs` and flips status to `acted`.
//!   2. `make` with no orchestrator returns 409.
//!   3. `dismiss` flips status to `dismissed` without needing an orchestrator.
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
async fn make_appends_block_to_orchestrator_pending_overseer_recs() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "act".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Top-level session = orchestrator (per find_orchestrator_for_project
    // heuristic). Role must be a CHECK-valid value; "implementation" is fine.
    let orch = app
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

    let r1 = recommendations::insert(
        app.mgr.db(),
        NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "warn".into(),
            title: "Add tests".into(),
            body: "Body 1".into(),
            rationale: "Why 1".into(),
            evidence_json: serde_json::json!([]),
        },
    )
    .unwrap()
    .unwrap();
    let r2 = recommendations::insert(
        app.mgr.db(),
        NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "info".into(),
            title: "Refactor".into(),
            body: "Body 2".into(),
            rationale: "Why 2".into(),
            evidence_json: serde_json::json!([]),
        },
    )
    .unwrap()
    .unwrap();

    let resp = post_json(
        &app,
        &format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r1, r2], "action": "make"}),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = v["results"].as_array().expect("results array");
    assert_eq!(arr.len(), 2);

    let s = app.mgr.db().get_session(orch.id).unwrap();
    let block = s.pending_overseer_recs.expect("orch has pending block");
    assert!(block.contains("Add tests"), "block missing first title: {block}");
    assert!(block.contains("Refactor"), "block missing second title: {block}");

    let r1_row = recommendations::get(app.mgr.db(), r1).unwrap();
    assert_eq!(r1_row.status, "acted");
    let r2_row = recommendations::get(app.mgr.db(), r2).unwrap();
    assert_eq!(r2_row.status, "acted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn make_with_no_orchestrator_returns_409() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "act2".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    // Note: NO top-level session spawned.
    let r = recommendations::insert(
        app.mgr.db(),
        NewRec {
            project_id: project.id,
            kind: "k".into(),
            severity: "warn".into(),
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
        &format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r], "action": "make"}),
    )
    .await;
    assert_eq!(resp.status(), 409);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dismiss_marks_status_only() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "dismiss".into(),
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
        &format!("/api/projects/{}/recommendations/act", project.id),
        serde_json::json!({"rec_ids": [r], "action": "dismiss"}),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let bytes = to_bytes(resp.into_body(), 256 * 1024).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["dismissed"], 1);
    let row = recommendations::get(app.mgr.db(), r).unwrap();
    assert_eq!(row.status, "dismissed");
}
