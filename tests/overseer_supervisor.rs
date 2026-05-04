//! Task 12: OverseerSupervisor — per-project lifecycle.
//!
//! Test scaffold note: the plan's `common::test_app::spawn_test_app()` /
//! `app.create_project_in_temp_repo(...)` helpers do not exist. We use the real
//! `tests/common/mod.rs::test_app_with_bus()` constructor and a small
//! TempDir+git-init+add_project helper local to this file (same pattern as
//! `tests/execute_plan_e2e.rs`).
//!
//! Timing note for `project_deleted_drops_task`: the supervisor's reconcile
//! loop defaults to 30s, which is far too slow for a test. We use the
//! `with_reconcile_interval(Duration)` builder to drop it to 50ms here.
mod common;

use galvolite::db::NewProject;
use galvolite::overseer::OverseerSupervisor;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
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

/// Create a TempDir + git init + add_project. Returns the temp dir guard
/// (caller must keep it alive for the duration of the test) and the inserted
/// project row.
fn make_project(
    app: &common::TestApp,
    name: &str,
) -> (TempDir, galvolite::db::Project) {
    let dir = TempDir::new().unwrap();
    init_git(dir.path());
    let p = app
        .mgr
        .add_project(NewProject {
            name: name.into(),
            path: dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();
    (dir, p)
}

fn enable_overseer_env() {
    // `overseer_enabled()` checks GALVOLITE_OVERSEER; OverseerConfig::from_env()
    // requires KIMI_API_KEY. Set both so the supervisor actually spawns tasks.
    std::env::set_var("GALVOLITE_OVERSEER", "1");
    std::env::set_var("KIMI_API_KEY", "test");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn start_spawns_one_task_per_existing_project() {
    enable_overseer_env();
    let app = common::test_app_with_bus();
    let (_d1, _p1) = make_project(&app, "a");
    let (_d2, _p2) = make_project(&app, "b");

    let sup = OverseerSupervisor::new(Arc::clone(&app.mgr));
    sup.spawn_for_existing_projects().await;
    assert_eq!(sup.task_count(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn project_added_after_start_spawns_task() {
    enable_overseer_env();
    let app = common::test_app_with_bus();

    let sup = OverseerSupervisor::new(Arc::clone(&app.mgr));
    sup.spawn_for_existing_projects().await;
    assert_eq!(sup.task_count(), 0);

    let (_d, _p) = make_project(&app, "added-late");

    // Bus listener spawned the task; ProjectAdded fan-out is async, so allow
    // a few scheduler ticks before checking.
    for _ in 0..20 {
        if sup.task_count() == 1 { break; }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(sup.task_count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn project_deleted_drops_task() {
    enable_overseer_env();
    let app = common::test_app_with_bus();
    let (_d, p) = make_project(&app, "a");

    let sup = OverseerSupervisor::new(Arc::clone(&app.mgr))
        .with_reconcile_interval(Duration::from_millis(50));
    sup.spawn_for_existing_projects().await;
    assert_eq!(sup.task_count(), 1);

    app.mgr.delete_project(p.id).await.unwrap();

    // Reconcile interval is 50ms; allow a few ticks before checking.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(sup.task_count(), 0);
}
