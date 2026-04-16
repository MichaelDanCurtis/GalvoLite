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
    pub(crate) state: RitualState,
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

use tokio::sync::{mpsc, Mutex};
use std::sync::Arc;

/// Side effects the driver needs. Abstracted so tests can observe without a real pty.
#[async_trait::async_trait]
pub trait SideEffects: Send + Sync + 'static {
    async fn inject_prompt(&self, prompt: &str);
    async fn send_clear(&self);
}

#[derive(Debug, Clone)]
pub enum DriverCommand {
    InjectPrompt(String),
    SendClear,
}

/// Test double: records commands to a channel.
pub struct MockSideEffects {
    tx: mpsc::Sender<DriverCommand>,
}

impl MockSideEffects {
    pub fn new(tx: mpsc::Sender<DriverCommand>) -> Self {
        Self { tx }
    }
}

#[async_trait::async_trait]
impl SideEffects for MockSideEffects {
    async fn inject_prompt(&self, prompt: &str) {
        let _ = self.tx.send(DriverCommand::InjectPrompt(prompt.to_string())).await;
    }
    async fn send_clear(&self) {
        let _ = self.tx.send(DriverCommand::SendClear).await;
    }
}

pub struct RitualDriver {
    machine: Arc<Mutex<RitualMachine>>,
    fx: Arc<dyn SideEffects>,
    latest_handoff_path: Arc<Mutex<Option<String>>>,
}

impl RitualDriver {
    pub fn new(fx: Arc<dyn SideEffects>) -> Self {
        Self {
            machine: Arc::new(Mutex::new(RitualMachine::new())),
            fx,
            latest_handoff_path: Arc::new(Mutex::new(None)),
        }
    }

    /// Test-only constructor.
    pub fn new_for_test(fx: MockSideEffects) -> Self {
        Self::new(Arc::new(fx))
    }

    pub async fn state(&self) -> RitualState {
        self.machine.lock().await.state().clone()
    }

    pub async fn set_latest_handoff_path(&self, path: String) {
        *self.latest_handoff_path.lock().await = Some(path);
    }

    pub async fn request_handoff(&self) {
        let before = self.machine.lock().await.state().clone();
        if !matches!(before, RitualState::Working) {
            return;
        }
        self.machine.lock().await.on_event(RitualEvent::RequestHandoff);
        self.fx
            .inject_prompt(
                "Handoff time. Follow the Handoff Protocol in your CLAUDE.md. \
                 Write your handoff file now, then emit [HANDOFF:READY] on its own line.",
            )
            .await;
    }

    pub async fn on_signal(&self, sig: Signal) {
        let sig_clone = sig.clone();
        self.machine.lock().await.on_event(RitualEvent::Signal(sig));

        // Drive follow-up side effects based on new state.
        let state = self.machine.lock().await.state().clone();
        match (state, sig_clone) {
            (RitualState::Clearing, Signal::HandoffReady) => {
                self.fx.send_clear().await;
                self.machine.lock().await.on_event(RitualEvent::ClearSent);

                // Immediately inject restore prompt.
                let restore = self.build_restore_prompt().await;
                self.fx.inject_prompt(&restore).await;
                self.machine
                    .lock()
                    .await
                    .on_event(RitualEvent::RestorePromptSent);
            }
            _ => {}
        }
    }

    async fn build_restore_prompt(&self) -> String {
        let path = self
            .latest_handoff_path
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| "<latest handoff>".to_string());
        format!(
            "Read `{path}`. You are the next cycle of the same session. \
             Resume from the Continuation Prompt section. \
             You have no memory of prior cycles — trust only the handoff file and the project state. \
             Emit [STATUS:WORKING] once you have read and understood the handoff."
        )
    }

    pub async fn resolve_attention(&self) {
        self.machine
            .lock()
            .await
            .on_event(RitualEvent::AttentionResolved);
    }

    pub async fn abort(&self, reason: &str) {
        self.machine
            .lock()
            .await
            .on_event(RitualEvent::Abort(reason.to_string()));
    }
}
