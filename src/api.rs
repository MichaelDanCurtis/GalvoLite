//! Axum HTTP API.
//!
//! Builds a Router that the CLI `serve` command and the Tauri binary both use.
//! All business logic is delegated to SessionManager.

use crate::session_manager::SessionManager;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

type AppState = Arc<SessionManager>;

pub fn router(mgr: Arc<SessionManager>) -> Router {
    let api = Router::new()
        .route("/api/projects", get(list_projects).post(create_project))
        .route("/api/projects/:id", get(get_project).delete(delete_project))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/:id", get(get_session).delete(kill_session))
        .route("/api/sessions/:id/handoff", post(trigger_handoff))
        .route("/api/sessions/:id/send", post(send_to_session))
        .route("/api/sessions/:id/autopilot", put(set_autopilot))
        .route("/api/sessions/:id/signals", get(get_signals))
        .route("/api/sessions/:id/terminal", get(terminal_ws))
        .route("/api/fs/list", get(list_directory))
        .layer(CorsLayer::permissive())
        .with_state(mgr);

    let ui_dir = std::path::PathBuf::from(
        std::env::var("GALVOLITE_UI_DIR").unwrap_or_else(|_| "ui/dist".to_string()),
    );
    if ui_dir.exists() {
        api.fallback_service(
            tower_http::services::ServeDir::new(&ui_dir)
                .fallback(tower_http::services::ServeFile::new(ui_dir.join("index.html"))),
        )
    } else {
        api
    }
}

/// Map a [`crate::error::Error`] into an HTTP response with a status appropriate
/// to the error variant. Per the Plan 1b Task 7 memo:
///
/// - `SessionNotFound` → 404 (session id does not exist)
/// - `ProjectNotFound` → 404 (project id does not exist; added in Plan 2)
/// - `SessionNotRunning` → 409 (session exists in DB but is not live)
/// - `InvalidPath` → 400 (client-supplied path failed validation; the reason
///   string is safe to echo back since it's generated from our validator,
///   not from filesystem or SQL internals)
/// - All other variants (including `Db` / `Io` / `Config`) → 500
fn err_to_response(e: crate::error::Error) -> Response {
    use crate::error::Error;
    match &e {
        Error::SessionNotFound(_) | Error::ProjectNotFound(_) => {
            (StatusCode::NOT_FOUND, e.to_string()).into_response()
        }
        Error::SessionNotRunning(_) => (StatusCode::CONFLICT, e.to_string()).into_response(),
        Error::InvalidPath(_) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
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

async fn delete_project(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.delete_project(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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

#[derive(Deserialize)]
struct AutopilotReq {
    enabled: bool,
}

async fn set_autopilot(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<AutopilotReq>,
) -> Response {
    match mgr.set_autopilot(id, req.enabled).await {
        Ok(()) => Json(serde_json::json!({"autopilot": req.enabled})).into_response(),
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
    let rx = match mgr.subscribe_output_only(id) {
        Ok(rx) => rx,
        Err(e) => return err_to_response(e),
    };
    ws.on_upgrade(move |socket| handle_terminal_ws(socket, rx, mgr, id))
        .into_response()
}

/// Client → server control frame format (JSON over `Message::Text`).
///
/// Currently only `resize` is recognized; unknown `type` values are ignored
/// so the protocol can grow without breaking older clients.
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientFrame {
    Resize { cols: u16, rows: u16 },
    Input { data: String },
}

async fn handle_terminal_ws(
    socket: axum::extract::ws::WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<Vec<u8>>,
    mgr: AppState,
    session_id: uuid::Uuid,
) {
    use axum::extract::ws::Message;
    use futures_util::{sink::SinkExt, stream::StreamExt};

    let (mut sink, mut stream) = socket.split();

    // Side channel: reader → writer, used to deliver fresh server-side
    // screen snapshots taken AFTER each client-driven resize. We do NOT
    // send a snapshot at WS open: the parser is sized to whatever the
    // PREVIOUS client (or initial PTY spawn) reported, while the new
    // xterm starts at default dims and only fires its `fit()`-driven
    // resize after the socket is up. A snapshot taken before that resize
    // contains absolute cursor positions that land on the wrong cells in
    // the client's grid, so an alt-screen TUI like Claude Code renders as
    // interleaved garbage. Defer the snapshot until the parser has been
    // resized to match the client; re-snapshot on every subsequent resize
    // so the client always lands on a coherent repaint.
    let (snap_tx, mut snap_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);

    let writer = async move {
        loop {
            tokio::select! {
                biased;
                snap = snap_rx.recv() => {
                    let Some(bytes) = snap else { break };
                    if sink.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(bytes) => {
                            if sink.send(Message::Binary(bytes)).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    };

    let mgr_for_reader = mgr.clone();
    let reader = async move {
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(text) => {
                    if let Ok(frame) = serde_json::from_str::<ClientFrame>(&text) {
                        match frame {
                            ClientFrame::Resize { cols, rows } => {
                                if mgr_for_reader.resize_pty(session_id, cols, rows).is_ok() {
                                    if let Ok(snap) = mgr_for_reader.snapshot_terminal(session_id) {
                                        if !snap.is_empty() && snap_tx.send(snap).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                            }
                            ClientFrame::Input { data } => {
                                let _ = mgr_for_reader.write_pty(session_id, data.into_bytes()).await;
                            }
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };

    tokio::select! {
        _ = writer => {},
        _ = reader => {},
    }
}

// --- Filesystem browser (server-driven directory picker) ---

#[derive(serde::Deserialize)]
struct FsListQuery {
    path: String,
}

#[derive(serde::Serialize)]
struct FsEntry {
    name: String,
    is_dir: bool,
}

#[derive(serde::Serialize)]
struct FsListing {
    path: String,
    parent: Option<String>,
    entries: Vec<FsEntry>,
}

async fn list_directory(
    axum::extract::Query(q): axum::extract::Query<FsListQuery>,
) -> Response {
    let p = std::path::PathBuf::from(&q.path);
    if !p.is_absolute() {
        return (StatusCode::BAD_REQUEST, "path must be absolute").into_response();
    }
    let canonical = match p.canonicalize() {
        Ok(c) => c,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("path not accessible: {e}"))
                .into_response()
        }
    };
    if !canonical.is_dir() {
        return (StatusCode::BAD_REQUEST, "path is not a directory").into_response();
    }

    let mut entries: Vec<FsEntry> = match std::fs::read_dir(&canonical) {
        Ok(rd) => rd
            .filter_map(|r| r.ok())
            .map(|de| {
                let name = de.file_name().to_string_lossy().into_owned();
                let is_dir = de
                    .file_type()
                    .map(|t| t.is_dir())
                    .unwrap_or(false);
                FsEntry { name, is_dir }
            })
            .collect(),
        Err(e) => {
            return (StatusCode::BAD_REQUEST, format!("read_dir failed: {e}"))
                .into_response()
        }
    };
    // Dirs first, then files; case-insensitive name within each group.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let parent = canonical
        .parent()
        .and_then(|p| p.to_str())
        .map(|s| s.to_owned());

    Json(FsListing {
        path: canonical.to_string_lossy().into_owned(),
        parent,
        entries,
    })
    .into_response()
}
