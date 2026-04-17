//! Axum HTTP API.
//!
//! Builds a Router that the CLI `serve` command and the Tauri binary both use.
//! All business logic is delegated to SessionManager.

use crate::session_manager::SessionManager;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

type AppState = Arc<SessionManager>;

pub fn router(mgr: Arc<SessionManager>) -> Router {
    Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/{id}", get(get_project))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", get(get_session).delete(kill_session))
        .route("/api/sessions/{id}/handoff", post(trigger_handoff))
        .route("/api/sessions/{id}/send", post(send_to_session))
        .route("/api/sessions/{id}/signals", get(get_signals))
        .route("/api/sessions/{id}/terminal", get(terminal_ws))
        .layer(CorsLayer::permissive())
        .with_state(mgr)
}

/// Map a [`crate::error::Error`] into an HTTP response with a status appropriate
/// to the error variant. Per the Plan 1b Task 7 memo:
///
/// - `SessionNotFound` → 404 (session id does not exist)
/// - `SessionNotRunning` → 409 (session exists in DB but is not live)
/// - All other variants (including `Db` / `Io` / `Config`) → 500
///
/// Note: `src/error.rs` does not have a dedicated `ProjectNotFound` variant.
/// Missing projects surface as `Error::Db(rusqlite::Error::QueryReturnedNoRows)`
/// from `Db::get_project`, which we currently map to 500. A future refactor may
/// introduce a `NotFound` variant; the central mapping here is the one place
/// that needs updating when that happens.
fn err_to_response(e: crate::error::Error) -> Response {
    use crate::error::Error;
    match &e {
        Error::SessionNotFound(_) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
        Error::SessionNotRunning(_) => (StatusCode::CONFLICT, e.to_string()).into_response(),
        _ => {
            // Opaque body for the 500 catchall — `Error::Io { path, .. }` and
            // `Error::Db(rusqlite::Error)` can leak filesystem paths and SQL
            // fragments. Log the full error for operators and return a fixed
            // string to the client.
            tracing::error!(error = ?e, "api internal error");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
        }
    }
}

// --- Project endpoints ---

async fn list_projects(State(mgr): State<AppState>) -> Response {
    match mgr.list_projects() {
        Ok(projects) => Json(projects).into_response(),
        Err(e) => err_to_response(e),
    }
}

#[derive(Deserialize)]
struct CreateProjectReq {
    name: String,
    path: String,
    remote_url: Option<String>,
}

async fn create_project(
    State(mgr): State<AppState>,
    Json(req): Json<CreateProjectReq>,
) -> Response {
    match mgr.add_project(crate::db::NewProject {
        name: req.name,
        path: req.path,
        remote_url: req.remote_url,
    }) {
        Ok(project) => (StatusCode::CREATED, Json(project)).into_response(),
        Err(e) => err_to_response(e),
    }
}

async fn get_project(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.get_project(id) {
        Ok(project) => Json(project).into_response(),
        Err(e) => err_to_response(e),
    }
}

// --- Session endpoints ---

async fn list_sessions(State(mgr): State<AppState>) -> Response {
    match mgr.list_sessions() {
        Ok(sessions) => Json(sessions).into_response(),
        Err(e) => err_to_response(e),
    }
}

#[derive(Deserialize)]
struct CreateSessionReq {
    project_id: uuid::Uuid,
    task: String,
    #[serde(default = "default_role")]
    role: String,
    #[serde(default = "default_claude_cmd")]
    claude_cmd: String,
    #[serde(default)]
    args: Vec<String>,
}

fn default_role() -> String {
    "implementation".into()
}
fn default_claude_cmd() -> String {
    "claude".into()
}

async fn create_session(
    State(mgr): State<AppState>,
    Json(req): Json<CreateSessionReq>,
) -> Response {
    match mgr
        .spawn_session(req.project_id, req.task, req.role, req.claude_cmd, req.args)
        .await
    {
        Ok(session) => (StatusCode::CREATED, Json(session)).into_response(),
        Err(e) => err_to_response(e),
    }
}

async fn get_session(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.get_session(id) {
        Ok(session) => Json(session).into_response(),
        Err(e) => err_to_response(e),
    }
}

async fn kill_session(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.kill_session(id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"status": "killed"}))).into_response(),
        Err(e) => err_to_response(e),
    }
}

async fn trigger_handoff(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.trigger_handoff(id).await {
        Ok(()) => Json(serde_json::json!({"status": "handoff initiated"})).into_response(),
        Err(e) => err_to_response(e),
    }
}

#[derive(Deserialize)]
struct SendReq {
    text: String,
    #[serde(default)]
    newline: bool,
}

async fn send_to_session(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<SendReq>,
) -> Response {
    match mgr.send_to_session(id, req.text, req.newline).await {
        Ok(()) => Json(serde_json::json!({"status": "sent"})).into_response(),
        Err(e) => err_to_response(e),
    }
}

async fn get_signals(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.get_signals(id) {
        Ok(signals) => Json(signals).into_response(),
        Err(e) => err_to_response(e),
    }
}

// --- WebSocket ---

async fn terminal_ws(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    ws: axum::extract::WebSocketUpgrade,
) -> Response {
    let rx = match mgr.subscribe_output(id) {
        Ok(rx) => rx,
        Err(e) => return err_to_response(e),
    };
    ws.on_upgrade(move |socket| handle_terminal_ws(socket, rx))
        .into_response()
}

async fn handle_terminal_ws(
    mut socket: axum::extract::ws::WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<String>,
) {
    use axum::extract::ws::Message;
    loop {
        match rx.recv().await {
            Ok(line) => {
                if socket.send(Message::Text(line)).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                let msg = format!("[galvolite: skipped {n} lines]");
                if socket.send(Message::Text(msg)).await.is_err() {
                    break;
                }
                // fall through: next recv() returns post-lag messages; must not break
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
}
