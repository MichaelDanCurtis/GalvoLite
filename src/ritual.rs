//! Ritual state machine + driver.
//!
//! The state machine (`RitualMachine`) is pure: events in, state out. It
//! emits no I/O. The driver (task 15) observes state transitions and performs
//! side effects (write prompt, send /clear, write handoff, etc.).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RitualState {
    /// Newly constructed.
    Idle,
    /// Normal operation.
    Working,
    /// User (or autopilot) has requested a handoff.
    AwaitingHandoffReady,
    /// Handoff validated; about to send `/clear`.
    Clearing,
    /// `/clear` sent; about to send restore prompt.
    Restoring,
    /// Operator/MCP requested attention. User must respond.
    Attention,
    /// Session complete (terminal).
    Complete,
    /// Something went wrong; human attention required.
    Error(String),
}

#[derive(Debug, Clone)]
pub enum RitualEvent {
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

impl Default for RitualMachine {
    fn default() -> Self {
        Self { state: RitualState::Idle }
    }
}

impl RitualMachine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &RitualState {
        &self.state
    }

    pub fn on_event(&mut self, ev: RitualEvent) {
        use RitualEvent as E;
        use RitualState as S;

        let next = match (self.state.clone(), ev) {
            (S::Working, E::RequestHandoff) => S::AwaitingHandoffReady,
            (S::Idle, E::RequestHandoff) => S::AwaitingHandoffReady,

            (S::Clearing, E::ClearSent) => S::Restoring,
            (S::Restoring, E::RestorePromptSent) => S::Restoring,

            (S::Attention, E::AttentionResolved) => S::Working,

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
    async fn inject_raw(&self, bytes: &[u8]);
}

#[derive(Debug, Clone)]
pub enum DriverCommand {
    InjectPrompt(String),
    SendClear,
    InjectRaw(Vec<u8>),
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
    async fn inject_raw(&self, bytes: &[u8]) {
        let _ = self.tx.send(DriverCommand::InjectRaw(bytes.to_vec())).await;
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
                 Call `write_handoff({summary, next_steps})` to record the continuation.",
            )
            .await;
    }

    /// Driver calls this to confirm the latest recorded handoff file actually
    /// exists and is well-formed.
    pub async fn validate_latest_handoff(&self) -> std::result::Result<(), crate::error::Error> {
        let path = self
            .latest_handoff_path
            .lock()
            .await
            .clone()
            .ok_or_else(|| crate::error::Error::Ritual("no handoff path recorded".into()))?;
        let body = crate::handoff::read_handoff(std::path::Path::new(&path))?;
        crate::handoff::validate_handoff_content(&body)
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
             Call `status_report({{kind: 'working'}})` once you have read and understood the handoff."
        )
    }

    pub async fn resolve_attention(&self) {
        self.machine
            .lock()
            .await
            .on_event(RitualEvent::AttentionResolved);
    }

    pub async fn inject_raw(&self, bytes: &[u8]) {
        self.fx.inject_raw(bytes).await;
    }

    pub async fn abort(&self, reason: &str) {
        self.machine
            .lock()
            .await
            .on_event(RitualEvent::Abort(reason.to_string()));
    }
}

use crate::pty::PtyHandleShared;

/// Production side effects: forwards to a pty handle.
pub struct PtySideEffects {
    pty: PtyHandleShared,
}

impl PtySideEffects {
    pub fn new(pty: PtyHandleShared) -> Self {
        Self { pty }
    }
}

#[async_trait::async_trait]
impl SideEffects for PtySideEffects {
    async fn inject_prompt(&self, prompt: &str) {
        // Claude Code's CLI accepts input on stdin terminated by Enter.
        let mut buf = prompt.as_bytes().to_vec();
        buf.push(b'\n');
        let _ = self.pty.write(&buf).await;
    }

    async fn send_clear(&self) {
        let _ = self.pty.write_str("/clear\n").await;
    }

    async fn inject_raw(&self, bytes: &[u8]) {
        let _ = self.pty.write(bytes).await;
    }
}
