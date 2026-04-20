//! Axum HTTP API.
//!
//! Builds a Router that the CLI `serve` command and the Tauri binary both use.
//! All business logic is delegated to SessionManager.

pub mod events;
pub mod hook_handlers;
pub mod hooks;
pub mod operator;

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
        .route("/api/sessions/:id/events", get(get_events))
        .route("/api/sessions/:id/plan_steps", get(get_plan_steps))
        .route("/api/sessions/:id/terminal", get(terminal_ws))
        .route("/api/fs/list", get(list_directory))
        .route("/hooks/ingest", post(hooks::ingest))
        .route("/operator/prompt/:id/answer", post(operator::answer))
        .route("/mcp", post(crate::mcp::transport::handle))
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

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<i64>,
}

/// Default page size for the Events feed when `?limit=` is omitted.
const EVENTS_DEFAULT_LIMIT: i64 = 100;
/// Hard cap; any larger value is clamped silently.
const EVENTS_MAX_LIMIT: i64 = 500;

/// `GET /api/sessions/:id/events?limit=N` — reverse-chronological events feed
/// for the dashboard's Events tab. Rows are shaped identically to the MCP
/// `read_events` response (id, ts, source, kind, payload, prev_state,
/// next_state) so `EventRow` is reusable. See [`crate::db::Db::read_recent_events`].
async fn get_events(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    axum::extract::Query(q): axum::extract::Query<EventsQuery>,
) -> Response {
    let raw = q.limit.unwrap_or(EVENTS_DEFAULT_LIMIT);
    // Clamp: reject nonsense (<=0) → default; cap enormous values.
    let limit = if raw <= 0 {
        EVENTS_DEFAULT_LIMIT
    } else {
        raw.min(EVENTS_MAX_LIMIT)
    };
    match mgr.db().read_recent_events(id, limit) {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => err_to_response(e),
    }
}

/// `GET /api/sessions/:id/plan_steps` — structured plan steps for the
/// dashboard's Plan tab, ordered by ordinal ASC. Row shape: `{ hash, text,
/// status, note }` matching the UI `PlanStep` type. Status `in_progress` is
/// rewritten to `working` by the DB helper so the wire shape is UI-native.
///
/// No pagination: plan_steps per session is small by design (dozens at most).
async fn get_plan_steps(
    State(mgr): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    match mgr.db().read_plan_steps(id) {
        Ok(rows) => Json(rows).into_response(),
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

    let (sink, mut stream) = socket.split();
    // The sink is shared between the PTY-bytes writer task and the
    // domain-event forwarder task: both can emit frames asynchronously,
    // so the writer half lives behind an Arc<Mutex<_>>. Each send takes
    // the lock briefly; contention is negligible because both producers
    // are bursty.
    //
    // TODO(plan-4 follow-up): Sink mutex is held across send().await in both
    // the writer and event_forwarder branches. Under client backpressure this
    // serializes both producers and risks broadcast lag. Consider moving to a
    // single writer task draining an mpsc<Message> funnel.
    let sink = std::sync::Arc::new(tokio::sync::Mutex::new(sink));

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

    let sink_for_writer = sink.clone();
    let writer = async move {
        loop {
            tokio::select! {
                biased;
                snap = snap_rx.recv() => {
                    let Some(bytes) = snap else { break };
                    let mut s = sink_for_writer.lock().await;
                    if s.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(bytes) => {
                            let mut s = sink_for_writer.lock().await;
                            if s.send(Message::Binary(bytes)).await.is_err() {
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

    // Domain-event forwarder: subscribe to the per-process event bus,
    // filter to events scoped to this session, serialize each as JSON,
    // and emit on the shared sink as a `Message::Text` frame. PTY bytes
    // (binary) and domain events (text) thus share one socket.
    let mut event_rx = mgr.subscribe_events();
    let sink_for_events = sink.clone();
    let event_forwarder = async move {
        loop {
            match event_rx.recv().await {
                Ok(evt) => {
                    if !evt.applies_to(session_id) {
                        continue;
                    }
                    let json = match serde_json::to_string(&evt) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, "domain event serialization failed; dropping frame");
                            continue;
                        }
                    };
                    let mut s = sink_for_events.lock().await;
                    if s.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    // When any of the three tasks ends (writer drained, reader observed
    // close, or event channel dropped), the others are dropped as well —
    // `tokio::select!` cancels the losing futures, releasing the sink
    // and the broadcast receivers. No background task leaks per WS.
    tokio::select! {
        _ = writer => {},
        _ = reader => {},
        _ = event_forwarder => {},
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
            tracing::warn!("fs_list canonicalize {:?}: {}", p, e);
            return (StatusCode::BAD_REQUEST, "path not accessible").into_response();
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
            tracing::warn!("fs_list read_dir {:?}: {}", canonical, e);
            return (StatusCode::BAD_REQUEST, "path not accessible").into_response();
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
