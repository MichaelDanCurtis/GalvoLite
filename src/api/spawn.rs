//! GET /api/sessions/:id/pending-rejoin — atomic read-and-clear of the
//! `pending_rejoin` field, with worktree GC for completed children.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::sync::Arc;

pub async fn get_pending_rejoin(
    State(mgr): State<Arc<crate::session_manager::SessionManager>>,
    Path(session_id): Path<uuid::Uuid>,
    headers: HeaderMap,
) -> Response {
    // --- Auth: require Bearer token ---
    let token = match headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(t) => t.to_owned(),
        None => {
            return (StatusCode::UNAUTHORIZED, "missing token").into_response();
        }
    };

    // --- Resolve token → session_id ---
    let resolved = match crate::api::auth::resolve_token(mgr.db(), &token) {
        Ok(id) => id,
        Err(_) => return (StatusCode::UNAUTHORIZED, "bad token").into_response(),
    };

    // --- Authorize: token must belong to the URL-path session ---
    if resolved != session_id {
        return (StatusCode::UNAUTHORIZED, "token session mismatch").into_response();
    }

    // --- Atomic read-and-clear via transaction (Correction C) ---
    let mut conn = match mgr.db().pool().get() {
        Ok(c) => c,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response(),
    };
    let tx = match conn.transaction() {
        Ok(t) => t,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response(),
    };

    let existing: Option<String> = tx
        .query_row(
            "SELECT pending_rejoin FROM sessions WHERE id = ?1",
            [session_id.to_string()],
            |r| r.get(0),
        )
        .ok()
        .flatten();

    let Some(markdown) = existing else {
        // Implicit rollback on drop — fine, no writes occurred.
        return (StatusCode::NOT_FOUND, "no pending rejoin").into_response();
    };

    tx.execute(
        "UPDATE sessions SET pending_rejoin = NULL WHERE id = ?1",
        [session_id.to_string()],
    )
    .ok();

    // Gather worktree paths for completed children inside the same tx so the
    // snapshot is consistent with the moment of clear. Collect fully before
    // closing the stmt so there is no borrow when we call tx.commit().
    let paths: Vec<String> = {
        let mut paths = Vec::new();
        if let Ok(mut stmt) = tx.prepare(
            "SELECT worktree_path FROM sessions \
             WHERE parent_session_id = ?1 AND terminal_status = 'complete'",
        ) {
            if let Ok(rows) =
                stmt.query_map([session_id.to_string()], |r| r.get::<_, String>(0))
            {
                paths = rows.filter_map(|r| r.ok()).collect();
            }
        }
        paths
    };

    if let Err(e) = tx.commit() {
        tracing::warn!(error = %e, "pending-rejoin commit failed");
        // Even on commit failure, return 200 with markdown — the read happened.
    }

    // Worktree GC: remove worktrees for completed children; ignore errors.
    for p in paths {
        let _ = crate::worktree::remove_worktree(std::path::Path::new(&p));
    }

    Json(json!({"markdown": markdown})).into_response()
}
