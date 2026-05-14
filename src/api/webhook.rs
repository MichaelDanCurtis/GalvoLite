//! Plan 10e — GitHub webhook receiver.
//!
//! Validates HMAC signature, parses JSON, routes by `X-GitHub-Event` header,
//! looks up the matching project by `repository.full_name`, and emits the
//! appropriate `DomainEvent`. Returns 200 noop for events we don't translate
//! and for repos with no matching GalvoLite project.

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;
use std::sync::Arc;

use crate::session_manager::SessionManager;

/// `POST /api/github/webhook` — see module docs.
pub async fn webhook_handler(
    State(mgr): State<Arc<SessionManager>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let secret = match std::env::var("GITHUB_WEBHOOK_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            return (StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "GITHUB_WEBHOOK_SECRET not set"}))).into_response();
        }
    };

    let sig_header = headers.get("x-hub-signature-256")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    if let Err(e) = crate::github::webhook::verify_signature(&body, sig_header, &secret) {
        tracing::warn!(error = %e, "webhook signature verification failed");
        return (StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": format!("{e}")}))).into_response();
    }

    let event_type = headers.get("x-github-event")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    // GitHub's webhook test ping
    if event_type == "ping" {
        return Json(serde_json::json!({"status": "pong"})).into_response();
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("invalid JSON: {e}")}))).into_response();
        }
    };

    // Look up the matching project by repository.full_name.
    let repo_full_name = payload.get("repository")
        .and_then(|r| r.get("full_name"))
        .and_then(|n| n.as_str())
        .unwrap_or("");

    let project = mgr.db().list_projects()
        .ok()
        .and_then(|ps| ps.into_iter().find(|p| p.github_repo.as_deref() == Some(repo_full_name)));

    let Some(project) = project else {
        return Json(serde_json::json!({"status": "noop", "reason": "no matching project"})).into_response();
    };

    // Translate event by type.
    match event_type {
        "pull_request" => {
            let action = payload.get("action").and_then(|a| a.as_str()).unwrap_or("");
            let merged = payload.get("pull_request")
                .and_then(|pr| pr.get("merged"))
                .and_then(|m| m.as_bool())
                .unwrap_or(false);
            if action == "closed" && merged {
                let pr_url = payload.get("pull_request")
                    .and_then(|pr| pr.get("html_url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let pr_number = payload.get("pull_request")
                    .and_then(|pr| pr.get("number"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                mgr.event_bus().publish(crate::api::events::DomainEvent::PullRequestMerged {
                    project_id: project.id,
                    pr_url,
                    pr_number,
                });
                return Json(serde_json::json!({"status": "ok", "event": "pull_request_merged"})).into_response();
            }
        }
        "issues" => {
            let action = payload.get("action").and_then(|a| a.as_str()).unwrap_or("");
            if action == "opened" {
                let issue_url = payload.get("issue")
                    .and_then(|i| i.get("html_url"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string();
                let issue_number = payload.get("issue")
                    .and_then(|i| i.get("number"))
                    .and_then(|n| n.as_u64())
                    .unwrap_or(0);
                let title = payload.get("issue")
                    .and_then(|i| i.get("title"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                mgr.event_bus().publish(crate::api::events::DomainEvent::IssueOpened {
                    project_id: project.id,
                    issue_url,
                    issue_number,
                    title,
                });
                return Json(serde_json::json!({"status": "ok", "event": "issue_opened"})).into_response();
            }
        }
        _ => {} // unknown event → fall through to noop
    }

    Json(serde_json::json!({"status": "noop", "reason": "unhandled event"})).into_response()
}
