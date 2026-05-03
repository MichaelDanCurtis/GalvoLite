//! Phase reactor tests: advance on rejoin (autonomy=full) + approval endpoint.
//! Follows the verbose style of execute_plan_e2e.rs — no fictional helpers.
mod common;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
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
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"])
        .current_dir(d)
        .status()
        .unwrap();
}

/// Two-phase plan: p1 (1 part, full) → p2 (1 part, full).
const PLAN_2P_FULL: &str = "- [ ] one\n- [ ] two\n";

const DIRECTIVE_2P_FULL: &str = "\
# Directive: reactor-test
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | full | — |
| p2 | 2 | full | p1 |
## Notes
";

/// Two-phase plan: p1 (full) → p2 (review).
const DIRECTIVE_2P_REVIEW: &str = "\
# Directive: reactor-test-review
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | full | — |
| p2 | 2 | review | p1 |
## Notes
";

/// POST /api/projects/:id/execute-plan — returns the response.
async fn post_execute(
    app: &common::TestApp,
    project_id: uuid::Uuid,
) -> axum::http::Response<Body> {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/projects/{}/execute-plan", project_id))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    app.app.clone().oneshot(req).await.unwrap()
}

/// POST /api/projects/:id/phases/:phase_id/approve — returns the response.
async fn post_approve(
    app: &common::TestApp,
    project_id: uuid::Uuid,
    phase_id: &str,
) -> axum::http::Response<Body> {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/projects/{}/phases/{}/approve", project_id, phase_id))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    app.app.clone().oneshot(req).await.unwrap()
}

/// Helper: list child sessions of `orch_id`.
fn list_children(app: &common::TestApp, orch_id: uuid::Uuid) -> Vec<galvolite::db::Session> {
    app.mgr
        .list_sessions()
        .unwrap()
        .into_iter()
        .filter(|s| s.parent_session_id == Some(orch_id))
        .collect()
}

/// Helper: mark a child session as terminal ("complete") via on_child_terminal.
async fn complete_child(app: &common::TestApp, child_id: uuid::Uuid) {
    app.mgr
        .on_child_terminal(child_id, "complete")
        .await
        .unwrap();
}

/// Helper: wait (poll) until `session.children_resolved` has been processed.
/// We detect this by watching `pending_rejoin` being set on the orchestrator
/// row, then give the reactor task additional time to finish `advance_next_phase`
/// (which itself spawns children asynchronously).
///
/// For the `review` case the reactor deliberately does NOT advance, so we just
/// need to confirm children_resolved fired (pending_rejoin set). For the `full`
/// case we give extra time after that for the reactor's advance to complete.
async fn wait_for_children_resolved(app: &common::TestApp, orch_id: uuid::Uuid) {
    for _ in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let conn = app.db.pool().get().unwrap();
        let pending: Option<String> = conn
            .query_row(
                "SELECT pending_rejoin FROM sessions WHERE id = ?1",
                [orch_id.to_string()],
                |r| r.get(0),
            )
            .unwrap_or(None);
        if pending.is_some() {
            return;
        }
    }
    // Timed out — test will assert below.
}

/// Helper: wait until the orchestrator has at least `min_children` children.
/// Used after full-autonomy advance to confirm the reactor actually spawned
/// the next phase's children.
async fn wait_for_child_count(app: &common::TestApp, orch_id: uuid::Uuid, min_children: usize) {
    for _ in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
        let count = list_children(app, orch_id).len();
        if count >= min_children {
            return;
        }
    }
    // Timed out — test will assert below.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_autonomy_advances_on_rejoin() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");

    let app = common::test_app_with_bus();
    // Start the phase reactor for this test's SessionManager.
    galvolite::plan_executor::start_phase_reactor(std::sync::Arc::clone(&app.mgr));

    let dir = TempDir::new().unwrap();
    init_git(dir.path());

    let gv_dir = dir.path().join(".galvolite");
    std::fs::create_dir_all(&gv_dir).unwrap();
    std::fs::write(gv_dir.join("plan.md"), PLAN_2P_FULL).unwrap();
    std::fs::write(gv_dir.join("directive.md"), DIRECTIVE_2P_FULL).unwrap();

    let project = app
        .mgr
        .add_project(NewProject {
            name: "reactor-full".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn a top-level session (the "orchestrator" heuristic: no parent, not ended).
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

    // Execute plan — spawns p1 children.
    let resp = post_execute(&app, project.id).await;
    assert_eq!(resp.status(), StatusCode::OK, "execute-plan must return 200");

    // Verify p1 spawned exactly 1 child (part 1 = "one").
    let kids_p1 = list_children(&app, orch.id);
    assert_eq!(kids_p1.len(), 1, "p1 has 1 part → 1 child; got {:?}", kids_p1.len());
    assert_eq!(kids_p1[0].task_summary, "one");

    // Complete p1's child — triggers on_child_terminal → session.children_resolved.
    complete_child(&app, kids_p1[0].id).await;

    // Wait for children_resolved to fire, then wait for the reactor to finish
    // spawning p2 (advance_next_phase is async; give it time to complete).
    wait_for_children_resolved(&app, orch.id).await;
    wait_for_child_count(&app, orch.id, 2).await;

    // p2 must now be spawned (part 2 = "two").
    let all_kids = list_children(&app, orch.id);
    let titles: Vec<_> = all_kids.iter().map(|s| s.task_summary.as_str()).collect();
    assert!(
        titles.contains(&"two"),
        "p2 should have been spawned automatically after p1 resolved; got: {titles:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_autonomy_blocks_until_approve() {
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "1");

    let app = common::test_app_with_bus();
    // Start the phase reactor for this test's SessionManager.
    galvolite::plan_executor::start_phase_reactor(std::sync::Arc::clone(&app.mgr));

    let dir = TempDir::new().unwrap();
    init_git(dir.path());

    let gv_dir = dir.path().join(".galvolite");
    std::fs::create_dir_all(&gv_dir).unwrap();
    std::fs::write(gv_dir.join("plan.md"), PLAN_2P_FULL).unwrap();
    std::fs::write(gv_dir.join("directive.md"), DIRECTIVE_2P_REVIEW).unwrap();

    let project = app
        .mgr
        .add_project(NewProject {
            name: "reactor-review".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Spawn orchestrator (top-level, no parent).
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

    // Execute plan — spawns p1 child.
    let resp = post_execute(&app, project.id).await;
    assert_eq!(resp.status(), StatusCode::OK, "execute-plan must return 200");

    let kids_p1 = list_children(&app, orch.id);
    assert_eq!(kids_p1.len(), 1, "p1 has 1 part → 1 child");

    // Complete p1's child.
    complete_child(&app, kids_p1[0].id).await;

    // Wait for the children_resolved event + reactor pass.
    wait_for_children_resolved(&app, orch.id).await;

    // p2 must NOT have auto-spawned — autonomy=review blocks until approve.
    let kids_after = list_children(&app, orch.id);
    assert_eq!(
        kids_after.len(),
        1,
        "p2 must not auto-spawn under review autonomy; got {} children",
        kids_after.len()
    );

    // Approve p2.
    let approve_resp = post_approve(&app, project.id, "p2").await;
    let status = approve_resp.status();
    let body_bytes = to_bytes(approve_resp.into_body(), 4096).await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "approve should return 200; body: {}",
        String::from_utf8_lossy(&body_bytes)
    );

    // p2 child must now be spawned.
    let kids_final = list_children(&app, orch.id);
    assert_eq!(
        kids_final.len(),
        2,
        "p2 child should be spawned after approve; got {} children",
        kids_final.len()
    );
    let final_titles: Vec<_> = kids_final.iter().map(|s| s.task_summary.as_str()).collect();
    assert!(
        final_titles.contains(&"two"),
        "p2 part title should be 'two'; got: {final_titles:?}"
    );
}
