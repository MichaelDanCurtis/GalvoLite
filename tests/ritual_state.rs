use galvolite::ritual::{RitualEvent, RitualMachine, RitualState};
use galvolite::signals::Signal;

#[test]
fn starts_in_working_after_initial_status() {
    let mut m = RitualMachine::new();
    assert_eq!(m.state(), &RitualState::Idle);
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    assert_eq!(m.state(), &RitualState::Working);
}

#[test]
fn request_handoff_transitions_to_awaiting_ready() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    assert_eq!(m.state(), &RitualState::AwaitingHandoffReady);
}

#[test]
fn handoff_ready_transitions_to_clearing() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffReady));
    assert_eq!(m.state(), &RitualState::Clearing);
}

#[test]
fn clear_sent_transitions_to_restoring() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffReady));
    m.on_event(RitualEvent::ClearSent);
    assert_eq!(m.state(), &RitualState::Restoring);
}

#[test]
fn restore_prompt_sent_returns_to_working() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffReady));
    m.on_event(RitualEvent::ClearSent);
    m.on_event(RitualEvent::RestorePromptSent);
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    assert_eq!(m.state(), &RitualState::Working);
}

#[test]
fn handoff_blocked_goes_to_error() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::RequestHandoff);
    m.on_event(RitualEvent::Signal(Signal::HandoffBlocked { reason: "no disk".into() }));
    match m.state() {
        RitualState::Error(_) => {}
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn attention_needed_transitions_to_attention() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::Signal(Signal::AttentionNeeded { reason: "?".into() }));
    assert_eq!(m.state(), &RitualState::Attention);
}

#[test]
fn status_complete_transitions_to_complete() {
    let mut m = RitualMachine::new();
    m.on_event(RitualEvent::Signal(Signal::StatusWorking));
    m.on_event(RitualEvent::Signal(Signal::StatusComplete));
    assert_eq!(m.state(), &RitualState::Complete);
}
