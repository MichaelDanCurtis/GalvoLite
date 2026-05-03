//! Plan 8 Task 24 — Overseer end-to-end with mocked Kimi.
//!
//! Plan drift fix: real helpers from `tests/common/mod.rs` (no
//! `spawn_test_app`/`create_project_in_temp_repo`/`spawn_orchestrator`).
mod common;

use axum::body::Body;
use axum::http::Request;
use galvolite::db::NewProject;
use galvolite::overseer::observer::{ContextSnapshot, EventRecord};
use galvolite::overseer::recommender::{review, OverseerState};
use galvolite::recommendations::{self, RecStatus};
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overseer_review_with_mock_kimi_inserts_rec_then_make_appends_to_orch() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());

    let project = app
        .mgr
        .add_project(NewProject {
            name: "e2e".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
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

    // Stage a snapshot with one event.
    let mut snap = ContextSnapshot::default();
    snap.events.push(EventRecord {
        session_id: orch.id,
        kind: "mcp.status_report".into(),
        source: "mcp".into(),
        ts: "t".into(),
        payload: serde_json::json!({"kind":"stuck"}),
    });

    let kimi_text = r#"{"recommendations":[
        {"kind":"stuck","severity":"warn","title":"Session stuck","body":"investigate","rationale":"no events","evidence_refs":[]}
    ]}"#;

    let mut state = OverseerState::new(100_000);
    let bus = app.mgr.event_bus().clone();
    review(
        app.mgr.db(),
        &bus,
        project.id,
        &snap,
        &mut state,
        |_, _| async move {
            Ok::<_, galvolite::overseer::kimi::KimiError>((kimi_text.to_string(), 50))
        },
    )
    .await
    .unwrap();

    let pending =
        recommendations::list_by_status(app.mgr.db(), project.id, RecStatus::Pending).unwrap();
    assert_eq!(pending.len(), 1);

    // Operator clicks Make via API.
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/projects/{}/recommendations/act", project.id))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "rec_ids": [pending[0].id], "action": "make"
            }))
            .unwrap(),
        ))
        .unwrap();
    let resp = app.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    let s = app.mgr.db().get_session(orch.id).unwrap();
    assert!(s
        .pending_overseer_recs
        .unwrap()
        .contains("Session stuck"));
}
