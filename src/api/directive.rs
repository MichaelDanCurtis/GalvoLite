//! GET/PUT `/api/projects/:id/directive` — read and write the orchestrator directive.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;
use uuid::Uuid;

use crate::directive;
use crate::session_manager::SessionManager;

type AppState = Arc<SessionManager>;

pub async fn get_directive(
    State(mgr): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Response {
    let project = match mgr.get_project(project_id) {
        Ok(p) => p,
        Err(e) => return crate::api::err_to_response(e),
    };
    let path = directive::directive_path(std::path::Path::new(&project.path));
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (StatusCode::NOT_FOUND, "no directive").into_response();
        }
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response(),
    };
    match directive::parse(&content) {
        Ok(d) => Json(d).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct PutBody {
    pub markdown: String,
}

pub async fn put_directive(
    State(mgr): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(body): Json<PutBody>,
) -> Response {
    let project = match mgr.get_project(project_id) {
        Ok(p) => p,
        Err(e) => return crate::api::err_to_response(e),
    };
    if let Err(e) = directive::parse(&body.markdown) {
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    let path = directive::directive_path(std::path::Path::new(&project.path));
    if let Err(e) = directive::write_atomic(&path, &body.markdown) {
        return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
    }
    mgr.event_bus().publish(
        crate::api::events::DomainEvent::DirectiveChanged { project_id },
    );
    StatusCode::OK.into_response()
}
