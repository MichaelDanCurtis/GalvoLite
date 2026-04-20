//! Tier-4 reserved stubs (spec §5.2–§5.6).
//!
//! Each stub is a real `ToolDef` with full input schema and a descriptor
//! suffixed `(RESERVED — Plan X)`. Handlers return
//! `ToolOutcome::AppError("not_implemented: <name> — owned by …")` so the
//! model recognizes the pattern and can ask the operator for the right path.
//!
//! Why register stubs at all: the model sees the complete catalog at session
//! start, so future-Claude won't try to invent ad-hoc replacements for things
//! we plan to ship.

use crate::mcp::registry::{Tier, ToolDef, ToolOutcome};
use serde_json::{json, Value};

/// Build one reserved-stub `ToolDef`. Kept as a function (rather than a macro)
/// because the description suffix and the `not_implemented` text need to
/// reference the same `name` + `owner_plan` strings, and the handler closure
/// captures them via a `&'static str` move.
fn stub(
    name: &'static str,
    description: &'static str,
    schema: Value,
    owner_plan: &'static str,
) -> ToolDef {
    // Build a `&'static str` description by leaking a heap-allocated String.
    // ToolDef requires &'static str fields and we generate one descriptor
    // per stub at startup (constant N), so a one-time leak per registry build
    // is acceptable.
    let desc: &'static str = Box::leak(
        format!("{description} (RESERVED — {owner_plan})").into_boxed_str(),
    );

    ToolDef {
        name,
        description: desc,
        input_schema: schema,
        tier: Tier::ReservedStub,
        handler: Box::new(move |_ctx, _args| {
            Box::pin(async move {
                ToolOutcome::AppError(format!(
                    "not_implemented: {name} — owned by {owner_plan}"
                ))
            })
        }),
    }
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        // §5.2 Path ownership — Plan 4
        stub(
            "claim_paths",
            "Claim file globs as exclusively-owned by this session.",
            json!({
                "type": "object",
                "properties": {
                    "globs": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["globs"]
            }),
            "Plan 4 (spawn/rejoin)",
        ),
        stub(
            "release_paths",
            "Release previously-claimed file globs.",
            json!({
                "type": "object",
                "properties": {
                    "globs": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["globs"]
            }),
            "Plan 4 (spawn/rejoin)",
        ),
        stub(
            "list_claims",
            "List active path claims across all sessions.",
            json!({ "type": "object", "properties": {} }),
            "Plan 4 (spawn/rejoin)",
        ),
        stub(
            "check_path",
            "Check if a path is claimed and by whom.",
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            "Plan 4 (spawn/rejoin)",
        ),
        // §5.3 Spawn — Plan 4
        stub(
            "spawn_child",
            "Spawn a child session with the given role + task.",
            json!({
                "type": "object",
                "properties": {
                    "role":       { "type": "string" },
                    "task":       { "type": "string" },
                    "depends_on": { "type": "array", "items": { "type": "string" } },
                    "worktree":   { "type": "string" }
                },
                "required": ["role", "task"]
            }),
            "Plan 4 (spawn/rejoin)",
        ),
        stub(
            "wait_for_children",
            "Block until the listed child sessions transition to Ended.",
            json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["ids"]
            }),
            "Plan 4 (spawn/rejoin)",
        ),
        // §5.4 Portfolio — Plan 5+
        stub(
            "list_available_mcps",
            "List MCP servers available from the configured portfolio (e.g. Docker MCP Gateway).",
            json!({ "type": "object", "properties": {} }),
            "Plan 5+ (MCP portfolio)",
        ),
        stub(
            "list_session_mcps",
            "List MCP servers currently configured for a session.",
            json!({
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }),
            "Plan 5+ (MCP portfolio)",
        ),
        stub(
            "configure_session_mcps",
            "Configure the MCP servers for a future spawn of a session profile.",
            json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "servers":    { "type": "array", "items": { "type": "string" } }
                },
                "required": ["session_id", "servers"]
            }),
            "Plan 5+ (MCP portfolio)",
        ),
        stub(
            "gateway_toggle",
            "Toggle a server on the Docker MCP Gateway.",
            json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" },
                    "enable": { "type": "boolean" }
                },
                "required": ["server", "enable"]
            }),
            "Plan 5+ (MCP portfolio)",
        ),
        // §5.5 GitHub — Plan 5+
        stub(
            "open_pr",
            "Open a GitHub pull request from the session's worktree.",
            json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "body":  { "type": "string" }
                },
                "required": ["title"]
            }),
            "Plan 5+ (GitHub surface)",
        ),
        stub(
            "list_prs",
            "List open pull requests for the project repo.",
            json!({ "type": "object", "properties": {} }),
            "Plan 5+ (GitHub surface)",
        ),
        stub(
            "request_review",
            "Request review on a PR.",
            json!({
                "type": "object",
                "properties": { "pr": { "type": "integer" } },
                "required": ["pr"]
            }),
            "Plan 5+ (GitHub surface)",
        ),
        stub(
            "merge_pr",
            "Merge a PR.",
            json!({
                "type": "object",
                "properties": { "pr": { "type": "integer" } },
                "required": ["pr"]
            }),
            "Plan 5+ (GitHub surface)",
        ),
        stub(
            "get_pr_comments",
            "Get comments on a PR.",
            json!({
                "type": "object",
                "properties": { "pr": { "type": "integer" } },
                "required": ["pr"]
            }),
            "Plan 5+ (GitHub surface)",
        ),
        // §5.6 Context r/w — capability-blocked
        stub(
            "read_context",
            "Read this session's full conversation context.",
            json!({ "type": "object", "properties": {} }),
            "capability-blocked (PTY can't surface context; would require Agent SDK migration)",
        ),
        stub(
            "write_context",
            "Write text directly into this session's context.",
            json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
            "capability-blocked (PTY can't surface context; would require Agent SDK migration)",
        ),
    ]
}
