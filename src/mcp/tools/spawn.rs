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
                    "task": { "type": "string" },
                    "plan_part": { "type": "integer", "minimum": 0 }
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
    #[serde(default)]
    plan_part: Option<u32>,
}

async fn spawn_session(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SpawnArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !ALLOWED_ROLES.contains(&parsed.role.as_str()) {
        return ToolOutcome::AppError(format!("unknown role: {}", parsed.role));
    }

    let child = match ctx.mgr.spawn_child(ctx.session_id, &parsed.role, &parsed.task, parsed.plan_part).await {
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

#[derive(Deserialize)]
struct WaitArgs {
    children: Vec<String>,
}

async fn wait_children(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: WaitArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let mut child_ids = Vec::with_capacity(parsed.children.len());
    for s in &parsed.children {
        match uuid::Uuid::parse_str(s) {
            Ok(u) => child_ids.push(u),
            Err(_) => return ToolOutcome::InvalidParams(format!("not a uuid: {s}")),
        }
    }

    // Validate ownership + check whether all are already terminal.
    let conn = match ctx.mgr.db().pool().get() {
        Ok(c) => c,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    let mut all_terminal = true;
    for cid in &child_ids {
        let row: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT parent_session_id, terminal_status FROM sessions WHERE id = ?1",
                [cid.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let Some((parent_str, term)) = row else {
            return ToolOutcome::AppError(format!("unknown_child: {cid}"));
        };
        if parent_str.as_deref() != Some(&ctx.session_id.to_string()) {
            return ToolOutcome::AppError(format!("not_owned_child: {cid}"));
        }
        if term.is_none() {
            all_terminal = false;
        }
    }
    drop(conn);

    if all_terminal {
        let md = match ctx.mgr.build_rejoin_markdown(ctx.session_id, &child_ids).await {
            Ok(m) => m,
            Err(e) => return ToolOutcome::AppError(e.to_string()),
        };
        let conn = ctx.mgr.db().pool().get().unwrap();
        let _ = conn.execute(
            "UPDATE sessions SET pending_rejoin = ?1 WHERE id = ?2",
            rusqlite::params![md, ctx.session_id.to_string()],
        );
        drop(conn);
        return ToolOutcome::Ok(json!({"ok": true, "immediate_rejoin": true}));
    }

    if let Err(e) = ctx.mgr.set_waiting_on(ctx.session_id, &child_ids).await {
        return ToolOutcome::AppError(e.to_string());
    }
    ToolOutcome::Ok(json!({"ok": true, "immediate_rejoin": false}))
}
