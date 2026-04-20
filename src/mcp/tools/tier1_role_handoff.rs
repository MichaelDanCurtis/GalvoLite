//! Tier-1 MCP tools: role + handoff.
//!
//! Spec §5.1: `set_role`, `write_handoff`, `read_handoff`. Role transitions
//! rewrite `CLAUDE.md` in the worktree so the next turn loads the new
//! profile. Handoff tools route through existing helpers in
//! [`crate::handoff`] — no file-layout logic duplicated.

use crate::handoff::{self, HandoffDir};
use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use crate::state::{self, EventInput, Source};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

const ROLES: &[&str] = &["orchestrator", "brainstorm", "implementation", "scout"];

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "set_role",
            description: "Switch this session's role mid-flight. Rewrites CLAUDE.md so the next turn loads the new profile.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "role": { "type": "string", "enum": ROLES }
                },
                "required": ["role"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(set_role(ctx, args))),
        },
        ToolDef {
            name: "write_handoff",
            description: "Write a structured handoff into .galvolite/handoffs/{session}/<ts>.md. Body must be a complete handoff per the profile contract.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "body": { "type": "string" }
                },
                "required": ["body"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(write_handoff(ctx, args))),
        },
        ToolDef {
            name: "read_handoff",
            description: "Read a handoff. With path returns that exact file; without args returns the most recent for this session.",
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(read_handoff(ctx, args))),
        },
    ]
}

// ---------------------------------------------------------------------------

fn handoff_dir_for(ctx: &ToolCtx) -> Result<HandoffDir, String> {
    let session = ctx
        .mgr
        .db()
        .get_session(ctx.session_id)
        .map_err(|e| e.to_string())?;
    let root = PathBuf::from(&session.worktree_path)
        .join(".galvolite")
        .join("handoffs");
    Ok(HandoffDir::new(root))
}

// ---------------------------------------------------------------------------
// set_role
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SetRoleArgs {
    role: String,
}

async fn set_role(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SetRoleArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !ROLES.contains(&parsed.role.as_str()) {
        return ToolOutcome::InvalidParams(format!("unknown role: {}", parsed.role));
    }

    // 1. Event + sessions.role update atomically (state::apply_column_side_effects).
    if let Err(e) = state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.set_role".into(),
            payload: json!({ "role": parsed.role }),
            dedup_key: None,
        },
    ) {
        return ToolOutcome::AppError(e.to_string());
    }

    // 2. Rewrite CLAUDE.md so next turn loads the new profile. Today only
    // `implementation_profile()` exists in claude_md; until other profiles
    // ship, prepend a role-marker header so callers (and tests) can see the
    // active role from disk.
    let session = match ctx.mgr.db().get_session(ctx.session_id) {
        Ok(s) => s,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    let body = format!(
        "# CLAUDE.md — role: {}\n\n{}",
        parsed.role,
        crate::claude_md::implementation_profile()
    );
    let path = PathBuf::from(&session.worktree_path).join("CLAUDE.md");
    if let Err(e) = std::fs::write(&path, body) {
        // Don't roll the role back — the model still sees the new role on
        // next turn via apply_event; CLAUDE.md will be retried at next
        // set_role / spawn.
        tracing::warn!(
            "set_role: failed to rewrite CLAUDE.md at {:?}: {}",
            path, e
        );
    }

    ToolOutcome::Ok(json!({ "role": parsed.role }))
}

// ---------------------------------------------------------------------------
// write_handoff
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WriteHandoffArgs {
    body: String,
}

async fn write_handoff(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: WriteHandoffArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if let Err(e) = handoff::validate_handoff_content(&parsed.body) {
        return ToolOutcome::InvalidParams(e.to_string());
    }

    let dir = match handoff_dir_for(&ctx) {
        Ok(d) => d,
        Err(e) => return ToolOutcome::AppError(e),
    };
    if let Err(e) = handoff::ensure_session_dir(&dir, ctx.session_id) {
        return ToolOutcome::AppError(e.to_string());
    }
    let path = handoff::handoff_path(&dir, ctx.session_id, Utc::now());
    if let Err(e) = std::fs::write(&path, &parsed.body) {
        return ToolOutcome::AppError(format!("write handoff: {e}"));
    }

    // Event + sessions.handoff_path update atomically.
    let path_str = path.to_string_lossy().into_owned();
    if let Err(e) = state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.write_handoff".into(),
            payload: json!({ "path": &path_str }),
            dedup_key: None,
        },
    ) {
        return ToolOutcome::AppError(e.to_string());
    }
    ToolOutcome::Ok(json!({ "path": path_str }))
}

// ---------------------------------------------------------------------------
// read_handoff
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ReadHandoffArgs {
    path: Option<String>,
}

async fn read_handoff(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: ReadHandoffArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    // Logging-only event row (no state change).
    let _ = state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.read_handoff".into(),
            payload: json!({ "path": parsed.path }),
            dedup_key: None,
        },
    );

    let path = match parsed.path {
        Some(p) => PathBuf::from(p),
        None => {
            let dir = match handoff_dir_for(&ctx) {
                Ok(d) => d,
                Err(e) => return ToolOutcome::AppError(e),
            };
            match handoff::latest_handoff_for(&dir, ctx.session_id) {
                Ok(Some(p)) => p,
                Ok(None) => {
                    return ToolOutcome::Ok(json!({ "content": null, "path": null }))
                }
                Err(e) => return ToolOutcome::AppError(e.to_string()),
            }
        }
    };
    match handoff::read_handoff(&path) {
        Ok(body) => ToolOutcome::Ok(json!({
            "content": body,
            "path": path.to_string_lossy(),
        })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}
