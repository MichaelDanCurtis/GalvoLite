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
