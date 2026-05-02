//! Tier-2/3 MCP tools: cross-session reads + writes + operator surface.
//!
//! Spec §5.3 (cross-session) and §5.4 (operator). `ask_operator` parks the
//! model turn on a oneshot registered with the SessionManager's
//! `OperatorPromptRegistry`, publishes an `operator.prompt` domain event for
//! subscribed dashboard sockets, and resolves when either an operator answer
//! arrives via `POST /api/operator/prompt/:id/answer` or the per-call timeout
//! fires (returning `{answer:null, timed_out:true}`). `notify_operator` is a
//! one-shot toast event — event row + `operator.toast` broadcast.

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
    let _ = state::apply_event_with_bus(
        ctx.mgr.db(),
        target,
        ctx.mgr.event_bus(),
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
// ask_operator
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

    // Durable record: the question was asked. Landed BEFORE the live prompt
    // event so event-row order matches causal order (ask row precedes any
    // answer row a future operator action might write).
    let _ = state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
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

    // Look up project so we can pass path + id into register_and_wait for
    // decision auto-recording.
    let session = match ctx.mgr.db().get_session(ctx.session_id) {
        Ok(s) => s,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    let project = match ctx.mgr.get_project(session.project_id) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };

    // Live wait: publishes an `operator.prompt` domain event and parks on a
    // oneshot until the HTTP answer lands (or the per-call timeout fires).
    // The registry returns `{answer, timed_out}` directly; we pass it through
    // unchanged and let the transport layer wrap it in content[0].text.
    let answer = ctx
        .mgr
        .operator_registry()
        .register_and_wait(
            ctx.mgr.event_bus(),
            ctx.session_id,
            project.id,
            std::path::PathBuf::from(&project.path),
            parsed.question.clone(),
            parsed.choices.clone(),
            parsed.timeout_secs,
        )
        .await;

    ToolOutcome::Ok(answer)
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

    let _ = state::apply_event_with_bus(
        ctx.mgr.db(),
        ctx.session_id,
        ctx.mgr.event_bus(),
        EventInput {
            source: Source::Mcp,
            kind: "mcp.notify_operator".into(),
            payload: json!({ "level": parsed.level, "message": parsed.message }),
            dedup_key: None,
        },
    );

    // Sub-plan 4: fan out to dashboard sockets as an operator.toast frame.
    ctx.mgr
        .event_bus()
        .publish(crate::api::events::DomainEvent::OperatorToast {
            session_id: ctx.session_id,
            level: parsed.level.clone(),
            message: parsed.message.clone(),
        });

    ToolOutcome::Ok(json!({ "ok": true }))
}
