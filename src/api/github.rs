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
    let (owner, name) = s.trim().split_once('/')?;
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
        tracing::warn!(project_id = %project_id, repo = %req.repo, "github link rejected: bad format");
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
        tracing::warn!(project_id = %project_id, "github link rejected: GITHUB_TOKEN not set");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };

    if let Err(e) = client.validate_repo(owner, name).await {
        tracing::warn!(project_id = %project_id, repo = %req.repo, error = %e, "github link rejected: validate_repo failed");
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": format!("{e}")})),
        )
            .into_response();
    }

    match mgr.db().set_project_github_repo(project_id, Some(&req.repo)) {
        Ok(()) => Json(LinkRepoResp { repo: req.repo }).into_response(),
        Err(e) => {
            tracing::warn!(project_id = %project_id, repo = %req.repo, error = %e, "github link rejected: db write failed");
            crate::api::err_to_response(e)
        }
    }
}

/// `GET /api/projects/:id/github/prs` — list open PRs for the project's
/// linked repo. Returns 404 if no repo is linked or project unknown,
/// 503 if no token, 502 on GitHub error.
pub async fn list_prs(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
) -> Response {
    let (owner, name) = match resolve_linked_repo(&mgr, project_id) {
        Ok(parts) => parts,
        Err(resp) => return resp,
    };

    let Some(client) = crate::github::Client::from_env() else {
        tracing::warn!(%project_id, "github list rejected: GITHUB_TOKEN not set");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };

    match client.list_open_prs(&owner, &name).await {
        Ok(prs) => Json(prs).into_response(),
        Err(e) => {
            tracing::warn!(%project_id, owner = %owner, name = %name, error = %e, "github list_prs failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("{e}")})),
            )
                .into_response()
        }
    }
}

/// `GET /api/projects/:id/github/issues` — same shape as list_prs but
/// for issues (PRs filtered out by the client).
pub async fn list_issues(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
) -> Response {
    let (owner, name) = match resolve_linked_repo(&mgr, project_id) {
        Ok(parts) => parts,
        Err(resp) => return resp,
    };

    let Some(client) = crate::github::Client::from_env() else {
        tracing::warn!(%project_id, "github list rejected: GITHUB_TOKEN not set");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };

    match client.list_open_issues(&owner, &name).await {
        Ok(issues) => Json(issues).into_response(),
        Err(e) => {
            tracing::warn!(%project_id, owner = %owner, name = %name, error = %e, "github list_issues failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("{e}")})),
            )
                .into_response()
        }
    }
}

/// Helper: fetch the project, parse its linked repo, return owner+name.
/// Folds all error cases (project not found, no linked repo, malformed
/// stored repo) into pre-built `Response` values so the call sites stay
/// flat.
///
/// `Db::get_project` surfaces a missing row as
/// `Error::Db(rusqlite::Error::QueryReturnedNoRows)` rather than
/// `Error::ProjectNotFound`, so we normalize here before delegating to
/// `err_to_response` — same pattern as `SessionManager::delete_project`.
#[allow(
    clippy::result_large_err,
    reason = "boundary-type Response is consumed at the handler immediately, no cloning"
)]
fn resolve_linked_repo(
    mgr: &Arc<SessionManager>,
    project_id: Uuid,
) -> Result<(String, String), Response> {
    let project = match mgr.db().get_project(project_id) {
        Ok(p) => p,
        Err(crate::error::Error::ProjectNotFound(_))
        | Err(crate::error::Error::Db(rusqlite::Error::QueryReturnedNoRows)) => {
            tracing::warn!(%project_id, "github request: project not found");
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "project not found"})),
            )
                .into_response());
        }
        Err(e) => {
            tracing::warn!(%project_id, error = %e, "github request: db lookup failed");
            return Err(crate::api::err_to_response(e));
        }
    };
    let stored = project.github_repo.ok_or_else(|| {
        tracing::warn!(%project_id, "github request: no linked repo");
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no linked repo"})),
        )
            .into_response()
    })?;
    match parse_repo(&stored) {
        Some((o, n)) => Ok((o.to_string(), n.to_string())),
        None => {
            tracing::warn!(%project_id, %stored, "github request: stored repo has bad format");
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "stored repo has bad format"})),
            )
                .into_response())
        }
    }
}
