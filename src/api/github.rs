//! Plan 10a — GitHub HTTP endpoints.

#[allow(unused_imports)]
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[allow(unused_imports)]
use uuid::Uuid;

use crate::session_manager::SessionManager;

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub connected: bool,
    pub user: Option<crate::github::User>,
    pub reason: Option<String>,
}

/// `GET /api/github/status` — returns whether a token was present and
/// validated at startup. Never touches GitHub on the request path.
pub async fn status(State(mgr): State<Arc<SessionManager>>) -> Response {
    let token_present = std::env::var("GITHUB_TOKEN")
        .ok()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let user = mgr.github_user();
    let body = match (token_present, user) {
        (false, _) => StatusResponse {
            connected: false,
            user: None,
            reason: Some("no token".into()),
        },
        (true, Some(u)) => StatusResponse {
            connected: true,
            user: Some(u),
            reason: None,
        },
        (true, None) => StatusResponse {
            connected: false,
            user: None,
            reason: Some("token invalid".into()),
        },
    };
    Json(body).into_response()
}
