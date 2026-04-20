//! `PostToolUse(Bash)`, `Notification` handlers.
//!
//! - `PostToolUse(Bash)`: pattern-match `command` against the operationally
//!   interesting prefixes (`git push|git commit|cargo test|npm test`). The
//!   hook event row is already written by `/hooks/ingest` via the funnel;
//!   `git push` additionally raises `system.attention_needed` so the funnel
//!   transitions the session to `NeedsAttention`. Other matched commands
//!   are logged-only (no extra event, no state change).
//! - `Notification`: no-op stub. The event row was written by `/hooks/ingest`
//!   via the funnel; Sub-plan 4 will wire WS fan-out from this kind to
//!   dashboard toasts.

use crate::db::Db;
use uuid::Uuid;

pub fn on_post_tool_use(
    db: &Db,
    session_id: Uuid,
    payload: &serde_json::Value,
    bus: &crate::api::events::EventBroadcaster,
) -> Option<String> {
    let cmd = payload
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if cmd.starts_with("git push") {
        let _ = crate::state::apply_event_with_bus(
            db,
            session_id,
            bus,
            crate::state::EventInput {
                source: crate::state::Source::System,
                kind: "system.attention_needed".into(),
                payload: serde_json::json!({
                    "reason": "git_push_observed",
                    "command": cmd,
                }),
                dedup_key: None,
            },
        );
    }
    None
}

pub fn on_notification(_db: &Db, _session_id: Uuid, _payload: &serde_json::Value) -> Option<String> {
    // Event row already written by /hooks/ingest's apply_event call.
    // Sub-plan 4 will wire WS fan-out from this kind to dashboard toasts.
    None
}
