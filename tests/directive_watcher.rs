//! Out-of-band directive edits emit DirectiveChanged via the filesystem
//! watcher (Sub-plan 7 Task 4). PUT /api/projects/:id/directive emits
//! the same event directly — that path is covered by tests/directive_api.rs.

use galvolite::api::events::DomainEvent;

mod common;
use common::test_app_with_bus;

const SAMPLE_DIRECTIVE: &str = r#"# Directive: smoke

Plan: docs/plan.md

## Phases

| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| P1 | 1, 2 | full | — |

## Escalation rules

- on tests failing → halt

## Notes

ok
"#;

#[tokio::test]
async fn external_edit_emits_directive_changed() {
    let app = test_app_with_bus();
    // Watchers also need to be started (test_app_with_bus does NOT call
    // start_directive_watchers — we need to call it explicitly OR add the
    // project after construction so spawn_watcher is invoked via add_project).
    // We use the live-add path: add_project triggers spawn_watcher directly.

    // Use a real on-disk dir we can write into.
    let proj_dir = tempfile::tempdir().unwrap();
    let proj_path = proj_dir.path().to_string_lossy().into_owned();

    let project = app
        .mgr
        .add_project(galvolite::db::NewProject {
            name: "watcher-test".into(),
            path: proj_path,
            remote_url: None,
        })
        .expect("add_project succeeds");

    // Subscribe AFTER add_project so the broadcast channel exists; the
    // watcher only fires on writes after this point anyway.
    let mut events = app.mgr.subscribe_events();

    // Brief settle to let notify finish registering the inotify/kqueue
    // watch on .galvolite before we write.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    // Write directly to the filesystem (NOT via the API).
    let dir_path = std::path::Path::new(&project.path).join(".galvolite");
    std::fs::create_dir_all(&dir_path).unwrap();
    std::fs::write(dir_path.join("directive.md"), SAMPLE_DIRECTIVE).unwrap();

    // Watcher should fire within 3 s.
    let frame = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        async {
            loop {
                match events.recv().await {
                    Ok(DomainEvent::DirectiveChanged { project_id }) if project_id == project.id => {
                        return Ok::<_, ()>(());
                    }
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        },
    )
    .await;

    frame
        .expect("watcher did not fire within 3s")
        .expect("event channel closed");
}

#[tokio::test]
async fn watcher_debounces_burst_writes() {
    let app = test_app_with_bus();
    let proj_dir = tempfile::tempdir().unwrap();
    let project = app
        .mgr
        .add_project(galvolite::db::NewProject {
            name: "burst".into(),
            path: proj_dir.path().to_string_lossy().into_owned(),
            remote_url: None,
        })
        .unwrap();

    let mut events = app.mgr.subscribe_events();
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    let target = std::path::Path::new(&project.path).join(".galvolite/directive.md");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    // Write three times back-to-back; debounce should collapse to ~1 frame.
    for i in 0..3 {
        std::fs::write(&target, format!("v{i}\n")).unwrap();
    }

    // Drain frames for 800 ms (debounce is 500 ms; we want one fire from
    // the burst, not three).
    let mut directive_frames = 0usize;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(
            std::time::Duration::from_millis(100),
            events.recv(),
        ).await {
            Ok(Ok(DomainEvent::DirectiveChanged { project_id })) if project_id == project.id => {
                directive_frames += 1;
            }
            Ok(Ok(_)) => continue,
            Ok(Err(_)) => break,
            Err(_) => continue,
        }
    }

    assert!(
        directive_frames >= 1,
        "expected at least 1 DirectiveChanged frame from burst"
    );
    assert!(
        directive_frames <= 2,
        "expected ≤2 frames (debounce active); got {directive_frames}"
    );
}
