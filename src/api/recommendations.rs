//! Plan 8 Task 13 — Recommendations REST: list endpoint.
//!
//! `GET /api/projects/:id/recommendations[?status=pending|acted|dismissed|deferred]`
//!
//! Returns the rows from [`crate::recommendations::list_by_status`] as JSON.
//! The `status` query parameter defaults to `pending`. Unknown values yield
//! `400 Bad Request` (we deliberately do not silently coerce — the dashboard
//! should pass a known value or omit the param entirely).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use crate::recommendations::{self, RecStatus};
use crate::session_manager::SessionManager;

#[derive(Deserialize)]
pub struct ActReq {
    pub rec_ids: Vec<Uuid>,
    pub action: String,
}

#[derive(Deserialize)]
pub struct ListQ {
    pub status: Option<String>,
}

pub async fn list(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
    Query(q): Query<ListQ>,
) -> Response {
    let status = match q.status.as_deref() {
        None | Some("pending") => RecStatus::Pending,
        Some("acted") => RecStatus::Acted,
        Some("dismissed") => RecStatus::Dismissed,
        Some("deferred") => RecStatus::Deferred,
        _ => return (StatusCode::BAD_REQUEST, "bad status").into_response(),
    };
    match recommendations::list_by_status(mgr.db(), project_id, status) {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => crate::api::err_to_response(e),
    }
}

/// `POST /api/projects/:id/recommendations/act` — body
/// `{ "rec_ids": [...], "action": "make"|"dismiss" }`.
///
/// `make`: append a single concatenated block to the project's orchestrator's
/// `pending_overseer_recs` (append-only — never overwrites prior content), then
/// flip each rec to `acted`. Race protection: only currently-`pending` recs
/// belonging to `project_id` are included; if all rec_ids fail that filter the
/// response is `200 {"results": []}` (no orchestrator side-effect, no error).
/// If no top-level session exists for the project, returns `409`.
///
/// `dismiss`: flip each rec to `dismissed`. No orchestrator required.
///
/// Any other action string → `400`.
pub async fn act(
    State(mgr): State<Arc<SessionManager>>,
    Path(project_id): Path<Uuid>,
    Json(req): Json<ActReq>,
) -> Response {
    if req.action == "make" {
        let Some(orch) = mgr.find_orchestrator_for_project(project_id) else {
            return (StatusCode::CONFLICT, "no orchestrator").into_response();
        };
        // Filter to currently-pending recs scoped to this project (race
        // protection — a concurrent dismiss/defer must not be silently
        // re-acted, and a stray rec_id from another project must not leak
        // into this orchestrator's injection block).
        let pending: Vec<recommendations::Recommendation> = req
            .rec_ids
            .iter()
            .filter_map(|id| recommendations::get(mgr.db(), *id).ok())
            .filter(|r| r.status == "pending" && r.project_id == project_id)
            .collect();

        if pending.is_empty() {
            return (StatusCode::OK, Json(serde_json::json!({"results": []})))
                .into_response();
        }
        let block = concat_for_injection(&pending);
        if let Err(e) = mgr.append_pending_overseer_recs(orch.id, &block) {
            return crate::api::err_to_response(e);
        }
        let mut results = Vec::new();
        for r in &pending {
            let outcome = match recommendations::set_status(mgr.db(), r.id, RecStatus::Acted) {
                Ok(()) => "acted",
                Err(_) => "error",
            };
            results.push(serde_json::json!({"rec_id": r.id, "outcome": outcome}));
            mgr.event_bus()
                .publish(crate::api::events::DomainEvent::RecommendationActed {
                    rec_id: r.id,
                    action: "make".into(),
                    route: Some(orch.id),
                });
        }
        return Json(serde_json::json!({"results": results})).into_response();
    }
    if req.action == "dismiss" {
        for id in &req.rec_ids {
            let _ = recommendations::set_status(mgr.db(), *id, RecStatus::Dismissed);
            mgr.event_bus()
                .publish(crate::api::events::DomainEvent::RecommendationActed {
                    rec_id: *id,
                    action: "dismiss".into(),
                    route: None,
                });
        }
        return Json(serde_json::json!({"dismissed": req.rec_ids.len()})).into_response();
    }
    (StatusCode::BAD_REQUEST, "action must be 'make' or 'dismiss'").into_response()
}

/// Build the markdown block injected into the orchestrator's
/// `pending_overseer_recs`. Format is intentionally human-readable so the
/// orchestrator's next handoff can paste it into context with no further
/// processing.
pub fn concat_for_injection(recs: &[recommendations::Recommendation]) -> String {
    let mut s = format!("## Apply these {} recommendation(s):\n\n", recs.len());
    for (i, r) in recs.iter().enumerate() {
        s.push_str(&format!(
            "### Rec {}: {}\n\n{}\n\n_Rationale: {}_\n",
            i + 1,
            r.title,
            r.body,
            r.rationale
        ));
        if i + 1 < recs.len() {
            s.push_str("\n---\n\n");
        }
    }
    s
}
