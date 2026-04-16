use galvolite::ritual::{DriverCommand, MockSideEffects, RitualDriver};
use galvolite::signals::Signal;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn full_ritual_sequence_issues_expected_commands() {
    // Mock side effects: writes command trace to a channel.
    let (trace_tx, mut trace_rx) = tokio::sync::mpsc::channel::<DriverCommand>(64);
    let mock = MockSideEffects::new(trace_tx);
    let driver = RitualDriver::new_for_test(mock);

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
