//! Plan 8 Task 13 — Recommendations REST: list endpoint.
//!
//! `GET /api/projects/:id/recommendations[?status=pending|acted|dismissed|deferred]`
//!
//! Returns the rows from [`crate::recommendations::list_by_status`] as JSON.
//! The `status` query parameter defaults to `pending`. Unknown values yield
//! `400 Bad Request` (we deliberately do not silently coerce — the dashboard
//! should pass a known value or omit the param entirely).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::recommendations::{self, RecStatus};
use crate::session_manager::SessionManager;

#[derive(Deserialize)]
pub struct ListQ {
    pub status: Option<String>,
}

pub async fn list(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<ListQ>,
) -> Response {
    let status = match q.status.as_deref() {
        None | Some("pending") => RecStatus::Pending,
        Some("acted") => RecStatus::Acted,
        Some("dismissed") => RecStatus::Dismissed,
        Some("deferred") => RecStatus::Deferred,
        _ => return (StatusCode::BAD_REQUEST, "bad status").into_response(),
    };
    match recommendations::list_by_status(mgr.db(), project_id, status) {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => crate::api::err_to_response(e),
    }
}
