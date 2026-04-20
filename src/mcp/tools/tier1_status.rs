//! Tier-1 MCP tools: status & lifecycle.
//!
//! Implements spec §5.1: `status_report`, `request_attention`, `set_plan`,
//! `plan_check_step`, `set_seed`. All handlers funnel through
//! [`crate::state::apply_event`]; column mutations (plan_markdown,
//! seed_markdown, last_status_note) happen inside the same SQLite
//! transaction.

use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use crate::state::{self, EventInput, Source};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "status_report",
            description:
                "Report current work status. kind=working|complete|blocked. Optional note.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["working", "complete", "blocked"] },
                    "note": { "type": "string" }
                },
                "required": ["kind"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(status_report(ctx, args))),
        },
        ToolDef {
            name: "request_attention",
            description:
                "Flag this session as needing operator attention with a reason string.",
            input_schema: json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(request_attention(ctx, args))),
        },
        ToolDef {
            name: "set_plan",
            description:
                "Replace the session's plan markdown. Checkbox steps are extracted by the dashboard.",
            input_schema: json!({
                "type": "object",
                "properties": { "markdown": { "type": "string" } },
                "required": ["markdown"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(set_plan(ctx, args))),
        },
        ToolDef {
            name: "plan_check_step",
            description:
                "Update one plan step's status by title (preferred) or step_hash. status: pending|working|done|blocked.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title":     { "type": "string" },
                    "step_hash": { "type": "string" },
                    "status":    { "type": "string", "enum": ["pending","working","done","blocked"] },
                    "note":      { "type": "string" }
                },
                "required": ["status"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(plan_check_step(ctx, args))),
        },
        ToolDef {
            name: "set_seed",
            description:
                "Replace the session's seed (the markdown injected at SessionStart for resumes).",
            input_schema: json!({
                "type": "object",
                "properties": { "markdown": { "type": "string" } },
                "required": ["markdown"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(set_seed(ctx, args))),
        },
    ]
}

// ---------------------------------------------------------------------------
// status_report
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StatusReportArgs {
    kind: String,
    note: Option<String>,
}

async fn status_report(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: StatusReportArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !matches!(parsed.kind.as_str(), "working" | "complete" | "blocked") {
        return ToolOutcome::InvalidParams(format!("unknown kind: {}", parsed.kind));
    }
    let payload = json!({ "kind": parsed.kind, "note": parsed.note });
    match state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.status_report".into(),
            payload,
            dedup_key: None,
        },
    ) {
        Ok(o) => ToolOutcome::Ok(json!({ "state": o.next_state.to_string() })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// request_attention
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RequestAttentionArgs {
    reason: String,
}

async fn request_attention(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: RequestAttentionArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    match state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.request_attention".into(),
            payload: json!({ "reason": parsed.reason }),
            dedup_key: None,
        },
    ) {
        Ok(o) => ToolOutcome::Ok(json!({ "state": o.next_state.to_string() })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// set_plan
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SetPlanArgs {
    markdown: String,
}

async fn set_plan(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SetPlanArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    match state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.set_plan".into(),
            payload: json!({ "markdown": parsed.markdown }),
            dedup_key: None,
        },
    ) {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// plan_check_step
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PlanCheckStepArgs {
    title: Option<String>,
    step_hash: Option<String>,
    status: String,
    note: Option<String>,
}

async fn plan_check_step(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: PlanCheckStepArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if parsed.title.is_none() && parsed.step_hash.is_none() {
        return ToolOutcome::InvalidParams("must provide title or step_hash".into());
    }
    if !matches!(
        parsed.status.as_str(),
        "pending" | "working" | "done" | "blocked"
    ) {
        return ToolOutcome::InvalidParams(format!("unknown status: {}", parsed.status));
    }
    match state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.plan_check_step".into(),
            payload: json!({
                "title":     parsed.title,
                "step_hash": parsed.step_hash,
                "status":    parsed.status,
                "note":      parsed.note,
            }),
            dedup_key: None,
        },
    ) {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// set_seed
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SetSeedArgs {
    markdown: String,
}

async fn set_seed(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SetSeedArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    match state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.set_seed".into(),
            payload: json!({ "markdown": parsed.markdown }),
            dedup_key: None,
        },
    ) {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}
