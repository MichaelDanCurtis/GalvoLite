//! JSON-RPC 2.0 envelopes + MCP-specific request/response shapes.
//!
//! Reference: <https://www.jsonrpc.org/specification> (envelope) and
//! <https://spec.modelcontextprotocol.io/specification/2025-03-26/> (MCP methods).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<JsonRpcId>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn validate(&self) -> Result<(), String> {
        if self.jsonrpc != "2.0" {
            return Err(format!("unsupported jsonrpc version: {}", self.jsonrpc));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<JsonRpcId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<JsonRpcId>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn error(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const UNAUTHORIZED: i32 = -32001; // MCP-specific extension

    pub fn parse_error(msg: impl Into<String>) -> Self {
        Self {
            code: Self::PARSE_ERROR,
            message: msg.into(),
            data: None,
        }
    }
    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            code: Self::INVALID_REQUEST,
            message: msg.into(),
            data: None,
        }
    }
    pub fn method_not_found(method: impl Into<String>) -> Self {
        let m = method.into();
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("method not found: {m}"),
            data: None,
        }
    }
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: Self::INVALID_PARAMS,
            message: format!("invalid params: {}", msg.into()),
            data: None,
        }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: Self::INTERNAL_ERROR,
            message: msg.into(),
            data: None,
        }
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            code: Self::UNAUTHORIZED,
            message: msg.into(),
            data: None,
        }
    }
}

// MCP method shapes —————————————————————————————————————————————————————

#[derive(Debug, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// We declare a static catalog at startup; no list_changed notifications.
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsCallResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not", default)]
    pub is_error: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}
