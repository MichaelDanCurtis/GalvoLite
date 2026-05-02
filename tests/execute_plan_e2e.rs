//! E2E test: POST /api/projects/:id/execute-plan spawns children for first phase.
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


#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn execute_plan_spawns_one_child_per_part_in_first_phase() {
    // Enable executor for this test process.
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");

    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());

    // Write plan + directive into .galvolite/
    let gv_dir = dir.path().join(".galvolite");
    std::fs::create_dir_all(&gv_dir).unwrap();
    std::fs::write(gv_dir.join("plan.md"), PLAN_MD).unwrap();
    std::fs::write(gv_dir.join("directive.md"), DIRECTIVE_MD).unwrap();

    let project = app
        .mgr
        .add_project(NewProject {
            name: "plan8-e2e".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn a top-level session (no parent) — this is what find_orchestrator_for_project
    // now selects. Role must be CHECK-constraint-valid; "orchestrator" is a runtime
    // profile, not a DB role value.
    let parent = app
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
    let orch_id = parent.id;

    // POST /api/projects/:id/execute-plan
    let req = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/projects/{}/execute-plan", project.id))
        .header("content-type", "application/json")
        .body(axum::body::Body::from("{}"))
        .unwrap();
    use tower::ServiceExt;
    let resp = app.app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::OK,
        "execute-plan should return 200"
    );

    // First phase only (p1, part 1) — autonomy=manual stops at boundary.
    // Children are sessions with parent_session_id = orch_id
    let kids: Vec<_> = app
        .mgr
        .list_sessions()
        .unwrap()
        .into_iter()
        .filter(|s| s.parent_session_id == Some(orch_id))
        .collect();
    assert_eq!(kids.len(), 1, "first phase has 1 part → 1 child");
    assert_eq!(kids[0].task_summary, "alpha");
}
