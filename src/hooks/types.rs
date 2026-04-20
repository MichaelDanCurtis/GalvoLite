//! Shared envelope types for `/hooks/ingest`.
//!
//! Both the `galvolite-hook` binary and the axum handler use these so the
//! wire format can never drift between the two.

use serde::{Deserialize, Serialize};

/// The seven Claude Code hooks GalvoLite cares about. The `Display` form
/// (via `Debug`) is also the suffix used in `events.kind` (`hook.<HookKind>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookKind {
    SessionStart,
    SessionEnd,
    PreCompact,
    Stop,
    UserPromptSubmit,
    PostToolUse,
    Notification,
}

impl HookKind {
    /// Render the kind as it appears in `events.kind`: e.g.
    /// `hook.SessionStart`.
    pub fn as_event_kind(&self) -> String {
        format!("hook.{self:?}")
    }
}

/// Envelope sent from `galvolite-hook` to `/hooks/ingest`.
#[derive(Debug, Serialize, Deserialize)]
pub struct HookIngestRequest {
    pub session_id: String,
    pub hook: HookKind,
    pub ts: String,
    pub event_id: String,
    pub payload: serde_json::Value,
}

/// Response from `/hooks/ingest`. `inject_text` is written to stdout by
/// `galvolite-hook` so Claude Code injects it into the model's context.
#[derive(Debug, Serialize, Deserialize)]
pub struct HookIngestResponse {
    pub action: HookAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inject_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What Claude Code should do after the hook fires. Today the binary
/// only emits `inject_text`; `block` is reserved for future use (e.g.
/// permission gating) and currently behaves like `proceed` from the
/// model's perspective because we never write to stdout when blocked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HookAction {
    Proceed,
    Continue,
    Block,
}
