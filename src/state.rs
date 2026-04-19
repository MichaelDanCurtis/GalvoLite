//! State funnel for the hooks+MCP coordination layer.
//!
//! See `docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md`
//! §8 for the design. Every session-state transition in GalvoLite goes through
//! [`apply_event`] (added in Task 5); this module owns the canonical
//! [`SessionState`] enum and the event input/outcome types that funnel
//! describes.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Session lifecycle states from spec §8.1. The DB stores the snake_case
/// rendering returned by [`SessionState::fmt`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Row inserted, child PTY not yet observed alive.
    Starting,
    /// PTY alive, no work in flight (default between turns).
    Idle,
    /// Model is mid-turn.
    Working,
    /// Model paused on an operator question (Finding 3 fix).
    AwaitingInput,
    /// Model asked for operator attention (`request_attention` /
    /// `status_report(blocked)`).
    NeedsAttention,
    /// PTY died unexpectedly, or row is left over from a prior schema.
    Error,
    /// Session ended cleanly (`SessionEnd` hook or `request_end` tool).
    Ended,
}

impl SessionState {
    /// `Error` and `Ended` are terminal — no transition out.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Error | Self::Ended)
    }
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Starting       => "starting",
            Self::Idle           => "idle",
            Self::Working        => "working",
            Self::AwaitingInput  => "awaiting_input",
            Self::NeedsAttention => "needs_attention",
            Self::Error          => "error",
            Self::Ended          => "ended",
        })
    }
}

impl FromStr for SessionState {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "starting"        => Self::Starting,
            "idle"            => Self::Idle,
            "working"         => Self::Working,
            "awaiting_input"  => Self::AwaitingInput,
            "needs_attention" => Self::NeedsAttention,
            "error"           => Self::Error,
            "ended"           => Self::Ended,
            other => return Err(Error::Config(format!("unknown session state: {other}"))),
        })
    }
}

/// Origin of a state-changing event. Stored verbatim in `events.source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Triggered by a model-chosen MCP tool call.
    Mcp,
    /// Fired by Claude Code via `/hooks/ingest`.
    Hook,
    /// Triggered by a dashboard / CLI action.
    Operator,
    /// Synthesized by GalvoLite itself (orphan reconciler, signal bridge,
    /// startup recovery markers).
    System,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Mcp      => "mcp",
            Self::Hook     => "hook",
            Self::Operator => "operator",
            Self::System   => "system",
        })
    }
}

impl FromStr for Source {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "mcp"      => Self::Mcp,
            "hook"     => Self::Hook,
            "operator" => Self::Operator,
            "system"   => Self::System,
            other => return Err(Error::Config(format!("unknown event source: {other}"))),
        })
    }
}

/// Input to [`apply_event`]. The funnel writes one `events` row per call
/// (skipped if `dedup_key` already exists).
#[derive(Debug, Clone)]
pub struct EventInput {
    pub source: Source,
    /// Free-form event kind, e.g. `"hook.SessionStart"`, `"mcp.set_role"`,
    /// `"signal.STATUS:WORKING"`, `"system.recovered"`. The convention is
    /// `"<source>.<name>"` but the funnel does not enforce it — kind is
    /// the dimension `read_events`/dashboard filters on, so use whatever
    /// the consumer expects.
    pub kind: String,
    pub payload: serde_json::Value,
    /// Optional UNIQUE key. If present and a prior row carries the same
    /// value, the funnel returns `deduped = true` without writing.
    pub dedup_key: Option<String>,
}

/// Result of a successful `apply_event` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOutcome {
    /// Rowid of the new `events` row, or the existing row if `deduped`.
    pub event_id: i64,
    /// State the session was in immediately before the event landed
    /// (`None` if this is the first event for a brand-new row).
    pub prev_state: Option<SessionState>,
    /// State the session is in after the event was applied. Equals
    /// `prev_state` for events that don't change state (logging-only).
    pub next_state: SessionState,
    /// `true` if the event was a no-op because `dedup_key` matched a
    /// prior row.
    pub deduped: bool,
}
