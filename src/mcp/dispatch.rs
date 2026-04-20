//! MCP method dispatcher.
//!
//! Routes JSON-RPC method names to handlers:
//!  * `initialize`    → handshake (Task 4)
//!  * `tools/list`    → registry descriptors (Task 4)
//!  * `tools/call`    → registry handler invocation (Task 5)
//!
//! Anything else returns JSON-RPC `method_not_found` (-32601).

use crate::mcp::protocol::*;
use crate::session_manager::SessionManager;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

pub async fn dispatch(
    mgr: Arc<SessionManager>,
    session_id: Uuid,
    req: JsonRpcRequest,
) -> JsonRpcResponse {
    match req.method.as_str() {
        "initialize" => {
            let result = InitializeResult {
                protocol_version: "2025-03-26".to_string(),
                capabilities: ServerCapabilities {
                    tools: ToolsCapability {
                        list_changed: false,
                    },
                },
                server_info: ServerInfo {
                    name: "galvolite".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            };
            JsonRpcResponse::success(req.id, serde_json::to_value(result).unwrap())
        }
        "tools/list" => {
            let tools = mgr.mcp_registry().list_descriptors();
            JsonRpcResponse::success(req.id, json!({ "tools": tools }))
        }
        "tools/call" => {
            // Parse params.
            let params: ToolsCallParams = match req
                .params
                .clone()
                .ok_or_else(|| "missing params".to_string())
                .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
            {
                Ok(p) => p,
                Err(e) => {
                    return JsonRpcResponse::error(req.id, JsonRpcError::invalid_params(e));
                }
            };

            let Some(def) = mgr.mcp_registry().get(&params.name) else {
                return JsonRpcResponse::error(
                    req.id,
                    JsonRpcError::method_not_found(format!(
                        "tools/call: unknown tool '{}'",
                        params.name
                    )),
                );
            };

            let ctx = crate::mcp::registry::ToolCtx {
                mgr: mgr.clone(),
                session_id,
            };
            let outcome = (def.handler)(ctx, params.arguments).await;

            match outcome {
                crate::mcp::registry::ToolOutcome::Ok(value) => {
                    let text = match serde_json::to_string(&value) {
                        Ok(s) => s,
                        Err(e) => {
                            return JsonRpcResponse::error(
                                req.id,
                                JsonRpcError::internal(format!(
                                    "serialize tool result failed: {e}"
                                )),
                            );
                        }
                    };
                    let result = ToolsCallResult {
                        content: vec![ToolContent::Text { text }],
                        is_error: false,
                    };
                    JsonRpcResponse::success(req.id, serde_json::to_value(result).unwrap())
                }
                crate::mcp::registry::ToolOutcome::AppError(msg) => {
                    let result = ToolsCallResult {
                        content: vec![ToolContent::Text { text: msg }],
                        is_error: true,
                    };
                    JsonRpcResponse::success(req.id, serde_json::to_value(result).unwrap())
                }
                crate::mcp::registry::ToolOutcome::InvalidParams(msg) => {
                    JsonRpcResponse::error(req.id, JsonRpcError::invalid_params(msg))
                }
            }
        }
        _ => JsonRpcResponse::error(req.id, JsonRpcError::method_not_found(req.method)),
    }
}
