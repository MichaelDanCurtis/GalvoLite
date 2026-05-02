//! `write_postmortem` MCP tool. Persists a postmortem to
//! `<project.path>/.galvolite/postmortems/part-<N>-<sid>.md` so future
//! executors on the same plan part get seeded with prior context.

use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use crate::postmortem::{self, Outcome, Postmortem};
use crate::state::{self, EventInput, Source};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "write_postmortem",
        description: "Write a postmortem for a finished plan part. The harness uses these to brief future executors on the same part.",
        input_schema: json!({
            "type": "object",
            "required": ["plan_part", "outcome", "cycles", "duration_seconds"],
            "properties": {
                "plan_part": { "type": "integer", "minimum": 0 },
                "outcome": { "enum": ["shipped", "stuck", "escalated"] },
                "cycles": { "type": "integer", "minimum": 0 },
                "duration_seconds": { "type": "integer", "minimum": 0 },
                "what_worked": { "type": "string", "default": "" },
                "gotchas": { "type": "string", "default": "" },
                "follow_ups": { "type": "string", "default": "" }
            }
        }),
        tier: Tier::SelfContained,
        handler: Box::new(|ctx, args| Box::pin(write_postmortem(ctx, args))),
    }]
}

#[derive(Deserialize)]
struct Args {
    plan_part: u32,
    outcome: Outcome,
    cycles: u32,
    duration_seconds: u64,
    #[serde(default)]
    what_worked: String,
    #[serde(default)]
    gotchas: String,
    #[serde(default)]
    follow_ups: String,
}

async fn write_postmortem(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: Args = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    // Look up project for this session.
    let session = match ctx.mgr.db().get_session(ctx.session_id) {
        Ok(s) => s,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    let project = match ctx.mgr.get_project(session.project_id) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };

    let pm = Postmortem {
        plan_part: parsed.plan_part,
        session_id: ctx.session_id,
        outcome: parsed.outcome,
        cycles: parsed.cycles,
        duration_seconds: parsed.duration_seconds,
        written_at: Utc::now(),
        what_worked: parsed.what_worked,
        gotchas: parsed.gotchas,
        follow_ups: parsed.follow_ups,
    };

    let path = match postmortem::write(std::path::Path::new(&project.path), &pm) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::AppError(format!("write postmortem: {e}")),
    };
    let path_str = path.to_string_lossy().into_owned();

    // Record the event row (no column side-effect).
    if let Err(e) = state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.write_postmortem".into(),
            payload: json!({ "plan_part": pm.plan_part, "path": &path_str }),
            dedup_key: None,
        },
    ) {
        return ToolOutcome::AppError(e.to_string());
    }

    // Project-scoped fan-out for the dashboard.
    ctx.mgr.event_bus().publish(crate::api::events::DomainEvent::PostmortemWritten {
        project_id: project.id,
        plan_part: pm.plan_part,
        path: path_str.clone(),
    });

    ToolOutcome::Ok(json!({ "path": path_str }))
}
