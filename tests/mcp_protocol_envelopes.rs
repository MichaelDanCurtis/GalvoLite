//! JSON-RPC 2.0 envelope serialization round-trip tests.
//!
//! These exercise the [`galvolite::mcp::protocol`] types directly without the
//! HTTP transport, so they remain fast and deterministic.

use galvolite::mcp::protocol::{
    JsonRpcError, JsonRpcId, JsonRpcRequest, JsonRpcResponse, ToolContent, ToolsCallResult,
};
use serde_json::json;

#[test]
fn request_round_trips_string_id() {
    let raw = r#"{"jsonrpc":"2.0","id":"abc","method":"tools/list","params":null}"#;
    let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
    assert!(req.validate().is_ok());
    assert_eq!(req.method, "tools/list");
    matches!(req.id, Some(JsonRpcId::String(_)));
}

#[test]
fn request_round_trips_numeric_id() {
    let raw = r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#;
    let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
    assert!(req.validate().is_ok());
    matches!(req.id, Some(JsonRpcId::Number(7)));
}

#[test]
fn validate_rejects_wrong_version() {
    let raw = r#"{"jsonrpc":"1.0","method":"initialize"}"#;
    let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
    assert!(req.validate().is_err());
}

#[test]
fn success_response_serializes_without_error_field() {
    let resp = JsonRpcResponse::success(Some(JsonRpcId::Number(1)), json!({"ok": true}));
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains("\"result\""));
    assert!(!s.contains("\"error\""));
    assert!(s.contains("\"jsonrpc\":\"2.0\""));
}

#[test]
fn error_response_serializes_without_result_field() {
    let resp =
        JsonRpcResponse::error(Some(JsonRpcId::Number(1)), JsonRpcError::method_not_found("xxx"));
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains("\"error\""));
    assert!(!s.contains("\"result\""));
    assert!(s.contains("-32601"));
}

#[test]
fn error_codes_match_jsonrpc_spec() {
    assert_eq!(JsonRpcError::PARSE_ERROR, -32700);
    assert_eq!(JsonRpcError::INVALID_REQUEST, -32600);
    assert_eq!(JsonRpcError::METHOD_NOT_FOUND, -32601);
    assert_eq!(JsonRpcError::INVALID_PARAMS, -32602);
    assert_eq!(JsonRpcError::INTERNAL_ERROR, -32603);
}

#[test]
fn tools_call_result_omits_is_error_when_false() {
    let r = ToolsCallResult {
        content: vec![ToolContent::Text {
            text: "hi".to_string(),
        }],
        is_error: false,
    };
    let s = serde_json::to_string(&r).unwrap();
    assert!(!s.contains("isError"));
}

#[test]
fn tools_call_result_includes_is_error_when_true() {
    let r = ToolsCallResult {
        content: vec![ToolContent::Text {
            text: "boom".to_string(),
        }],
        is_error: true,
    };
    let s = serde_json::to_string(&r).unwrap();
    assert!(s.contains("\"isError\":true"));
}

#[test]
fn tool_content_text_uses_type_tag() {
    let c = ToolContent::Text {
        text: "hello".to_string(),
    };
    let s = serde_json::to_string(&c).unwrap();
    assert!(s.contains("\"type\":\"text\""));
    assert!(s.contains("\"text\":\"hello\""));
}
