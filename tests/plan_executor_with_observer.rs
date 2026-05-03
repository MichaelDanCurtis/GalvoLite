//! Plan 8 Task 23 — plan executor + observer cross-integration test.
//!
//! Plan drift fix: plan uses fictional `spawn_test_app`, `create_project_with_plan_and_directive`,
//! `spawn_orchestrator`, `app.post`. Adapted to real helpers (`test_app_with_bus()` + inline git
//! setup + `add_project` + oneshot POST), mirroring `tests/execute_plan_e2e.rs`.
mod common;

use axum::body::Body;
use axum::http::Request;
use galvolite::api::events::DomainEvent;
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

const PLAN_MD: &str = "- [ ] alpha\n";
const DIRECTIVE_MD: &str = "\
# Directive: x
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | manual | — |
## Notes
";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn execute_plan_emits_phase_events_into_bus() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());

    let gv_dir = dir.path().join(".galvolite");
    std::fs::create_dir_all(&gv_dir).unwrap();
    std::fs::write(gv_dir.join("plan.md"), PLAN_MD).unwrap();
    std::fs::write(gv_dir.join("directive.md"), DIRECTIVE_MD).unwrap();

    let project = app
        .mgr
        .add_project(NewProject {
            name: "p8-t23".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    // Top-level session (orchestrator per heuristic).
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

    let mut rx = app.mgr.event_bus().subscribe();

    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/projects/{}/execute-plan", project.id))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let resp = app.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), 200);

    // Drain bus for up to 2s. Per phase reactor, executing first phase with
    // autonomy=manual should at minimum surface a phase event (PhaseAdvanced
    // when entering, or PhaseReviewNeeded when its parts complete). We accept
    // either as evidence the cross-integration is wired.
    let mut got_phase_event = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await {
            Ok(Ok(DomainEvent::PhaseAdvanced { phase_id, .. })) if phase_id == "p1" => {
                got_phase_event = true;
                break;
            }
            Ok(Ok(DomainEvent::PhaseReviewNeeded { phase_id, .. })) if phase_id == "p1" => {
                got_phase_event = true;
                break;
            }
            _ => {}
        }
    }
    assert!(got_phase_event, "no PhaseAdvanced or PhaseReviewNeeded event for p1");
}
