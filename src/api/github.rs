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

// ---------------------------------------------------------------------------
// Plan 10b Task 5 — `GET /api/sessions/:id/pr-draft`.
//
// Computes a draft PR title/body for an operator to edit before submitting.
// The body is built from session metadata + git shell-outs against the
// session's worktree; orchestrator sessions also get a Plan section read
// from `<worktree>/.galvolite/plan.md`. We never touch GitHub except for
// `default_branch` (which falls back to "main" on error).
// ---------------------------------------------------------------------------

use std::path::Path as FsPath;

#[derive(Debug, Serialize)]
pub struct PrDraftResponse {
    pub title: String,
    pub body: String,
    pub base: String,
    pub commits_ahead: u32,
    pub commits_behind: u32,
}

/// Run `git -C <worktree>` with the given args, return stdout as String.
/// Returns empty string on any error (best-effort; PR draft computation
/// continues even if the commit log can't be produced).
fn git_capture(worktree: &FsPath, args: &[&str]) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(args)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).into_owned())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Compute (commits_ahead, commits_behind) for `<base>...HEAD` in the given
/// worktree. Returns (0, 0) on any git error. `git rev-list --left-right
/// --count A...B` prints `<left> <right>` — left is commits reachable from
/// A but not B (behind) and right is commits reachable from B but not A
/// (ahead).
fn commits_ahead_behind(worktree: &FsPath, base: &str) -> (u32, u32) {
    let out = git_capture(
        worktree,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base}...HEAD"),
        ],
    );
    let mut parts = out.split_whitespace();
    let behind = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ahead = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (ahead, behind)
}

/// `git log --oneline --no-decorate -n 50 <base>..HEAD` — capped so we never
/// return a runaway body.
fn commit_log(worktree: &FsPath, base: &str) -> Vec<String> {
    git_capture(
        worktree,
        &[
            "log",
            "--oneline",
            "--no-decorate",
            "-n",
            "50",
            &format!("{base}..HEAD"),
        ],
    )
    .lines()
    .map(|l| l.to_string())
    .collect()
}

/// Build the PR body from session + worktree state. Top-level
/// orchestrator sessions get a Plan section sourced from
/// `.galvolite/plan.md`; child sessions get just the task summary +
/// commit list.
///
/// Note on the orchestrator predicate: the `sessions.role` schema CHECK
/// (migrations/0001_initial.sql) only allows
/// `('implementation','brainstorm','scout','fix')` — `'orchestrator'`
/// is never persisted as a role, it's a runtime profile signaled via
/// `[ROLE:orchestrator]`. The codebase identifies orchestrator
/// sessions structurally as top-level (no parent, not ended); see
/// `SessionManager::find_orchestrator_for_project`. We follow the same
/// convention here so the Plan section actually fires for top-level
/// sessions that own a `.galvolite/plan.md`.
fn build_pr_body(session: &crate::db::Session, worktree: &FsPath, base: &str) -> String {
    let mut body = format!("## Task\n{}\n\n## Commits\n", session.task_summary);
    let log = commit_log(worktree, base);
    if log.is_empty() {
        body.push_str("(no commits ahead of base)\n");
    } else {
        for line in log {
            body.push_str(&format!("- {line}\n"));
        }
    }
    if session.parent_session_id.is_none() && session.ended_at.is_none() {
        let plan_path = worktree.join(".galvolite/plan.md");
        let plan = std::fs::read_to_string(&plan_path).unwrap_or_else(|_| "(no plan.md)".into());
        body.push_str("\n## Plan\n");
        body.push_str(&plan);
    }
    body
}

/// Truncate a string at a word boundary at or below `max` chars. Operates
/// on chars (not bytes) so multibyte input doesn't split a codepoint.
fn truncate_title(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    // Find last whitespace at or before max; if none, hard-cut at max-1.
    let cap_idx = chars
        .iter()
        .enumerate()
        .take(max)
        .filter(|(_, c)| c.is_whitespace())
        .map(|(i, _)| i)
        .next_back()
        .unwrap_or(max - 1);
    chars.iter().take(cap_idx).collect::<String>().trim().to_string()
}

/// `GET /api/sessions/:id/pr-draft` — compute a draft PR title/body for
/// an operator to edit before submitting. Returns 404 if the session is
/// missing, the project has no linked repo, or the session has no branch;
/// 503 if no GitHub token is configured. Never touches GitHub except for
/// the `default_branch` lookup (which falls back to "main" on error).
pub async fn pr_draft(
    State(mgr): State<Arc<SessionManager>>,
    Path(session_id): Path<Uuid>,
) -> Response {
    let (session, project) = match resolve_session_for_pr(&mgr, session_id) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let Some(repo_str) = project.github_repo.as_ref() else {
        tracing::warn!(%session_id, "pr_draft: project has no linked repo");
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no linked repo"})),
        )
            .into_response();
    };
    let Some((owner, name)) = parse_repo(repo_str) else {
        tracing::warn!(%session_id, repo = %repo_str, "pr_draft: stored repo has bad format");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "stored repo has bad format"})),
        )
            .into_response();
    };
    if session.branch.as_deref().is_none() {
        tracing::warn!(%session_id, "pr_draft: session has no branch");
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "session has no branch"})),
        )
            .into_response();
    }

    let Some(client) = crate::github::Client::from_env() else {
        tracing::warn!(%session_id, "pr_draft: GITHUB_TOKEN not set");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };

    // Default base: ask GitHub; fall back to "main" on error so the
    // operator still gets a usable draft if the API blip-fails.
    let base = match client.default_branch(owner, name).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, "pr_draft: default_branch lookup failed; falling back to 'main'");
            "main".to_string()
        }
    };

    let worktree = std::path::PathBuf::from(&session.worktree_path);
    let (commits_ahead, commits_behind) = commits_ahead_behind(&worktree, &base);
    let body = build_pr_body(&session, &worktree, &base);
    let title = truncate_title(&session.task_summary, 72);

    Json(PrDraftResponse {
        title,
        body,
        base,
        commits_ahead,
        commits_behind,
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Plan 10b Task 6 — `POST /api/sessions/:id/pr`.
//
// Pushes the session's branch via the `github::push` helper, then opens a
// PR via `github::Client::create_pull_request`. On success persists the
// returned `html_url` into `sessions.pr_url` and emits the
// `SessionPrCreated` domain event. Reuses `resolve_session_for_pr` and
// `parse_repo` helpers from the pr-draft handler above.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct OpenPrReq {
    pub title: String,
    pub body: String,
    pub base: String,
}

#[derive(Debug, Serialize)]
pub struct OpenPrResp {
    pub pr_url: String,
    pub pr_number: u64,
}

/// `POST /api/sessions/:id/pr` — push the session's branch and open a PR.
/// Returns 200 with `{pr_url, pr_number}` on success. 422 if a PR already
/// exists (`session.pr_url` is set); 503 if no token; 502 on push or PR
/// creation failure (with token-redacted error from push or octocrab
/// error from PR creation).
pub async fn open_pr(
    State(mgr): State<Arc<SessionManager>>,
    Path(session_id): Path<Uuid>,
    Json(req): Json<OpenPrReq>,
) -> Response {
    let (session, project) = match resolve_session_for_pr(&mgr, session_id) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    if let Some(existing) = session.pr_url.as_ref() {
        tracing::warn!(%session_id, %existing, "open_pr rejected: PR already exists");
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": "PR already exists for this session",
                "pr_url": existing,
            })),
        )
            .into_response();
    }

    let Some(repo_str) = project.github_repo.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no linked repo"})),
        )
            .into_response();
    };
    let Some((owner, name)) = parse_repo(repo_str) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "stored repo has bad format"})),
        )
            .into_response();
    };
    let Some(branch) = session.branch.as_deref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "session has no branch"})),
        )
            .into_response();
    };

    let Some(client) = crate::github::Client::from_env() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

    // Push branch first.
    let worktree = std::path::PathBuf::from(&session.worktree_path);
    if let Err(e) = crate::github::push::push_branch(&worktree, branch, owner, name, &token) {
        tracing::warn!(%session_id, error = %e, "git push failed");
        return (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("{e}")})),
        )
            .into_response();
    }

    // Create PR.
    let pr_info = match client
        .create_pull_request(owner, name, branch, &req.base, &req.title, &req.body, false)
        .await
    {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!(%session_id, error = %e, "create_pull_request failed");
            return (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("{e}")})),
            )
                .into_response();
        }
    };

    // Persist + emit event.
    if let Err(e) = mgr.db().set_session_pr_url(session_id, Some(&pr_info.html_url)) {
        tracing::warn!(%session_id, error = ?e, "set_session_pr_url failed after PR created");
        return crate::api::err_to_response(e);
    }
    mgr.event_bus()
        .publish(crate::api::events::DomainEvent::SessionPrCreated {
            session_id,
            project_id: session.project_id,
            pr_url: pr_info.html_url.clone(),
            pr_number: pr_info.number,
        });

    Json(OpenPrResp {
        pr_url: pr_info.html_url,
        pr_number: pr_info.number,
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// Plan 10c — `GET /api/sessions/:id/pr-reviews`.
//
// Returns top-level reviews + inline comments for the session's linked PR.
// 404 if session has no pr_url; 503 if no token; 502 on GitHub error;
// 500 if the stored repo or pr_url has an unexpected shape.
// ---------------------------------------------------------------------------

/// Plan 10c — extract PR number from a GitHub PR URL.
/// Matches `https://github.com/<owner>/<name>/pull/<number>` (with or
/// without trailing path/fragment). Returns None if shape unexpected.
fn parse_pr_number(url: &str) -> Option<u64> {
    let after_pull = url.split("/pull/").nth(1)?;
    let num_str: String = after_pull
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num_str.parse().ok()
}

/// `GET /api/sessions/:id/pr-reviews` — returns reviews + inline comments
/// for the session's linked PR. 404 if session has no pr_url; 503 if no
/// token; 502 on GitHub error; 500 if pr_url has unexpected shape.
pub async fn pr_reviews(
    State(mgr): State<Arc<SessionManager>>,
    Path(session_id): Path<Uuid>,
) -> Response {
    let (session, project) = match resolve_session_for_pr(&mgr, session_id) {
        Ok(s) => s,
        Err(resp) => return resp,
    };

    let Some(pr_url) = session.pr_url.as_deref() else {
        tracing::warn!(%session_id, "pr_reviews: session has no PR url");
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no PR for this session"})),
        )
            .into_response();
    };
    let Some(pr_number) = parse_pr_number(pr_url) else {
        tracing::warn!(%session_id, %pr_url, "pr_reviews: pr_url has unexpected shape");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "could not parse PR number from url"})),
        )
            .into_response();
    };

    let Some(repo_str) = project.github_repo.as_ref() else {
        tracing::warn!(%session_id, "pr_reviews: project has no linked repo");
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no linked repo"})),
        )
            .into_response();
    };
    let Some((owner, name)) = parse_repo(repo_str) else {
        tracing::warn!(%session_id, repo = %repo_str, "pr_reviews: stored repo has bad format");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "stored repo has bad format"})),
        )
            .into_response();
    };

    let Some(client) = crate::github::Client::from_env() else {
        tracing::warn!(%session_id, "pr_reviews: GITHUB_TOKEN not set");
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "GITHUB_TOKEN not set on server"})),
        )
            .into_response();
    };

    match client.list_pr_reviews(owner, name, pr_number).await {
        Ok(bundle) => Json(bundle).into_response(),
        Err(e) => {
            tracing::warn!(%session_id, %pr_number, error = %e, "list_pr_reviews failed");
            (
                StatusCode::BAD_GATEWAY,
                Json(serde_json::json!({"error": format!("{e}")})),
            )
                .into_response()
        }
    }
}

/// Helper: load session + parent project, returning the appropriate
/// error response on missing session/project or DB failure. Mirrors the
/// `resolve_linked_repo` shape but works at session granularity since
/// the pr-draft handler keys off a session id, not a project id.
#[allow(
    clippy::result_large_err,
    reason = "boundary-type Response is consumed at the handler immediately, no cloning"
)]
fn resolve_session_for_pr(
    mgr: &Arc<SessionManager>,
    session_id: Uuid,
) -> Result<(crate::db::Session, crate::db::Project), Response> {
    let session = match mgr.db().get_session(session_id) {
        Ok(s) => s,
        Err(crate::error::Error::SessionNotFound(_))
        | Err(crate::error::Error::Db(rusqlite::Error::QueryReturnedNoRows)) => {
            tracing::warn!(%session_id, "pr_draft: session not found");
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "session not found"})),
            )
                .into_response());
        }
        Err(e) => {
            tracing::warn!(%session_id, error = %e, "pr_draft: session lookup failed");
            return Err(crate::api::err_to_response(e));
        }
    };
    let project = match mgr.db().get_project(session.project_id) {
        Ok(p) => p,
        Err(crate::error::Error::ProjectNotFound(_))
        | Err(crate::error::Error::Db(rusqlite::Error::QueryReturnedNoRows)) => {
            tracing::warn!(%session_id, project_id = %session.project_id, "pr_draft: parent project not found");
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "project not found"})),
            )
                .into_response());
        }
        Err(e) => {
            tracing::warn!(%session_id, error = %e, "pr_draft: project lookup failed");
            return Err(crate::api::err_to_response(e));
        }
    };
    Ok((session, project))
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
