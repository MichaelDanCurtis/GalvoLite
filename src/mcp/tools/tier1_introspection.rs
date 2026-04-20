//! Tier-1 MCP tools: introspection + self-control.
//!
//! Spec §5.1: `whoami`, `get_transcript`, `get_pty_log`, `pause_self`,
//! `request_end`. The first three are read-only and do NOT write event
//! rows (minimum-write principle). `pause_self` and `request_end` go
//! through `state::apply_event`.

use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use crate::state::{self, EventInput, Source};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "whoami",
            description: "Return this session's identity: id, role, project_id, worktree, parent_id, started_at.",
            input_schema: json!({ "type": "object", "properties": {} }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(whoami(ctx, args))),
        },
        ToolDef {
            name: "get_transcript",
            description: "Return recent PTY scrollback lines for this session (or another with session_id). turns optional.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "turns":      { "type": "integer", "minimum": 1, "maximum": 50 }
                }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(get_transcript(ctx, args))),
        },
        ToolDef {
            name: "get_pty_log",
            description: "Return ANSI-stripped PTY scrollback for this session (or another with session_id). limit optional.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "limit":      { "type": "integer", "minimum": 1, "maximum": 1000 }
                }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(get_pty_log(ctx, args))),
        },
        ToolDef {
            name: "pause_self",
            description: "Voluntarily mark self as awaiting input. Operator can dismiss to resume.",
            input_schema: json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(pause_self(ctx, args))),
        },
        ToolDef {
            name: "request_end",
            description: "Request a graceful end to this session. Operator confirms via dashboard.",
            input_schema: json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(request_end(ctx, args))),
        },
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn target_session(ctx: &ToolCtx, raw: Option<&str>) -> Result<uuid::Uuid, String> {
    match raw {
        Some(s) => uuid::Uuid::parse_str(s).map_err(|_| "session_id not a uuid".into()),
        None => Ok(ctx.session_id),
    }
}

/// Strip a minimal subset of ANSI escape sequences (CSI + SGR) so the text is
/// readable by the model. This is intentionally conservative — for a richer
/// terminal-aware decode the dashboard already runs vt100 over the same data.
fn strip_ansi(input: &[u8]) -> String {
    let s = String::from_utf8_lossy(input);
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip through the next final byte of a CSI / OSC sequence.
            if let Some(&next) = chars.peek() {
                if next == '[' {
                    // CSI: ESC [ <params> <intermediate> <final 0x40-0x7E>
                    chars.next();
                    while let Some(&p) = chars.peek() {
                        chars.next();
                        if ('\x40'..='\x7e').contains(&p) {
                            break;
                        }
                    }
                    continue;
                } else if next == ']' {
                    // OSC: ESC ] ... BEL or ESC \
                    chars.next();
                    while let Some(p) = chars.next() {
                        if p == '\x07' {
                            break;
                        }
                        if p == '\x1b' {
                            // ESC \ terminator
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    continue;
                } else {
                    // Unknown escape — drop ESC and the next byte.
                    chars.next();
                    continue;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------------------
// whoami
// ---------------------------------------------------------------------------

async fn whoami(ctx: ToolCtx, _args: Value) -> ToolOutcome {
    let session = match ctx.mgr.db().get_session(ctx.session_id) {
        Ok(s) => s,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    ToolOutcome::Ok(json!({
        "id":         session.id.to_string(),
        "role":       session.role,
        "project_id": session.project_id.to_string(),
        "worktree":   session.worktree_path,
        "parent_id":  session.parent_session_id.map(|u| u.to_string()),
        "started_at": session.created_at.to_rfc3339(),
        "state":      session.state.to_string(),
    }))
}

// ---------------------------------------------------------------------------
// get_transcript
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct TranscriptArgs {
    session_id: Option<String>,
    turns: Option<usize>,
}

async fn get_transcript(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: TranscriptArgs = serde_json::from_value(args).unwrap_or_default();
    let target = match target_session(&ctx, parsed.session_id.as_deref()) {
        Ok(u) => u,
        Err(e) => return ToolOutcome::InvalidParams(e),
    };
    let turns = parsed.turns.unwrap_or(5);

    // For not-running sessions the scrollback is unavailable. Return an empty
    // lines array with running=false so the model can adapt.
    match ctx.mgr.snapshot_terminal(target) {
        Ok(bytes) => {
            let text = strip_ansi(&bytes);
            // Split into newline-separated lines and keep the tail.
            let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
            let tail = lines
                .iter()
                .rev()
                .take(turns)
                .rev()
                .cloned()
                .collect::<Vec<_>>();
            ToolOutcome::Ok(json!({ "lines": tail, "running": true }))
        }
        Err(crate::error::Error::SessionNotRunning(_)) => {
            ToolOutcome::Ok(json!({ "lines": [], "running": false }))
        }
        Err(crate::error::Error::SessionNotFound(_)) => {
            ToolOutcome::AppError(format!("session not found: {target}"))
        }
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// get_pty_log
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct PtyLogArgs {
    session_id: Option<String>,
    limit: Option<usize>,
}

async fn get_pty_log(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: PtyLogArgs = serde_json::from_value(args).unwrap_or_default();
    let target = match target_session(&ctx, parsed.session_id.as_deref()) {
        Ok(u) => u,
        Err(e) => return ToolOutcome::InvalidParams(e),
    };
    let limit = parsed.limit.unwrap_or(200);
    match ctx.mgr.snapshot_terminal(target) {
        Ok(bytes) => {
            let text = strip_ansi(&bytes);
            // Trim to last `limit` lines so the model gets a bounded payload.
            let lines: Vec<&str> = text.lines().collect();
            let start = lines.len().saturating_sub(limit);
            let log = lines[start..].join("\n");
            ToolOutcome::Ok(json!({ "log": log, "running": true }))
        }
        Err(crate::error::Error::SessionNotRunning(_)) => {
            ToolOutcome::Ok(json!({ "log": "", "running": false }))
        }
        Err(crate::error::Error::SessionNotFound(_)) => {
            ToolOutcome::AppError(format!("session not found: {target}"))
        }
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// pause_self
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PauseSelfArgs {
    reason: String,
}

async fn pause_self(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: PauseSelfArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    match state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.pause_self".into(),
            payload: json!({ "reason": parsed.reason }),
            dedup_key: None,
        },
    ) {
        Ok(o) => ToolOutcome::Ok(json!({ "state": o.next_state.to_string() })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// request_end
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RequestEndArgs {
    reason: String,
}

async fn request_end(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: RequestEndArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    match state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.request_end".into(),
            payload: json!({ "reason": parsed.reason }),
            dedup_key: None,
        },
    ) {
        Ok(o) => ToolOutcome::Ok(json!({ "state": o.next_state.to_string() })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strip_ansi_removes_sgr() {
        assert_eq!(strip_ansi(b"\x1b[31mhello\x1b[0m"), "hello");
    }

    #[test]
    fn strip_ansi_preserves_plain_text() {
        assert_eq!(strip_ansi(b"plain text\nsecond"), "plain text\nsecond");
    }

    #[test]
    fn strip_ansi_handles_complex_csi() {
        // Cursor positioning + clear screen + colored text.
        let input = b"\x1b[2J\x1b[H\x1b[1;32mready\x1b[0m";
        assert_eq!(strip_ansi(input), "ready");
    }
}
