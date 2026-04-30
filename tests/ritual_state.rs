use galvolite::ritual::{RitualEvent, RitualMachine, RitualState};

#[test]
fn starts_in_idle() {
    let m = RitualMachine::new();
    assert_eq!(m.state(), &RitualState::Idle);
}

#[test]
fn request_handoff_transitions_idle_to_awaiting_ready() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::RequestHandoff);
    assert_eq!(m.state(), &RitualState::AwaitingHandoffReady);
}

#[test]
fn clear_sent_outside_clearing_is_noop() {
    let mut m = RitualMachine::new();
    // Force Clearing via direct manipulation of public API: drive
    // handoff into AwaitingHandoffReady, then nothing pushes to Clearing
    // without a signal — so this scenario is now unreachable in the pure
    // state machine; just verify ClearSent in non-Clearing is a no-op.
    m.on_event(RitualEvent::ClearSent);
    assert_eq!(m.state(), &RitualState::Idle);
}

#[test]
fn abort_transitions_to_error() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Abort("disk full".into()));
    match m.state() {
        RitualState::Error(_) => {}
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn attention_resolved_outside_attention_is_noop() {
    let mut m = RitualMachine::new();
    // Manually wire Attention via Abort + nothing (no Signal arms),
    // so this asserts the AttentionResolved arm's no-op behavior on Idle.
    m.on_event(RitualEvent::AttentionResolved);
    assert_eq!(m.state(), &RitualState::Idle);
}
