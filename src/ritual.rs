//! Ritual state machine + driver.
//!
//! The state machine (`RitualMachine`) is pure: events in, state out. It
//! emits no I/O. The driver (task 15) observes state transitions and performs
//! side effects (write prompt, send /clear, write handoff, etc.).

use crate::signals::Signal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RitualState {
    /// Newly constructed, no signals seen yet.
    Idle,
    /// Normal operation.
    Working,
    /// User (or autopilot) has requested a handoff; waiting for `[HANDOFF:READY]`.
    AwaitingHandoffReady,
    /// Handoff validated; about to send `/clear`.
    Clearing,
    /// `/clear` sent; about to send restore prompt.
    Restoring,
    /// Claude emitted `[ATTENTION:NEEDED ...]`. User must respond.
    Attention,
    /// `[STATUS:COMPLETE]` received. Terminal (session ready for PR step).
    Complete,
    /// Something went wrong; human attention required.
    Error(String),
}

#[derive(Debug, Clone)]
pub enum RitualEvent {
    Signal(Signal),
    /// User (or autopilot) asks to initiate the ritual.
    RequestHandoff,
    /// Driver has sent `/clear`.
    ClearSent,
    /// Driver has sent the restore prompt.
    RestorePromptSent,
    /// User supplied the response Claude was waiting for.
    AttentionResolved,
    /// Fatal abort; state machine terminates.
    Abort(String),
}

#[derive(Debug, Clone)]
pub struct RitualMachine {
    state: RitualState,
}

impl RitualMachine {
    pub fn new() -> Self {
        Self { state: RitualState::Idle }
    }

    pub fn state(&self) -> &RitualState {
        &self.state
    }

    pub fn on_event(&mut self, ev: RitualEvent) {
        use RitualEvent as E;
        use RitualState as S;
        use Signal as Sg;

        let next = match (self.state.clone(), ev) {
            (S::Idle, E::Signal(Sg::StatusWorking)) => S::Working,

            (S::Working, E::RequestHandoff) => S::AwaitingHandoffReady,

            (S::AwaitingHandoffReady, E::Signal(Sg::HandoffReady)) => S::Clearing,
            (S::AwaitingHandoffReady, E::Signal(Sg::HandoffBlocked { reason })) => {
                S::Error(format!("handoff blocked: {reason}"))
            }

            (S::Clearing, E::ClearSent) => S::Restoring,
            (S::Restoring, E::RestorePromptSent) => S::Restoring,
            (S::Restoring, E::Signal(Sg::StatusWorking)) => S::Working,

            (_, E::Signal(Sg::AttentionNeeded { .. })) => S::Attention,
            (S::Attention, E::AttentionResolved) => S::Working,

            (S::Working, E::Signal(Sg::AttentionContext)) => S::AwaitingHandoffReady,

            (_, E::Signal(Sg::StatusComplete)) => S::Complete,
            (_, E::Abort(r)) => S::Error(r),

            (s, _) => s,
        };
        self.state = next;
    }
}
