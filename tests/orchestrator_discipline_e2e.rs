//! End-to-end discipline check for Sub-plan 7. Verifies the data flow that
//! the orchestrator-discipline contract depends on, without driving a real
//! claude PTY (the contract itself is enforced via CLAUDE.md profile text
//! and is covered by tests/profiles_turn_discipline.rs).
//!
//! Scenario:
//!   1. Operator-side: write plan.md + directive.md (simulates brainstorm output).
//!   2. Verify both REST endpoints return them — SpawnControls gate would
//!      unlock at this point in the UI.
//!   3. Spawn an "implementation" child for plan_part=1 (parent uses /bin/cat
//!      as PTY stand-in).
//!   4. Child writes a postmortem via the write_postmortem path.
//!   5. Record an operator decision directly via decisions::record (mirrors what
//!      OperatorPromptRegistry::answer does after operator response).
//!   6. Call claude_md::build_prior_context for the same plan_part — both
//!      the postmortem AND the decision must surface.
//!   7. Assert directive.md and plan.md mtimes UNCHANGED across the run
//!      — no orchestrator-side edit happened.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::SystemTime;

use chrono::Utc;
use tempfile::TempDir;
use tower::ServiceExt;

use galvolite::db::NewProject;
use galvolite::decisions::{record as record_decision, Decision};
use galvolite::postmortem::{write as write_postmortem, Outcome, Postmortem};

fn init_git(d: &Path) {
    Command::new("git")
        .args(["init", "-q", "-b", "main"])
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
        .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "i"])
        .current_dir(d)
        .status()
        .unwrap();
}

fn mtime(p: &Path) -> SystemTime {
    std::fs::metadata(p).unwrap().modified().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discipline_e2e_plan_directive_postmortem_decision_flow() {
    let app = common::test_app_with_bus();
    // The project lives in a separate TempDir so we can control its git repo
    // independently of the harness DB directory.
    let proj_dir = TempDir::new().unwrap();
    init_git(proj_dir.path());

    let proj = app.mgr.add_project(NewProject {
        name: "disc".into(),
        path: proj_dir.path().to_string_lossy().into_owned(),
        remote_url: None,
    }).unwrap();

    // -------------------------------------------------------------------------
    // 1. Brainstorm output: plan.md + directive.md land on disk under .galvolite.
    // -------------------------------------------------------------------------
    let g_dir = proj_dir.path().join(".galvolite");
    std::fs::create_dir_all(&g_dir).unwrap();

    let plan_path = g_dir.join("plan.md");
    std::fs::write(&plan_path, "## Parts\n- 1: build the thing\n").unwrap();

    // directive.md must parse successfully: needs `# Directive: <name>`,
    // `Plan: <path>`, and a `## Phases` table with at least one data row.
    // Depends on cell must be `-` (single ASCII dash) or `—` (em-dash) for
    // parse_depends_on to treat it as "no deps". Escalation rule lines must
    // contain `→` (U+2192) to be recognised; lines without it are skipped.
    let directive_md = "\
# Directive: disc

Plan: ./plan.md

## Phases

| ID | Parts | Autonomy | Depends on |
|----|-------|----------|------------|
| P1 | 1     | full     | -          |

## Escalation rules

- On schema change → ask_operator

## Notes

minimal
";
    let directive_path = g_dir.join("directive.md");
    std::fs::write(&directive_path, directive_md).unwrap();

    let initial_directive_mtime = mtime(&directive_path);
    let initial_plan_mtime = mtime(&plan_path);

    // -------------------------------------------------------------------------
    // 2. SpawnControls gate would unlock — both endpoints return 200.
    // -------------------------------------------------------------------------
    let plan_resp = app.app.clone().oneshot(
        axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/projects/{}/plan", proj.id))
            .body(axum::body::Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(
        plan_resp.status(),
        axum::http::StatusCode::OK,
        "GET /plan should return 200"
    );

    let dir_resp = app.app.clone().oneshot(
        axum::http::Request::builder()
            .method("GET")
            .uri(format!("/api/projects/{}/directive", proj.id))
            .body(axum::body::Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(
        dir_resp.status(),
        axum::http::StatusCode::OK,
        "GET /directive should return 200"
    );

    // -------------------------------------------------------------------------
    // 3. Spawn a parent session (uses /bin/cat as PTY stand-in), then a child.
    // -------------------------------------------------------------------------
    let parent = app.mgr.spawn_session(
        proj.id,
        "parent".into(),
        "implementation".into(),
        "/bin/cat".into(),
        vec![],
    ).await.unwrap();

    let child = app.mgr.spawn_child(parent.id, "implementation", "P1 work", Some(1)).await.unwrap();

    // -------------------------------------------------------------------------
    // 4. Child writes a postmortem on completion.
    //    postmortem::write uses the project path as base.
    // -------------------------------------------------------------------------
    write_postmortem(
        proj_dir.path(),
        &Postmortem {
            plan_part: 1,
            session_id: child.id,
            outcome: Outcome::Shipped,
            cycles: 2,
            duration_seconds: 1800,
            written_at: Utc::now(),
            what_worked: "TDD scaffolded fast".into(),
            gotchas: "Worktree GC raced with child exit".into(),
            follow_ups: "Audit GC ordering".into(),
        },
    ).unwrap();

    // -------------------------------------------------------------------------
    // 5. Record an operator decision (mirrors what answer() does internally).
    // -------------------------------------------------------------------------
    record_decision(
        proj_dir.path(),
        &Decision {
            session_id: parent.id,
            asked_at: Utc::now(),
            question: "Use SQLite for the cache?".into(),
            answer: "Yes, single-writer is fine".into(),
            choices: None,
        },
    ).unwrap();

    // -------------------------------------------------------------------------
    // 6. Build prior context for plan_part=1 — must contain both artifacts.
    // -------------------------------------------------------------------------
    let prior = galvolite::claude_md::build_prior_context(proj_dir.path(), Some(1));

    assert!(
        prior.contains("## Prior context"),
        "prior context header missing:\n{prior}"
    );
    assert!(
        prior.contains("Worktree GC raced"),
        "postmortem gotcha not injected into prior context:\n{prior}"
    );
    assert!(
        prior.contains("Use SQLite for the cache?"),
        "decision question not injected into prior context:\n{prior}"
    );

    // -------------------------------------------------------------------------
    // 7. plan.md and directive.md must be unmodified — no orchestrator-side edit.
    // -------------------------------------------------------------------------
    assert_eq!(
        mtime(&directive_path),
        initial_directive_mtime,
        "directive.md mtime changed; something wrote to it during the test"
    );
    assert_eq!(
        mtime(&plan_path),
        initial_plan_mtime,
        "plan.md mtime changed; something wrote to it during the test"
    );

    // Cleanup: kill the parent so PTY resources are released before the process exits.
    let _ = app.mgr.kill_session(parent.id).await;
}
