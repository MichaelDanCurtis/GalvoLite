//! Validation tests for POST /api/projects/:id/execute-plan error paths.
mod common;

use galvolite::db::NewProject;
use std::process::Command;
use tempfile::TempDir;
use uuid::Uuid;

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

/// Insert an orchestrator session directly, bypassing the role CHECK constraint.
fn insert_orchestrator(db: &galvolite::db::Db, project_id: Uuid, worktree_path: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.pool().get().unwrap();
    conn.execute(
        "INSERT INTO sessions (id, project_id, role, branch, worktree_path, state, autopilot, task_summary, created_at)
         VALUES (?1, ?2, 'implementation', NULL, ?3, 'idle', 0, 'orchestrator task', ?4)",
        rusqlite::params![id.to_string(), project_id.to_string(), worktree_path, now],
    ).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;").ok();
    conn.execute(
        "UPDATE sessions SET role = 'orchestrator' WHERE id = ?1",
        rusqlite::params![id.to_string()],
    ).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints = OFF;").ok();
    id
}

async fn post_execute(
    app: &common::TestApp,
    project_id: Uuid,
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

    // Insert orchestrator so we don't hit NoOrchestrator before MissingArtifact.
    insert_orchestrator(&app.db, project.id, &dir.path().to_string_lossy());

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

    insert_orchestrator(&app.db, project.id, &dir.path().to_string_lossy());

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
