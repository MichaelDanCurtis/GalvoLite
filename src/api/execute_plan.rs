use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::plan_executor::{execute_plan, ExecuteError};
use crate::session_manager::SessionManager;

pub async fn handler(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
) -> Response {
    if !crate::overseer::config::executor_enabled() {
        return (StatusCode::FORBIDDEN, "GALVOLITE_PLAN_EXECUTOR=0").into_response();
    }
    match execute_plan(mgr, project_id).await {
        Ok(ids) => Json(serde_json::json!({"spawned": ids})).into_response(),
        Err(ExecuteError::MissingArtifact(name)) => {
            (StatusCode::BAD_REQUEST, format!("missing artifact: {name}")).into_response()
        }
        Err(ExecuteError::Derive(e)) => {
            (StatusCode::BAD_REQUEST, e.to_string()).into_response()
        }
        Err(ExecuteError::NoOrchestrator(_)) => {
            (StatusCode::CONFLICT, "no orchestrator session — spawn one first").into_response()
        }
        Err(ExecuteError::AlreadyRunning) => {
            (StatusCode::CONFLICT, "orchestrator already has running children").into_response()
        }
        Err(ExecuteError::Spawn(e)) => crate::api::err_to_response(e),
    }
}
