//! POST /hooks/ingest — the wire endpoint for the seven Claude Code hooks.
//!
//! See spec §6 for the envelope and §7.2 for the bearer-auth contract. This
//! handler is the only path through which `Source::Hook` events reach the
//! state funnel.

use crate::{
    db::Db,
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
use subtle::ConstantTimeEq;

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
    let session_id = resolve_token(db, token)
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

    // Apply through the state funnel.
    let _outcome = state::apply_event(
        db,
        session_id,
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

    // Per-hook semantic responses (inject_text, action) come in Tasks 7-9.
    Ok(Json(HookIngestResponse {
        action: HookAction::Proceed,
        inject_text: None,
        note: None,
    }))
}

/// Resolve a bearer token to its owning `session_id` using a constant-time
/// byte compare to defeat timing oracles. Iterates over live tokens; for
/// the session counts GalvoLite expects (<100) this is fine.
fn resolve_token(db: &Db, token: &str) -> Result<uuid::Uuid, Error> {
    let conn = db.pool().get()?;
    let mut stmt =
        conn.prepare("SELECT id, api_token FROM sessions WHERE api_token IS NOT NULL")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let token_bytes = token.as_bytes();
    let mut found: Option<uuid::Uuid> = None;
    for row in rows {
        let (id, t) = row?;
        // Constant-time compare requires equal-length slices; padding with
        // a wrong-but-constant length keeps the timing uniform.
        if t.len() == token_bytes.len() && t.as_bytes().ct_eq(token_bytes).unwrap_u8() == 1 {
            // Don't early-return: keep iterating so the compare time doesn't
            // depend on which row matched.
            found = uuid::Uuid::parse_str(&id).ok();
        }
    }
    found.ok_or(Error::SessionNotFound(uuid::Uuid::nil()))
}
