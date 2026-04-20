//! Tier-2/3 MCP tools: cross-session reads + writes + operator surface.
//!
//! Spec §5.3 (cross-session) and §5.4 (operator). The operator tools
//! (`ask_operator`, `notify_operator`) ship in DEGRADED MODE — they write
//! event rows today but the dashboard WS round-trip lands in Sub-plan 4.
//! `ask_operator` returns `{answer:null, timed_out:true, degraded:true}`
//! immediately so the model never hangs.

use crate::error::Error;
use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use crate::state::{self, EventInput, Source};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_siblings",
            description: "List sibling sessions in the same project (or all projects if project_id omitted).",
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": { "type": "string" } }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(list_siblings(ctx, args))),
        },
        ToolDef {
            name: "status_of",
            description: "Get the full state snapshot of any session by id.",
            input_schema: json!({
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(status_of(ctx, args))),
        },
        ToolDef {
            name: "read_events",
            description: "Read event log entries. Defaults to current session; pass session_id to read another's.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "since":      { "type": "string" },
                    "limit":      { "type": "integer", "minimum": 1, "maximum": 500 }
                }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(read_events(ctx, args))),
        },
        ToolDef {
            name: "send_to_session",
            description: "Write text into another session's PTY. submit=true appends \\r so the receiver treats it as Enter.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "text":       { "type": "string" },
                    "submit":     { "type": "boolean" }
                },
                "required": ["session_id", "text"]
            }),
            tier: Tier::CrossSessionPty,
            handler: Box::new(|ctx, args| Box::pin(send_to_session(ctx, args))),
        },
        ToolDef {
            name: "ask_operator",
            description: "Ask the operator a question and wait for an answer. Returns {answer, timed_out}. (Degraded: returns timed_out=true immediately until Sub-plan 4 wires the dashboard prompt UI.)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question":     { "type": "string" },
                    "choices":      { "type": "array", "items": { "type": "string" } },
                    "timeout_secs": { "type": "integer", "minimum": 1 }
                },
                "required": ["question"]
            }),
            tier: Tier::DashboardDegraded,
            handler: Box::new(|ctx, args| Box::pin(ask_operator(ctx, args))),
        },
        ToolDef {
            name: "notify_operator",
            description: "Push a non-blocking notification to the operator. (Event row written today; dashboard toast lands in Sub-plan 4.)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "level":   { "type": "string", "enum": ["info","warn","urgent"] },
                    "message": { "type": "string" }
                },
                "required": ["level", "message"]
            }),
            tier: Tier::DashboardDegraded,
            handler: Box::new(|ctx, args| Box::pin(notify_operator(ctx, args))),
        },
    ]
}

// ---------------------------------------------------------------------------
// list_siblings
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct ListSiblingsArgs {
    project_id: Option<String>,
}

async fn list_siblings(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: ListSiblingsArgs = serde_json::from_value(args).unwrap_or_default();

    // Resolve target project: explicit arg overrides; otherwise the caller's project.
    let project_id = match parsed.project_id {
        Some(p) => match Uuid::parse_str(&p) {
            Ok(u) => u,
            Err(_) => return ToolOutcome::InvalidParams("project_id not a uuid".into()),
        },
        None => match ctx.mgr.db().get_session(ctx.session_id) {
            Ok(s) => s.project_id,
            Err(e) => return ToolOutcome::AppError(e.to_string()),
        },
    };

    match ctx
        .mgr
        .db()
        .list_session_siblings(project_id, Some(ctx.session_id))
    {
        Ok(rows) => ToolOutcome::Ok(json!({ "siblings": rows })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// status_of
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StatusOfArgs {
    session_id: String,
}

async fn status_of(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: StatusOfArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let target = match Uuid::parse_str(&parsed.session_id) {
        Ok(u) => u,
        Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
    };
    match ctx.mgr.db().get_session(target) {
        Ok(s) => ToolOutcome::Ok(json!({
            "id":         s.id.to_string(),
            "project_id": s.project_id.to_string(),
            "role":       s.role,
            "branch":     s.branch,
            "worktree":   s.worktree_path,
            "state":      format!("{:?}", s.state),
            "autopilot":  s.autopilot,
            "parent_id":  s.parent_session_id.map(|u| u.to_string()),
            "task_summary": s.task_summary,
            "created_at": s.created_at.to_rfc3339(),
            "ended_at":   s.ended_at.map(|d| d.to_rfc3339()),
        })),
        Err(Error::SessionNotFound(_)) => {
            ToolOutcome::AppError(format!("session not found: {target}"))
        }
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// read_events
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct ReadEventsArgs {
    session_id: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
}

async fn read_events(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: ReadEventsArgs = serde_json::from_value(args).unwrap_or_default();
    let target = match parsed.session_id {
        Some(s) => match Uuid::parse_str(&s) {
            Ok(u) => u,
            Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
        },
        None => ctx.session_id,
    };
    let limit = parsed.limit.unwrap_or(50);
    match ctx
        .mgr
        .db()
        .read_events_for_session(target, parsed.since.as_deref(), limit)
    {
        Ok(events) => ToolOutcome::Ok(json!({ "events": events })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// send_to_session
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SendToSessionArgs {
    session_id: String,
    text: String,
    submit: Option<bool>,
}

async fn send_to_session(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SendToSessionArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let target = match Uuid::parse_str(&parsed.session_id) {
        Ok(u) => u,
        Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
    };
    let submit = parsed.submit.unwrap_or(false);

    // Write event row against the TARGET session so it shows up in the
    // receiver's event log. Best-effort: if the target row is gone we still
    // attempt the PTY write below (which will then return SessionNotFound).
    let _ = state::apply_event(
        ctx.mgr.db(),
        target,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.send_to_session".into(),
            payload: json!({
                "from": ctx.session_id.to_string(),
                "text": parsed.text,
                "submit": submit,
            }),
            dedup_key: None,
        },
    );

    let mut payload_bytes = parsed.text.into_bytes();
    if submit {
        payload_bytes.push(b'\r');
    }

    match ctx.mgr.write_pty(target, payload_bytes).await {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true, "delivered": "pty" })),
        // Target session has no live PTY (DB-only seeded session, or it ended
        // already). The event row is still written above so the operator sees
        // the attempt; surface a structured outcome rather than a hard error.
        Err(Error::SessionNotRunning(_)) => ToolOutcome::Ok(json!({
            "ok": true,
            "delivered": "event_only",
            "note": "target session has no running PTY; event row written for audit",
        })),
        Err(Error::SessionNotFound(_)) => {
            ToolOutcome::AppError(format!("session not found: {target}"))
        }
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// ask_operator (degraded)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct AskOperatorArgs {
    question: String,
    choices: Option<Vec<String>>,
    timeout_secs: Option<u64>,
}

async fn ask_operator(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: AskOperatorArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    let _ = state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.ask_operator".into(),
            payload: json!({
                "question":     parsed.question,
                "choices":      parsed.choices,
                "timeout_secs": parsed.timeout_secs,
            }),
            dedup_key: None,
        },
    );

    // DEGRADED MODE — Sub-plan 4 wires the WS prompt round-trip; until then
    // we return an immediate timed-out reply so the model doesn't hang.
    tracing::warn!(
        session = %ctx.session_id,
        "ask_operator called in degraded mode (Sub-plan 4 not landed) — \
         returning timed_out immediately"
    );

    ToolOutcome::Ok(json!({
        "answer":    serde_json::Value::Null,
        "timed_out": true,
        "degraded":  true,
    }))
}

// ---------------------------------------------------------------------------
// notify_operator (degraded)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct NotifyOperatorArgs {
    level: String,
    message: String,
}

async fn notify_operator(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: NotifyOperatorArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !matches!(parsed.level.as_str(), "info" | "warn" | "urgent") {
        return ToolOutcome::InvalidParams(format!("unknown level: {}", parsed.level));
    }

    let _ = state::apply_event(
        ctx.mgr.db(),
        ctx.session_id,
        EventInput {
            source: Source::Mcp,
            kind: "mcp.notify_operator".into(),
            payload: json!({ "level": parsed.level, "message": parsed.message }),
            dedup_key: None,
        },
    );

    // Sub-plan 4 fans this out as a dashboard toast via WS. Today: event row only.
    tracing::warn!(
        session = %ctx.session_id,
        "notify_operator called in degraded mode (Sub-plan 4 not landed) — \
         event row written, no dashboard toast"
    );

    ToolOutcome::Ok(json!({ "ok": true, "degraded": true }))
}
