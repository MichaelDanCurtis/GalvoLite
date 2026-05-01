//! Sub-plan 6 MCP tools: spawn_session, wait_children. See spec §3.1.

use crate::api::events::DomainEvent;
use crate::mcp::registry::{Tier, ToolCtx, ToolDef, ToolOutcome};
use serde::Deserialize;
use serde_json::{json, Value};

/// Roles the DB schema allows (matches CHECK constraint in 0001_initial.sql).
const ALLOWED_ROLES: &[&str] = &["implementation", "brainstorm", "scout", "fix"];

const SOFT_CAP_CONCURRENCY: i64 = 4;
const SOFT_CAP_DEPTH: i64 = 3;

pub fn tools() -> Vec<ToolDef> {
    let allowed_roles_value: Value = ALLOWED_ROLES.iter().copied().collect();
    vec![
        ToolDef {
            name: "spawn_session",
            description: "Spawn a child session in its own git worktree. Returns the child's session_id, worktree_path, and branch.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "role": { "type": "string", "enum": allowed_roles_value },
                    "task": { "type": "string" }
                },
                "required": ["role", "task"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(spawn_session(ctx, args))),
        },
        ToolDef {
            name: "wait_children",
            description: "Park this session until every listed child reaches terminal status. Returns immediately; rejoin is delivered via additionalContext on the next prompt.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "children": {
                        "type": "array",
                        "items": { "type": "string", "format": "uuid" }
                    }
                },
                "required": ["children"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(wait_children(ctx, args))),
        },
    ]
}

#[derive(Deserialize)]
struct SpawnArgs {
    role: String,
    task: String,
}

async fn spawn_session(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SpawnArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !ALLOWED_ROLES.contains(&parsed.role.as_str()) {
        return ToolOutcome::AppError(format!("unknown role: {}", parsed.role));
    }

    let child = match ctx.mgr.spawn_child(ctx.session_id, &parsed.role, &parsed.task).await {
        Ok(c) => c,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };

    // Soft-cap: warn if project has too many active sessions.
    if let Ok(active) = active_count_for_project(ctx.mgr.db(), child.project_id) {
        if active > SOFT_CAP_CONCURRENCY {
            ctx.mgr.event_bus().publish(DomainEvent::OperatorToast {
                session_id: ctx.session_id,
                level: "warn".into(),
                message: format!(
                    "Project active sessions = {active} (soft cap {SOFT_CAP_CONCURRENCY})"
                ),
            });
        }
    }

    // Soft-cap: warn if spawn depth is too large.
    if let Ok(depth) = walk_depth(ctx.mgr.db(), child.id) {
        if depth > SOFT_CAP_DEPTH {
            ctx.mgr.event_bus().publish(DomainEvent::OperatorToast {
                session_id: ctx.session_id,
                level: "warn".into(),
                message: format!("Spawn depth = {depth} (soft cap {SOFT_CAP_DEPTH})"),
            });
        }
    }

    ToolOutcome::Ok(json!({
        "session_id":    child.id,
        "worktree_path": child.worktree_path,
        "branch":        child.branch,
    }))
}

fn active_count_for_project(
    db: &crate::db::Db,
    project_id: uuid::Uuid,
) -> crate::error::Result<i64> {
    let conn = db.pool().get()?;
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sessions WHERE project_id = ?1 AND ended_at IS NULL",
        [project_id.to_string()],
        |r| r.get(0),
    )?;
    Ok(n)
}

fn walk_depth(db: &crate::db::Db, session_id: uuid::Uuid) -> crate::error::Result<i64> {
    let conn = db.pool().get()?;
    let mut depth = 0i64;
    let mut current = session_id.to_string();
    while let Some(parent) = conn
        .query_row(
            "SELECT parent_session_id FROM sessions WHERE id = ?1",
            [current.as_str()],
            |r| r.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
    {
        depth += 1;
        if depth > 10 {
            break;
        }
        current = parent;
    }
    Ok(depth)
}

async fn wait_children(_ctx: ToolCtx, _args: Value) -> ToolOutcome {
    ToolOutcome::AppError("not implemented yet".into())
}
