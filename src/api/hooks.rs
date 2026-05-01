//! POST /hooks/ingest — the wire endpoint for the seven Claude Code hooks.
//!
//! See spec §6 for the envelope and §7.2 for the bearer-auth contract. This
//! handler is the only path through which `Source::Hook` events reach the
//! state funnel.

use crate::{
    error::Error,
    hooks::types::*,
    state::{self, EventInput, Source},
};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use std::sync::Arc;

/// Per Sub-plan 2 architecture: app state is `Arc<SessionManager>` for the
/// rest of the API, but this handler only needs the `Db` handle. We extract
/// it from the manager via `State<Arc<crate::session_manager::SessionManager>>`
/// in the wiring layer (see `super::router`).
pub async fn ingest(
    State(mgr): State<Arc<crate::session_manager::SessionManager>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<HookIngestResponse>, (StatusCode, String)> {
    let db = mgr.db();

    let token = headers
        .get("X-Galvolite-Token")
        .and_then(|h| h.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing token".into()))?;

    // Resolve token → session_id with constant-time compare.
    let session_id = crate::api::auth::resolve_token(db, token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "bad token".into()))?;

    // Parse body. Manual parse so malformed JSON returns 400 (axum's Json
    // extractor would 422 with a body that leaks structure details).
    let req: HookIngestRequest = serde_json::from_slice(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid envelope: {e}")))?;

    // Verify envelope's session_id matches token-resolved session_id.
    let envelope_id = uuid::Uuid::parse_str(&req.session_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid session_id".into()))?;
    if envelope_id != session_id {
        return Err((StatusCode::UNAUTHORIZED, "session_id mismatch".into()));
    }

    // Compute event kind.
    let kind = req.hook.as_event_kind();

    let bus = mgr.event_bus();

    // Apply through the state funnel.
    let _outcome = state::apply_event_with_bus(
        db,
        session_id,
        bus,
        EventInput {
            source: Source::Hook,
            kind,
            payload: req.payload.clone(),
            dedup_key: Some(req.event_id.clone()),
        },
    )
    .map_err(|e| match e {
        Error::TerminalState { .. } => (StatusCode::CONFLICT, e.to_string()),
        Error::EventPayload(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        Error::SessionNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()),
        other => {
            tracing::error!(error = ?other, "hooks/ingest internal error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        }
    })?;

    // Per-hook semantics (inject_text, action). Lifecycle handlers in
    // Task 7; turn/observe handlers in Tasks 8-9.
    use super::hook_handlers::{lifecycle, observe, turn};
    let inject = match req.hook {
        HookKind::SessionStart => lifecycle::on_session_start(db, session_id, &req.payload),
        HookKind::SessionEnd => {
            lifecycle::on_session_end(db, session_id, &req.payload, bus);
            None
        }
        HookKind::UserPromptSubmit => {
            turn::on_user_prompt_submit(db, session_id, &req.payload)
        }
        HookKind::Stop => turn::on_stop(db, session_id, bus),
        HookKind::PreCompact => turn::on_pre_compact(db, session_id),
        HookKind::PostToolUse => observe::on_post_tool_use(db, session_id, &req.payload, bus),
        HookKind::Notification => observe::on_notification(db, session_id, &req.payload),
    };

    Ok(Json(HookIngestResponse {
        action: HookAction::Proceed,
        inject_text: inject,
        note: None,
    }))
}

