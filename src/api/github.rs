//! Plan 10a — GitHub HTTP endpoints.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

#[derive(Debug, Deserialize)]
pub struct LinkRepoReq {
    pub repo: String, // "owner/name"
}

#[derive(Debug, Serialize)]
pub struct LinkRepoResp {
    pub repo: String,
}

/// Parse "owner/name" into its parts. Returns None if format is invalid.
fn parse_repo(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    let mut parts = s.splitn(2, '/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if owner.is_empty()
        || name.is_empty()
        || owner.contains(' ')
        || name.contains(' ')
        || name.contains('/')
    {
        return None;
    }
    Some((owner, name))
}

/// `PUT /api/projects/:id/github/repo` — body `{"repo": "owner/name"}`.
///
/// Format-validates locally, then hits `GET /repos/{owner}/{name}` via
/// our cached `Client` to confirm the repo exists and the PAT can see
/// it, then writes `projects.github_repo`. Returns 200 on success, 422
/// on bad format or unknown repo, 503 if no GitHub client is available
/// (no token at startup).
pub async fn link_repo(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<LinkRepoReq>,
) -> Response {
    let Some((owner, name)) = parse_repo(&req.repo) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "expected format 'owner/name'"})),
        )
            .into_response();
    };

    // Construct a fresh Client for this request from env. Cheap; the
    // octocrab builder doesn't perform any I/O. Returning early when the
    // env var is missing surfaces a clearer error than a 502 from the
    // validate call.
    let Some(client) = crate::github::Client::from_env() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };

    if let Err(e) = client.validate_repo(owner, name).await {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": format!("{e}")})),
        )
            .into_response();
    }

    match mgr.db().set_project_github_repo(project_id, Some(&req.repo)) {
        Ok(()) => Json(LinkRepoResp { repo: req.repo }).into_response(),
        Err(e) => crate::api::err_to_response(e),
    }
}
