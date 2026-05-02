//! GET `/api/projects/:id/postmortems?part=N` — list postmortems for a part.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::postmortem;
use crate::session_manager::SessionManager;

type AppState = Arc<SessionManager>;

#[derive(Deserialize)]
pub struct Q {
    pub part: u32,
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
    match postmortem::list_for_part(std::path::Path::new(&project.path), q.part) {
        Ok(list) => Json(list).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
