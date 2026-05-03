//! `UserPromptSubmit`, `Stop`, `PreCompact` hook handlers.
//!
//! - `UserPromptSubmit`: bump `sessions.turn_count` and `sessions.est_tokens`,
//!   inject a context-warning at ≥80% of the assumed limit.
//! - `Stop`: count consecutive `hook.Stop` events since the last
//!   `mcp.status_report` or `hook.UserPromptSubmit`. <3 → inject
//!   "report status or continue"; ≥3 → emit `system.stop_max_retries`
//!   so the funnel transitions Working → Idle.
//! - `PreCompact`: inject "write_handoff now" if no recent `mcp.write_handoff`.
//!   Logs a `tracing::warn!` flagging spec §15 dep #3 (inject_text capability
//!   in PreCompact is unverified pending Sub-plan 5 real-claude smoke).

use crate::db::Db;
use uuid::Uuid;

const CONTEXT_LIMIT: i64 = 32_000;
const STOP_RETRY_LIMIT: i64 = 3;

pub fn on_user_prompt_submit(
    db: &Db,
    session_id: Uuid,
    payload: &serde_json::Value,
) -> Option<String> {
    let est = payload
        .get("est_tokens")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    // Bump counters. The hook event row was already written by /hooks/ingest;
    // these are best-effort counter updates. If the SQL fails we still want
    // to return None rather than panic — the hook is fail-soft.
    let conn = db.pool().get().ok()?;
    let _ = conn.execute(
        "UPDATE sessions SET turn_count = COALESCE(turn_count,0)+1,
                              est_tokens = COALESCE(est_tokens,0)+?2
         WHERE id = ?1",
        rusqlite::params![session_id.to_string(), est],
    );

    let total: i64 = conn
        .query_row(
            "SELECT COALESCE(est_tokens, 0) FROM sessions WHERE id = ?1",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let warn = if total * 100 >= CONTEXT_LIMIT * 80 {
        Some(format!(
            "Context approaching limit ({total}/{CONTEXT_LIMIT} tokens, {}%). \
             Consider write_handoff before context runs out.",
            (total * 100) / CONTEXT_LIMIT
        ))
    } else {
        None
    };

    // Drain pending_overseer_recs for the project's orchestrator session.
    //
    // "Orchestrator" is NOT stored as a role — the DB CHECK constraint at
    // migrations/0001_initial.sql:20 forbids it (allowed roles are
    // implementation/brainstorm/scout/fix). Instead we use the same
    // top-level-session heuristic as `SessionManager::find_orchestrator_for_project`:
    // most recent non-terminal session for this project with no parent.
    // If THIS session is that orchestrator, drain its pending recs into
    // inject_text and clear the field.
    let orch_id: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions
             WHERE project_id = (SELECT project_id FROM sessions WHERE id = ?1)
               AND parent_session_id IS NULL
               AND ended_at IS NULL
             ORDER BY created_at DESC
             LIMIT 1",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .ok();

    if orch_id.as_deref() == Some(session_id.to_string().as_str()) {
        let pending: Option<String> = conn
            .query_row(
                "SELECT pending_overseer_recs FROM sessions WHERE id = ?1",
                [session_id.to_string()],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if let Some(block) = pending {
            let _ = conn.execute(
                "UPDATE sessions SET pending_overseer_recs = NULL WHERE id = ?1",
                [session_id.to_string()],
            );
            let combined = match warn {
                Some(w) => format!("{w}\n\n{block}"),
                None => block,
            };
            return Some(combined);
        }
    }

    warn
}

pub fn on_stop(
    db: &Db,
    session_id: Uuid,
    bus: &crate::api::events::EventBroadcaster,
) -> Option<String> {
    let conn = db.pool().get().ok()?;
    // Count consecutive hook.Stop events since the last reset event
    // (status_report or UserPromptSubmit). The current hook.Stop has
    // already been written by /hooks/ingest, so it's included.
    let mut stmt = conn
        .prepare("SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC, id DESC LIMIT 20")
        .ok()?;
    let kinds: Vec<String> = stmt
        .query_map([session_id.to_string()], |r| r.get(0))
        .ok()?
        .filter_map(Result::ok)
        .collect();

    let mut consecutive = 0i64;
    for k in &kinds {
        match k.as_str() {
            "hook.Stop" => consecutive += 1,
            "mcp.status_report" | "hook.UserPromptSubmit" => break,
            _ => continue, // skip unrelated events
        }
    }

    if consecutive >= STOP_RETRY_LIMIT {
        // Transition to Idle by emitting a synthetic system event.
        let _ = crate::state::apply_event_with_bus(
            db,
            session_id,
            bus,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "system.stop_max_retries".into(),
                payload: serde_json::json!({"consecutive": consecutive}),
                dedup_key: None,
            },
        );
        None
    } else {
        // Schedule a 3 s watchdog: if no UserPromptSubmit arrives, mark
        // the session AwaitingInput. The watchdog re-reads the events
        // table and only fires if hook.Stop is still the most recent
        // event for this session — a quick UserPromptSubmit before the
        // 3 s mark naturally cancels it.
        let db_for_task = db.clone();
        let bus_for_task = bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let conn = match db_for_task.pool().get() {
                Ok(c) => c,
                Err(_) => return,
            };
            let last_kind: Option<String> = conn
                .query_row(
                    "SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC, id DESC LIMIT 1",
                    [session_id.to_string()],
                    |r| r.get(0),
                )
                .ok();
            if last_kind.as_deref() == Some("hook.Stop") {
                let _ = crate::state::apply_event_with_bus(
                    &db_for_task,
                    session_id,
                    &bus_for_task,
                    crate::state::EventInput {
                        source: crate::state::Source::System,
                        kind: "system.awaiting_input".into(),
                        payload: serde_json::json!({}),
                        dedup_key: Some(format!(
                            "awaiting:{session_id}:{}",
                            chrono::Utc::now().timestamp_millis()
                        )),
                    },
                );
            }
        });
        Some("Please report your status (status_report) or continue working.".into())
    }
}

pub fn on_pre_compact(db: &Db, session_id: Uuid) -> Option<String> {
    tracing::warn!(
        "PreCompact inject_text capability unverified — see spec §15 dep #3"
    );

    let conn = db.pool().get().ok()?;
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM (
                SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT 50
             ) WHERE kind = 'mcp.write_handoff'",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if count == 0 {
        Some("Compaction imminent. Call write_handoff now to preserve continuity.".into())
    } else {
        None
    }
}
