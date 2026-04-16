//! End-to-end: spawn mock-claude via the real PtyHandle + SignalPump +
//! RitualDriver + handoff validation, trigger a handoff, and assert the
//! ritual completes losslessly.

use galvolite::claude_md::{implementation_profile, InjectionPrompt};
use galvolite::db::{Db, NewProject, NewSession};
use galvolite::handoff::HandoffDir;
use galvolite::pty::{PtyHandle, PtyOpts, SignalPump};
use galvolite::ritual::{PtySideEffects, RitualDriver, RitualState};
use galvolite::signals::Signal;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::{sleep, timeout};

fn mock_claude_path() -> std::path::PathBuf {
    // When `cargo test` runs, binary is at target/<profile>/mock-claude.
    let exe = std::env::current_exe().unwrap();
    let mut dir = exe.parent().unwrap().to_path_buf();
    if dir.ends_with("deps") {
        dir.pop();
    }
    dir.join("mock-claude")
}

#[tokio::test]
async fn full_ritual_with_mock_claude() {
    // ---------- Setup ----------
    let data_dir = TempDir::new().unwrap();
    let proj_dir = TempDir::new().unwrap();
    let db = Db::open(data_dir.path().join("t.db")).unwrap();

    let proj = db.insert_project(NewProject {
        name: "integration".into(),
        path: proj_dir.path().to_string_lossy().into(),
        remote_url: None,
    }).unwrap();

    let session = db.insert_session(NewSession {
        project_id: proj.id,
        role: "implementation".into(),
        branch: None,
        worktree_path: proj_dir.path().to_string_lossy().into(),
        task_summary: "mock integration".into(),
        parent_session_id: None,
    }).unwrap();

    // Write CLAUDE.md + prep handoff dir
    let galvolite_dir = proj_dir.path().join(".galvolite");
    std::fs::create_dir_all(&galvolite_dir).unwrap();
    let claude_md_path = galvolite_dir.join("CLAUDE-session.md");
    std::fs::write(&claude_md_path, implementation_profile()).unwrap();
    let handoff_dir = HandoffDir::new(galvolite_dir.join("handoffs"));
    galvolite::handoff::ensure_session_dir(&handoff_dir, session.id).unwrap();

    // ---------- Spawn mock ----------
    let pty = PtyHandle::spawn(PtyOpts {
        cwd: proj_dir.path().to_path_buf(),
        command: mock_claude_path().to_string_lossy().into(),
        args: vec![],
        env: vec![
            ("GALVO_MOCK_HANDOFF_DIR".into(), handoff_dir.root().to_string_lossy().into()),
            ("GALVO_MOCK_SESSION_ID".into(), session.id.to_string()),
        ],
    }).expect("spawn");
    let mut pump = SignalPump::start(pty, db.clone(), session.id);
    let fx = Arc::new(PtySideEffects::new(pump.handle()));
    let driver = Arc::new(RitualDriver::new(fx));

    // ---------- Inject initial prompt ----------
    // Send a minimal one-line prompt so mock-claude sees exactly one trigger line.
    // (InjectionPrompt::render() embeds \n which would create multiple lines through
    // the pty's BufReader and cause multiple StatusWorking emissions.)
    let session_id_str = session.id.to_string();
    let claude_md_path_str = claude_md_path.to_string_lossy().into_owned();
    let _inj = InjectionPrompt {
        task_summary: "mock integration",
        session_id: &session_id_str,
        claude_md_path: &claude_md_path_str,
    };
    pump.handle().write_str("GalvoLite session begin\n").await.unwrap();

    // ---------- Wait for first StatusWorking ----------
    // Drain until we see a StatusWorking (the initial prompt may produce more than one
    // if the pty echoes intermediate lines that also match mock's patterns).
    let mut got_working = false;
    for _ in 0..5 {
        let sig = timeout(Duration::from_secs(5), pump.next_signal())
            .await.expect("timeout waiting for first StatusWorking").expect("signal");
        if sig == Signal::StatusWorking {
            driver.on_signal(sig).await;
            got_working = true;
            break;
        }
    }
    assert!(got_working, "never received StatusWorking after initial prompt");
    assert_eq!(driver.state().await, RitualState::Working);

    // ---------- Trigger handoff ----------
    driver.request_handoff().await;
    // Driver injected handoff prompt; mock will write file + emit HandoffReady.

    // Drain signals until HandoffReady — there may be extra StatusWorking signals
    // queued from the initial prompt's pty echo lines.
    let mut handoff_sig = None;
    for _ in 0..5 {
        let sig = timeout(Duration::from_secs(5), pump.next_signal())
            .await.expect("timeout waiting for HandoffReady").expect("signal");
        if sig == Signal::HandoffReady {
            handoff_sig = Some(sig);
            break;
        }
        // Any intermediate StatusWorking: feed to driver and keep draining.
        if sig == Signal::StatusWorking {
            driver.on_signal(sig).await;
        }
    }
    let sig = handoff_sig.expect("never received HandoffReady");
    assert_eq!(sig, Signal::HandoffReady);

    // Look up latest handoff file and tell the driver.
    let latest = galvolite::handoff::latest_handoff_for(&handoff_dir, session.id)
        .unwrap().expect("handoff file written");
    driver.set_latest_handoff_path(latest.to_string_lossy().into()).await;

    driver.on_signal(sig).await;

    // Driver should now be in Restoring (clear sent, restore prompt sent).
    // Wait for mock's post-restore StatusWorking.
    let sig = timeout(Duration::from_secs(5), pump.next_signal())
        .await.expect("timeout").expect("signal");
    assert_eq!(sig, Signal::StatusWorking);
    driver.on_signal(sig).await;

    // ---------- Assert: ritual completed losslessly ----------
    assert_eq!(driver.state().await, RitualState::Working);

    // Signals persisted
    let rows = db.list_signals_for_session(session.id).unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
    assert!(kinds.contains(&"STATUS:WORKING"));
    assert!(kinds.contains(&"HANDOFF:READY"));
    // At least 2 StatusWorking entries (one per cycle)
    let hb_count = kinds.iter().filter(|k| **k == "STATUS:WORKING").count();
    assert!(hb_count >= 2, "expected ≥2 heartbeats, got {hb_count}");

    // Cleanup
    sleep(Duration::from_millis(50)).await;
}
