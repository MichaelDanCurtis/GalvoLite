//! GET `/api/projects/:id/decisions?limit=N` — list recent decisions.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::decisions;
use crate::session_manager::SessionManager;

type AppState = Arc<SessionManager>;

#[derive(Deserialize)]
pub struct Q {
    pub limit: Option<usize>,
}

pub async fn list(
    State(mgr): State<AppState>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<Q>,
) -> Response {
    let project = match mgr.get_project(project_id) {
        Ok(p) => p,
        Err(e) => return crate::api::err_to_response(e),
    };
    let limit = q.limit.unwrap_or(50);
    match decisions::list_recent(std::path::Path::new(&project.path), limit) {
        Ok(list) => Json(list).into_response(),
        Err(e) => {
            tracing::error!(
                project_id = %project_id,
                limit,
                error = %e,
                "list decisions failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
        }
    }
}
