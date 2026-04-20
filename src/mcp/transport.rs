//! MCP Streamable HTTP transport.
//!
//! Per MCP 2025-03-26, the client sends a single JSON-RPC request as the body
//! of a POST. The server responds either with `application/json` (default) or
//! upgrades to a one-shot `text/event-stream` framing if the client advertises
//! `Accept: text/event-stream`. We don't open a persistent SSE channel; only
//! the framing differs.
//!
//! Auth: bearer token in the `Authorization` header, resolved through
//! [`crate::auth::resolve_token`] against `sessions.api_token`.

use crate::mcp::dispatch;
use crate::mcp::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use crate::session_manager::SessionManager;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Extracts the bearer token from `Authorization: Bearer <token>`.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim())
}

/// Returns true when the client wants `text/event-stream` framing.
fn wants_sse(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.contains("text/event-stream"))
        .unwrap_or(false)
}

/// Encode a JSON-RPC response body either as JSON or as a single SSE event.
fn encode(resp: &JsonRpcResponse, sse: bool) -> Response {
    let body = serde_json::to_string(resp).unwrap_or_else(|_| {
        // Should never fail; guard with a minimal -32603 envelope.
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialize failed"}}"#
            .to_string()
    });
    if sse {
        let framed = format!("event: message\ndata: {body}\n\n");
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/event-stream")],
            framed,
        )
            .into_response()
    } else {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response()
    }
}

/// Build a top-level error envelope (id=null) when the request couldn't even be
/// parsed. Always returned with HTTP 200 per JSON-RPC convention so the client
/// reads the body for the actual error code.
fn error_envelope(err: JsonRpcError, sse: bool) -> Response {
    encode(&JsonRpcResponse::error(None, err), sse)
}

pub async fn handle(
    State(mgr): State<Arc<SessionManager>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let sse = wants_sse(&headers);

    // 1) Auth — resolve bearer → session id, before parsing the body.
    let token = match bearer(&headers) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => {
            return error_envelope(
                JsonRpcError::unauthorized("missing or malformed Authorization header"),
                sse,
            );
        }
    };
    let session_id = match crate::auth::resolve_token(mgr.db(), &token) {
        Ok(sid) => sid,
        Err(_) => {
            return error_envelope(JsonRpcError::unauthorized("invalid bearer token"), sse);
        }
    };

    // 2) Parse JSON-RPC envelope.
    let req: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return error_envelope(JsonRpcError::parse_error(e.to_string()), sse);
        }
    };
    if let Err(msg) = req.validate() {
        let id = req.id.clone();
        return encode(
            &JsonRpcResponse::error(id, JsonRpcError::invalid_request(msg)),
            sse,
        );
    }

    // 3) Dispatch.
    let resp = dispatch::dispatch(mgr, session_id, req).await;
    encode(&resp, sse)
}
