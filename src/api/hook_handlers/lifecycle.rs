//! `SessionStart` and `SessionEnd` hook handlers.
//!
//! - `SessionStart`: build seed text combining role profile, optional last-
//!   handoff summary (when `is_resume`), and an MCP coordination pointer.
//! - `SessionEnd`: clean exit (exit_code 0) → Ended, unclean → Error. If no
//!   `mcp.write_handoff` event in the last 50 entries, auto-generate a
//!   minimal handoff pointer from the event log.

use crate::db::Db;
use uuid::Uuid;

pub fn on_session_start(
    db: &Db,
    session_id: Uuid,
    payload: &serde_json::Value,
) -> Option<String> {
    let role = db.get_session(session_id).ok().map(|s| s.role)?;
    let mut seed = String::new();

    seed.push_str(&format!("# Role: {role}\n\n"));
    seed.push_str(&render_role_profile(&role));
    seed.push_str("\n\n");

    if payload
        .get("is_resume")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        if let Some(handoff) = last_handoff_summary(db, session_id) {
            seed.push_str("## Last handoff\n\n");
            seed.push_str(&handoff);
            seed.push_str("\n\n");
        }
    }

    seed.push_str("## MCP coordination\n\n");
    seed.push_str("Active coordination via MCP tools at `galvolite://`. ");
    seed.push_str("Use status_report, set_plan, write_handoff to communicate state.\n");

    Some(seed)
}

fn render_role_profile(role: &str) -> String {
    // Minimal placeholder profile — Sub-plan 4+ may pull from
    // .galvolite/profiles/<role>.md.
    match role {
        "implementation" => {
            "You are an implementation worker. Focus on writing tested code.".into()
        }
        "brainstorm" => {
            "You are a brainstorming partner. Help shape ideas before code.".into()
        }
        "orchestrator" => {
            "You are an orchestrator. Coordinate other sessions; do not write code yourself."
                .into()
        }
        "scout" => "You are a scout. Explore unfamiliar code and report findings.".into(),
        other => format!("You are a {other} worker."),
    }
}

fn last_handoff_summary(db: &Db, session_id: Uuid) -> Option<String> {
    let conn = db.pool().get().ok()?;
    let row: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM events
             WHERE session_id = ?1 AND kind = 'mcp.write_handoff'
             ORDER BY ts DESC LIMIT 1",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .ok();
    row.and_then(|p| serde_json::from_str::<serde_json::Value>(&p).ok())
        .and_then(|v| {
            v.get("summary")
                .and_then(|s| s.as_str())
                .map(String::from)
        })
}

pub fn on_session_end(db: &Db, session_id: Uuid, payload: &serde_json::Value) {
    let exit_code = payload
        .get("exit_code")
        .and_then(|v| v.as_i64())
        .unwrap_or(-1);

    // The apply_event call from /hooks/ingest already wrote a hook.SessionEnd
    // event but transition() needs to know clean vs unclean — emit a follow-
    // up system event with the resolved state so the funnel records both
    // the hook fact and the final disposition.
    let kind = if exit_code == 0 {
        "system.session_ended"
    } else {
        "system.session_errored"
    };
    let _ = crate::state::apply_event(
        db,
        session_id,
        crate::state::EventInput {
            source: crate::state::Source::System,
            kind: kind.into(),
            payload: serde_json::json!({"exit_code": exit_code}),
            dedup_key: Some(format!("session-end:{session_id}:{exit_code}")),
        },
    );

    // Auto-handoff if missing.
    if !has_recent_write_handoff(db, session_id, 50) {
        let _ = write_auto_handoff(db, session_id);
    }
}

fn has_recent_write_handoff(db: &Db, session_id: Uuid, window: usize) -> bool {
    let conn = match db.pool().get() {
        Ok(c) => c,
        Err(_) => return false,
    };
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM (
                SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT ?2
             ) WHERE kind = 'mcp.write_handoff'",
            rusqlite::params![session_id.to_string(), window as i64],
            |r| r.get(0),
        )
        .unwrap_or(0);
    count > 0
}

fn write_auto_handoff(db: &Db, session_id: Uuid) -> std::io::Result<()> {
    let dir = std::path::PathBuf::from(format!(".galvolite/handoffs/{session_id}"));
    std::fs::create_dir_all(&dir)?;
    let summary = build_summary_from_events(db, session_id);
    let pointer = serde_json::json!({
        "session_id": session_id.to_string(),
        "auto_generated": true,
        "summary": summary,
    });
    std::fs::write(
        dir.join("pointer.json"),
        serde_json::to_string_pretty(&pointer).unwrap(),
    )
}

fn build_summary_from_events(db: &Db, session_id: Uuid) -> String {
    // Minimal: list the last 10 event kinds. Sub-plan 5 may upgrade this.
    let conn = match db.pool().get() {
        Ok(c) => c,
        Err(_) => return "(no db)".into(),
    };
    let mut stmt = match conn.prepare(
        "SELECT kind FROM events WHERE session_id = ?1 ORDER BY ts DESC LIMIT 10",
    ) {
        Ok(s) => s,
        Err(_) => return "(prepare failed)".into(),
    };
    let kinds: Vec<String> = stmt
        .query_map([session_id.to_string()], |r| r.get(0))
        .map(|m| m.filter_map(Result::ok).collect())
        .unwrap_or_default();
    format!("Recent activity: {}", kinds.join(", "))
}
