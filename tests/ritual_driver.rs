use galvolite::ritual::{DriverCommand, MockSideEffects, RitualDriver};
use galvolite::signals::Signal;
use std::time::Duration;
use tokio::time::timeout;

/// Minimal handoff body satisfying all required sections and the
/// Continuation Prompt length constraint (≥50 chars).
const VALID_HANDOFF: &str = "\
## Task Assignment\ncontent\n\
## Current State\ncontent\n\
## Decisions Made\ncontent\n\
## Modified Files\ncontent\n\
## Open Questions\ncontent\n\
## Next Steps\ncontent\n\
## Critical Context\ncontent\n\
## Git State\ncontent\n\
## Continuation Prompt\nResume work on the task as described above. All context is in this handoff file.\n";

#[tokio::test]
async fn full_ritual_sequence_issues_expected_commands() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    let handoff_path = tmp.path().join("handoff.md");
    std::fs::write(&handoff_path, VALID_HANDOFF).unwrap();

    // Mock side effects: writes command trace to a channel.
    let (trace_tx, mut trace_rx) = tokio::sync::mpsc::channel::<DriverCommand>(64);
    let mock = MockSideEffects::new(trace_tx);
    let driver = RitualDriver::new_for_test(mock);
    driver.set_latest_handoff_path(handoff_path.to_string_lossy().into()).await;

    // Prime with a working heartbeat.
    driver.on_signal(Signal::StatusWorking).await;

    // User asks for handoff.
    driver.request_handoff().await;
    // Driver should now have injected the "Handoff time" prompt.
    let c1 = timeout(Duration::from_millis(500), trace_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(c1, DriverCommand::InjectPrompt(ref s) if s.contains("Handoff time")));

    // Claude writes file, emits HANDOFF:READY.
    driver.on_signal(Signal::HandoffReady).await;

    // Driver should send /clear keystroke.
    let c2 = timeout(Duration::from_millis(500), trace_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(c2, DriverCommand::SendClear));

    // Then the restore prompt.
    let c3 = timeout(Duration::from_millis(500), trace_rx.recv()).await.unwrap().unwrap();
    assert!(matches!(c3, DriverCommand::InjectPrompt(ref s) if s.contains("Read")));
}

#[tokio::test]
async fn handoff_validation_failure_triggers_retry_prompt() {
    use tempfile::TempDir;
    let tmp = TempDir::new().unwrap();
    // No handoff file actually written → read fails.

    let (trace_tx, mut trace_rx) = tokio::sync::mpsc::channel(64);
    let mock = MockSideEffects::new(trace_tx);
    let driver = RitualDriver::new_for_test(mock);
    driver.set_latest_handoff_path(tmp.path().join("missing.md").to_string_lossy().into()).await;

    driver.on_signal(Signal::StatusWorking).await;
    driver.request_handoff().await;
    let _prompt = trace_rx.recv().await.unwrap();

    driver.on_signal(Signal::HandoffReady).await;

    // Driver should have injected a retry prompt, NOT sent /clear.
    let next = trace_rx.recv().await.unwrap();
    match next {
        DriverCommand::InjectPrompt(s) => assert!(s.contains("validation failed")),
        other => panic!("expected retry prompt, got {other:?}"),
    }
}
