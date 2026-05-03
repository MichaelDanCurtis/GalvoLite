//! Plan 8 Task 17 — UserPromptSubmit drains `pending_overseer_recs` into
//! `inject_text` for orchestrator sessions and clears the field.
//!
//! "Orchestrator" is identified via the top-level-session heuristic
//! (`parent_session_id IS NULL` + most-recent non-terminal). The DB CHECK
//! constraint forbids storing "orchestrator" as a `role`; both sessions in
//! the second test use `role="implementation"` and rely on parentage to
//! determine which session is the orchestrator.

mod common;

use galvolite::db::{NewProject, NewSession};
use galvolite::hooks::types::HookKind;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn user_prompt_submit_for_orch_drains_pending_overseer_recs_into_inject_text() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "orch-drain".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Top-level session: the heuristic in find_orchestrator_for_project
    // selects the most-recent non-terminal session with parent_session_id
    // IS NULL. Insert directly so we don't have to spawn a real PTY.
    let orch = app
        .mgr
        .db()
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(), // "orchestrator" is forbidden by DB CHECK
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "root".into(),
            parent_session_id: None,
        })
        .unwrap();
    let token = format!("tok_{}", uuid::Uuid::new_v4().simple());
    app.mgr.db().set_api_token(orch.id, &token).unwrap();

    app.mgr
        .append_pending_overseer_recs(
            orch.id,
            "## Apply these 1 recommendation(s):\n\n### Rec 1: T\n\nB",
        )
        .unwrap();

    let resp = common::post_hook(
        &app,
        &token,
        &orch.id,
        HookKind::UserPromptSubmit,
        serde_json::json!({"est_tokens": 0}),
    )
    .await;

    let inject = resp.inject_text.as_deref().unwrap_or("");
    assert!(
        inject.contains("Apply these 1 recommendation"),
        "inject_text should carry the pending overseer recs block, got: {inject:?}"
    );

    let s = app.mgr.db().get_session(orch.id).unwrap();
    assert!(
        s.pending_overseer_recs.is_none(),
        "pending_overseer_recs should be cleared after drain, got: {:?}",
        s.pending_overseer_recs
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn non_orch_session_is_not_affected() {
    let app = common::test_app_with_bus();
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let project = app
        .mgr
        .add_project(NewProject {
            name: "impl".into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    // Parent (top-level) session — this is what the heuristic will pick as
    // the project's orchestrator. We never POST a hook for it.
    let parent = app
        .mgr
        .db()
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "root".into(),
            parent_session_id: None,
        })
        .unwrap();

    // Child session — has a parent, so the heuristic skips it. The drain
    // path must NOT fire for this session.
    let child = app
        .mgr
        .db()
        .insert_session(NewSession {
            project_id: project.id,
            role: "implementation".into(),
            branch: None,
            worktree_path: project.path.clone(),
            task_summary: "child".into(),
            parent_session_id: Some(parent.id),
        })
        .unwrap();
    let token = format!("tok_{}", uuid::Uuid::new_v4().simple());
    app.mgr.db().set_api_token(child.id, &token).unwrap();

    app.mgr
        .append_pending_overseer_recs(child.id, "should not be drained")
        .unwrap();

    let _ = common::post_hook(
        &app,
        &token,
        &child.id,
        HookKind::UserPromptSubmit,
        serde_json::json!({"est_tokens": 0}),
    )
    .await;

    let row = app.mgr.db().get_session(child.id).unwrap();
    assert_eq!(
        row.pending_overseer_recs.as_deref(),
        Some("should not be drained"),
        "non-orchestrator session must keep its pending_overseer_recs intact"
    );
}
