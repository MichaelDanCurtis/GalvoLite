use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::plan_executor::{execute_plan, ExecuteError, PhasePlan};
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

/// POST /api/projects/:id/phases/:phase_id/approve
///
/// Operator-driven advancement for phases whose `autonomy` is `review` or
/// `manual`. The reactor deliberately skips those phases; this endpoint lets
/// the operator unblock them.
///
/// Error responses:
/// - 403  GALVOLITE_PLAN_EXECUTOR=0 (feature flag off)
/// - 409  no orchestrator found for project
/// - 409  `phase_id` does not match the head of the pending-phases queue
/// - 200  phase advanced; body `{"approved": "<phase_id>"}`
pub async fn approve_phase(
    State(mgr): State<Arc<SessionManager>>,
    Path((project_id, phase_id)): Path<(Uuid, String)>,
) -> Response {
    if !crate::overseer::config::executor_enabled() {
        return (StatusCode::FORBIDDEN, "GALVOLITE_PLAN_EXECUTOR=0").into_response();
    }
    let orch = match mgr.find_orchestrator_for_project(project_id) {
        Some(o) => o,
        None => return (StatusCode::CONFLICT, "no orchestrator").into_response(),
    };
    // Verify that phase_id matches the head of the pending_phases queue.
    let raw: Option<String> = {
        let conn = match mgr.db().pool().get() {
            Ok(c) => c,
            Err(e) => return crate::api::err_to_response(crate::error::Error::DbPool(e)),
        };
        conn.query_row(
            "SELECT pending_phases_json FROM sessions WHERE id = ?1",
            [orch.id.to_string()],
            |r| r.get(0),
        )
        .ok()
        .flatten()
    };
    let queue: Vec<PhasePlan> = raw
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    if queue.first().map(|p| p.id.as_str()) != Some(phase_id.as_str()) {
        return (StatusCode::CONFLICT, "phase is not next in queue").into_response();
    }
    match crate::plan_executor::advance_next_phase(&mgr, orch.id, project_id).await {
        Ok(_) => Json(serde_json::json!({"approved": phase_id})).into_response(),
        Err(e) => crate::api::err_to_response(e),
    }
}
