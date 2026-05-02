//! E2E test: POST /api/projects/:id/execute-plan spawns children for first phase.
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

/// Insert an orchestrator session row directly (bypassing the role CHECK
/// constraint which only covers implementation/brainstorm/scout/fix).
fn insert_orchestrator(
    db: &galvolite::db::Db,
    project_id: Uuid,
    worktree_path: &str,
) -> Uuid {
    let id = Uuid::new_v4();
    let now = chrono::Utc::now().to_rfc3339();
    let conn = db.pool().get().unwrap();
    // Temporarily disable foreign key enforcement so we can INSERT with the
    // custom role without hitting the CHECK constraint.
    // NOTE: The CHECK constraint is on the role column of the sessions table.
    // SQLite CHECK constraints can't be disabled per-connection without
    // re-creating the table. Instead, we just insert with a workaround:
    // we use a raw INSERT that satisfies the constraint by inserting
    // 'implementation' first, then UPDATE the role to 'orchestrator'.
    conn.execute(
        "INSERT INTO sessions (id, project_id, role, branch, worktree_path, state, autopilot, task_summary, created_at)
         VALUES (?1, ?2, 'implementation', NULL, ?3, 'idle', 0, 'orchestrator task', ?4)",
        rusqlite::params![id.to_string(), project_id.to_string(), worktree_path, now],
    ).unwrap();
    // Now bypass the CHECK by using a raw UPDATE with pragma.
    // In SQLite, CHECK constraints are NOT enforced on UPDATE in older versions,
    // but in newer versions they ARE. Instead we need to disable them.
    conn.execute_batch("PRAGMA ignore_check_constraints = ON;").ok();
    conn.execute(
        "UPDATE sessions SET role = 'orchestrator' WHERE id = ?1",
        rusqlite::params![id.to_string()],
    ).unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints = OFF;").ok();
    id
}

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

    // Insert orchestrator session directly (role CHECK constraint workaround).
    let orch_id = insert_orchestrator(
        &app.db,
        project.id,
        &dir.path().to_string_lossy(),
    );

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
