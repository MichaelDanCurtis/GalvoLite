//! GET `/api/projects/:id/plan` — returns the project's plan.md if present.
//!
//! Body: `{ "markdown": "<contents>" }` on 200, or 404 if no plan.md exists.
//! Plan.md is project-scoped (lives at `<project.path>/.galvolite/plan.md`)
//! and only humans edit it.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::session_manager::SessionManager;

type AppState = Arc<SessionManager>;

#[derive(Serialize)]
pub struct PlanResponse {
    pub markdown: String,
}

pub async fn get_plan(
    State(mgr): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Response {
    let project = match mgr.get_project(project_id) {
        Ok(p) => p,
        Err(e) => return crate::api::err_to_response(e),
    };
    let plan_path = std::path::Path::new(&project.path)
        .join(".galvolite")
        .join("plan.md");
    match std::fs::read_to_string(&plan_path) {
        Ok(markdown) => Json(PlanResponse { markdown }).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "no plan.md").into_response()
        }
        Err(e) => {
            tracing::error!(
                project_id = %project_id,
                error = %e,
                "read plan.md failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
        }
    }
}
