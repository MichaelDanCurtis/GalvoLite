//! Validation tests for POST /api/projects/:id/execute-plan error paths.
mod common;

use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;

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

const PLAN_MD: &str = "\
- [ ] alpha
- [ ] beta
- [ ] gamma
";

const DIRECTIVE_MD: &str = "\
# Directive: demo
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | manual | — |
| p2 | 2, 3 | manual | p1 |
## Notes
";

async fn post_execute(
    app: &common::TestApp,
    project_id: uuid::Uuid,
) -> axum::http::Response<axum::body::Body> {
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/projects/{}/execute-plan", project_id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from("{}"))
        .unwrap();
    app.app.clone().oneshot(req).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_plan_returns_400() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");

    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());

    // Write only directive — no plan.md
    let gv_dir = dir.path().join(".galvolite");
    std::fs::create_dir_all(&gv_dir).unwrap();
    std::fs::write(gv_dir.join("directive.md"), DIRECTIVE_MD).unwrap();

    let project = app
        .mgr
        .add_project(NewProject {
            name: "missing-plan".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn a top-level session so we don't hit NoOrchestrator before MissingArtifact.
    app.mgr
        .spawn_session(
            project.id,
            "root".into(),
            "implementation".into(),
            "/bin/cat".into(),
            vec![],
        )
        .await
        .unwrap();

    let resp = post_execute(&app, project.id).await;
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_orchestrator_returns_409() {
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
            name: "no-orch".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // No orchestrator inserted — should get 409.
    let resp = post_execute(&app, project.id).await;
    assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn double_execute_returns_409() {
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
            name: "double-exec".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn a top-level session — satisfies find_orchestrator_for_project heuristic.
    app.mgr
        .spawn_session(
            project.id,
            "root".into(),
            "implementation".into(),
            "/bin/cat".into(),
            vec![],
        )
        .await
        .unwrap();

    // First execute — should succeed (spawns a child via spawn_child + git worktree).
    let resp1 = post_execute(&app, project.id).await;
    assert_eq!(resp1.status(), axum::http::StatusCode::OK, "first execute should succeed");

    // Second execute — should fail 409 AlreadyRunning.
    let resp2 = post_execute(&app, project.id).await;
    assert_eq!(
        resp2.status(),
        axum::http::StatusCode::CONFLICT,
        "second execute should return 409"
    );
}
