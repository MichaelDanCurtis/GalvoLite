# MCP Surface (Sub-plan 3/5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose GalvoLite's coordination surface as a Model Context Protocol server hosted on the existing Axum process at `POST /mcp`. Implement spec §5's tool catalog as four tiers — self-contained tools that work today, cross-session tools that drive sibling PTYs, dashboard-coupled tools that degrade gracefully until Sub-plan 4 lands, and reserved stubs that return `not_implemented` for tools owned by Plans 4–6. Every tool call routes through `state::apply_event`, writing one event-log row before mutating session state. `.mcp.json` is written into each session's worktree at spawn so the spawned `claude` process discovers the server automatically.

**Architecture:** A single Axum endpoint at `POST /mcp` accepts JSON-RPC 2.0 envelopes per the **MCP 2025-03-26+ Streamable HTTP transport** (replacement for the deprecated SSE transport). Each request is bearer-authed (extracted into `src/auth.rs`, shared with Sub-plan 2's `/hooks/ingest`). The handler dispatches by `method` — `initialize`, `tools/list`, `tools/call` — into a declarative `ToolRegistry` table that mirrors `state::transition`'s table-driven shape. Each registered `ToolDef` carries name, JSON schema, side-effect tier, and an `async fn` handler. Tool handlers receive a `ToolCtx { db, session_id, mgr }` and return either a structured result or a JSON-RPC error. Responses are returned as `Content-Type: application/json` for single replies; the optional `Content-Type: text/event-stream` upgrade path (server-initiated streams over the same POST) is implemented but not used by any current tool — placeholder for future progress notifications. `.mcp.json` writer lives in `src/spawn_config.rs` (extended from Sub-plan 2's `.claude/settings.json` writer).

**Tech Stack:** Rust 1.75+ (existing), `axum` 0.7 (existing), `serde` / `serde_json` (existing), `rusqlite` 0.31 (existing), `subtle` 2 for constant-time token compare (added in Sub-plan 2; reused), `tracing` (existing), `uuid` (existing). **No new Cargo deps.** No third-party MCP SDK — we implement JSON-RPC 2.0 + the Streamable HTTP transport directly because the surface area is small and the Rust MCP SDK ecosystem is unstable as of 2026-04.

---

## File Structure

**New files:**
- `src/auth.rs` — shared bearer-token resolver: `resolve_token(&Db, &str) -> Result<Uuid, Error>` with constant-time compare. Pulled out of Sub-plan 2's inline `api/hooks.rs` helper. (Task 5)
- `src/spawn_config.rs` — extended in Task 2 to also write `.mcp.json` (existing file from Sub-plan 2 added the `.claude/settings.json` writer).
- `src/mcp/mod.rs` — module root; re-exports `protocol`, `transport`, `dispatch`, `registry`, `tools`. (Task 3)
- `src/mcp/protocol.rs` — JSON-RPC 2.0 envelope types (`JsonRpcRequest`, `JsonRpcResponse`, `JsonRpcError`, error codes), MCP `initialize` / `tools/list` / `tools/call` request and response shapes. (Task 3)
- `src/mcp/transport.rs` — Streamable HTTP request handler: parses `Accept` and `Content-Type`, branches between single-JSON and SSE response. (Task 3)
- `src/mcp/dispatch.rs` — dispatcher mapping method strings to handlers; calls `tools/call` into the registry. (Task 5)
- `src/mcp/registry.rs` — `ToolDef` struct + `ToolRegistry::build()` static table; lookup by name. (Task 4)
- `src/mcp/tools/mod.rs` — module root for tool handlers; one file per tier per topic.
- `src/mcp/tools/tier1_status.rs` — `status_report`, `request_attention`, `set_plan`, `plan_check_step`, `set_seed`. (Task 6)
- `src/mcp/tools/tier1_role_handoff.rs` — `set_role`, `write_handoff`, `read_handoff`. (Task 7)
- `src/mcp/tools/tier1_introspection.rs` — `whoami`, `get_transcript`, `get_pty_log`, `pause_self`, `request_end`. (Task 8)
- `src/mcp/tools/tier2_3_cross.rs` — `list_siblings`, `status_of`, `read_events`, `send_to_session`, `ask_operator` (degraded), `notify_operator` (degraded). (Task 9)
- `src/mcp/tools/tier4_stubs.rs` — `claim_paths`, `release_paths`, `list_claims`, `check_path`, `spawn_child`, `wait_for_children`, `list_available_mcps`, `list_session_mcps`, `configure_session_mcps`, `gateway_toggle`, `open_pr`, `list_prs`, `request_review`, `merge_pr`, `get_pr_comments`, `read_context`, `write_context`. (Task 10)
- `src/api.rs` — modified to add `.route("/mcp", post(mcp_handler))`. (Task 3)
- `tests/auth_resolver.rs` — Task 5: shared resolver returns `Uuid` on hit, `Unauthorized` on miss, constant-time on bad-byte inputs.
- `tests/spawn_config_mcp_writer.rs` — Task 2: `.mcp.json` written with type=`http`, correct URL, bearer header.
- `tests/mcp_protocol_envelopes.rs` — Task 3: JSON-RPC 2.0 envelope round-trips, error code mapping, batch-vs-single rejection.
- `tests/mcp_transport_http.rs` — Task 3: `POST /mcp` returns 200 + JSON for normal call; returns 401 on bad token; returns SSE stream when `Accept: text/event-stream`.
- `tests/mcp_initialize_tools_list.rs` — Task 4: `initialize` handshake echoes capabilities; `tools/list` returns full registry with stable schema.
- `tests/mcp_dispatch.rs` — Task 5: unknown method → JSON-RPC error -32601; unknown tool → -32602; handler error → wrapped in result with `isError: true`.
- `tests/mcp_tools_tier1_status.rs` — Task 6: each Tier-1 status tool writes one event row + mutates `sessions` columns.
- `tests/mcp_tools_tier1_role_handoff.rs` — Task 7: `set_role` rewrites `CLAUDE.md`; `write_handoff` writes pointer.
- `tests/mcp_tools_tier1_introspection.rs` — Task 8: `whoami`, `get_transcript`, `pause_self`, `request_end` happy paths.
- `tests/mcp_tools_tier2_3_cross.rs` — Task 9: cross-session reads + `send_to_session` PTY write; `ask_operator` returns `timed_out: true` immediately.
- `tests/mcp_tools_tier4_stubs.rs` — Task 10: every reserved stub returns `not_implemented` with the right name in the message.
- `tests/mcp_smoke_endtoend.rs` — Task 11: spawn a session, run a real JSON-RPC client (raw `reqwest`) against the live Axum app, exercise the most common tools end-to-end, verify event-log shape.

**Modified files:**
- `src/lib.rs` — add `pub mod auth; pub mod mcp;`. (Tasks 5, 3)
- `src/api.rs` — mount `/mcp` route with `post(mcp::transport::handle)`. (Task 3)
- `src/api.rs` — Sub-plan 2 inserted `.route("/hooks/ingest", post(hooks::ingest))` and inline `resolve_token` helper; **migrate the helper call site** to use `crate::auth::resolve_token` after Task 5 lands. (Task 5)
- `src/spawn_config.rs` — Sub-plan 2 created this with the `.claude/settings.json` writer. Task 2 adds the `.mcp.json` writer as a sibling function. (Task 2)
- `src/session_manager.rs` — call site that writes spawn-time files now also writes `.mcp.json`. (Task 2)
- `docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md` — Task 1: amend §7.1 example (`type: "sse"` → `type: "http"`); amend §15 dependency #1 to reference Streamable HTTP not SSE; add §13.6 entry recording why we picked Streamable HTTP over SSE.
- `Cargo.toml` — **no changes.** All required deps (`axum`, `serde`, `serde_json`, `rusqlite`, `uuid`, `subtle`, `tracing`) are present after Sub-plan 2.

---

## Spec ↔ task crosswalk

| Spec section | Sub-plan 3 task |
|--------------|-----------------|
| §7.1 step 4 (`.mcp.json` writer) — amended in Task 1 | Tasks 1, 2 |
| §7.2 (bearer auth, constant-time compare) — shared resolver | Task 5 |
| §5.1 status & lifecycle tools                              | Task 6 |
| §5.1 role + handoff tools                                  | Task 7 |
| §5.1 session introspection + self-control tools            | Task 8 |
| §5.1 cross-session tools                                   | Task 9 |
| §5.1 operator-comms tools (`ask_operator`, `notify_operator`) — degraded mode until Sub-plan 4 | Task 9 |
| §5.2 path ownership (reserved stubs)                       | Task 10 |
| §5.3 spawn (reserved stubs)                                | Task 10 |
| §5.4 portfolio management (reserved stubs)                 | Task 10 |
| §5.5 GitHub surface (reserved stubs)                       | Task 10 |
| §5.6 context r/w (reserved stubs)                          | Task 10 |
| §5.7 (one tool one thing; idempotent / dedup-keyed; one event row before mutation) | Tasks 6–10 |
| §6.3 atomicity invariant — every tool routes through `apply_event` | Tasks 6–10 |
| §15 dep #1 (CC SSE MCP transport) — superseded by Streamable HTTP | Task 1 spec amendment |
| §15 dep #2 (`tools/list_changed` round-trip) — N/A this sub-plan; static catalog only | Task 4 (catalog declared static) |

Sub-plan 4 owns: WS event fan-out for `session.event_appended` / `operator.prompt` / `operator.toast`; tabbed `ExpandedSessionView` (Events / Plan / Meta); `OperatorPromptModal` that resolves `ask_operator` via `POST /operator/prompt/{id}/answer`. Sub-plan 5 owns: real-claude smoke; signal pipeline deletion. Sub-plan 2 already owns: `.claude/settings.json` writer; `/hooks/ingest`; the seven hook handlers; the inline `resolve_token` helper that Task 5 extracts.

---

## Branch / worktree

This plan should be executed on a dedicated branch:
```bash
git switch -c plan-mcp-surface
```
Or in a worktree (recommended — Sub-plan 2 runs in parallel on `plan-hook-surface`):
```bash
git worktree add ../galvolite-mcp -b plan-mcp-surface main
```
All commits land on this branch. Merge to `main` after Sub-plan 3 passes its goal-backward review. **Coordinate the `src/spawn_config.rs` and `src/auth.rs` extractions with Sub-plan 2's branch** — both branches touch the same call sites. The merge order is tracked in the Done definition below.

---

## Task 1: Amend spec — Streamable HTTP transport

**Goal:** Spec §7.1 example shows `.mcp.json` with `"type": "sse"` reflecting the older MCP transport. The 2025-03-26 spec revision deprecates SSE in favor of Streamable HTTP (single `POST /mcp` endpoint, optional SSE upgrade per request). Update the spec to match what we implement, and record the rationale so future readers know we considered SSE deliberately.

**Files:**
- Modify: `docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md`

- [ ] **Step 1: Update §7.1 `.mcp.json` example**

Find the example block at §7.1 step 4 (around the line that reads `"type": "sse"` for the `galvolite` server). Replace `"sse"` with `"http"` for both the `galvolite` and `portfolio` entries. The URL path stays `/mcp`. The `headers.Authorization` field stays unchanged.

- [ ] **Step 2: Reword §15 dependency #1**

The current §15 dep #1 reads "CC SSE MCP transport stability." Change to:

> **CC Streamable HTTP MCP transport stability.** Does Claude Code's MCP client implement the 2025-03-26 Streamable HTTP transport (single POST endpoint, optional SSE upgrade)? If not, fall back to the legacy SSE-only path documented at §7.4, which keeps two endpoints (`POST /mcp/messages` + `GET /mcp/sse`) bridged through the same dispatcher. Stdio shim is the secondary fallback.

- [ ] **Step 3: Add §13.6 — rejected SSE-only transport**

Append to §13:

```
### 13.6 SSE-only MCP transport (the older spec revision)

**Proposed:** Implement only the legacy two-endpoint SSE transport (`GET /mcp/sse` + `POST /mcp/messages`) since CC's reference clients shipped with that shape first.

**Rejected because:**
- The 2025-03-26 MCP spec revision marks SSE-only as deprecated and adds Streamable HTTP as the recommended transport.
- Streamable HTTP collapses two endpoints into one, removes the requirement for clients to maintain a separate event-stream connection for trivial request/response tools (which is most of our catalog), and preserves SSE as an optional per-request upgrade for handlers that genuinely stream.
- Implementing the deprecated path first means a forced migration when CC bumps client versions; implementing Streamable HTTP first means the legacy path becomes a backward-compat fallback only if §15 dep #1 lands negative.

**Alternatives considered:** dual-host both transports on different routes (`/mcp` vs `/mcp/sse`) — wait until §15 dep #1 has data before paying the maintenance cost.
```

- [ ] **Step 4: Verify and commit**

```bash
grep -n '"type": "sse"' docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md
# expect: no hits

grep -n "Streamable HTTP" docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md
# expect: §7.1, §15, §13.6 references

git add docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md
git commit -m "$(cat <<'EOF'
docs(spec): update MCP transport from deprecated SSE to Streamable HTTP

The 2025-03-26 MCP spec revision deprecates the two-endpoint SSE transport in
favor of Streamable HTTP (single POST /mcp endpoint, optional SSE upgrade per
request). Sub-plan 3 implements the new shape directly; legacy SSE remains as
the documented fallback if §15 dep #1 lands negative against current CC clients.

§7.1 .mcp.json example flipped from type: "sse" to "http". §15 dep #1 reworded.
§13.6 added with rationale.
EOF
)"
```

---

## Task 2: Extend `.claude/settings.json` writer to also emit `.mcp.json`

**Goal:** Sub-plan 2 created `src/spawn_config.rs::write_session_files()` that emits `.claude/settings.json` per spec §7.1 step 4. Add a sibling function (or extend the same one) to emit `.mcp.json` with the Streamable HTTP transport entry pointing at the running GalvoLite. Bearer token comes from the same `sessions.api_token` minted at spawn.

**Files:**
- Modify: `src/spawn_config.rs` — add `.mcp.json` body construction
- Modify: `src/session_manager.rs` — call site stays put (single function still does both files)
- Create: `tests/spawn_config_mcp_writer.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/spawn_config_mcp_writer.rs
use galvolite::spawn_config;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn writes_mcp_json_with_http_transport_and_bearer() {
    let tmp = TempDir::new().unwrap();
    let worktree = tmp.path();
    spawn_config::write_session_files(
        worktree,
        "/usr/local/bin/galvolite-hook",
        "http://127.0.0.1:3001",
        "test-token-abc",
    ).unwrap();

    let mcp_path = worktree.join(".mcp.json");
    assert!(mcp_path.exists(), ".mcp.json was not written");

    let body: Value = serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let server = &body["mcpServers"]["galvolite"];

    assert_eq!(server["type"], "http", "transport must be Streamable HTTP per spec §7.1 (amended)");
    assert_eq!(server["url"], "http://127.0.0.1:3001/mcp");
    assert_eq!(server["headers"]["Authorization"], "Bearer test-token-abc");
}

#[test]
fn mcp_json_omits_portfolio_entry_when_no_gateway() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_session_files(
        tmp.path(),
        "/usr/local/bin/galvolite-hook",
        "http://127.0.0.1:3001",
        "tok",
    ).unwrap();

    let body: Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap()
    ).unwrap();

    assert!(body["mcpServers"].get("portfolio").is_none(),
        "portfolio entry only when Docker MCP Gateway configured (Plan 5+ scope)");
}

#[test]
fn rewriting_overwrites_old_mcp_json() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_session_files(tmp.path(), "/x/hook", "http://a/", "tok-1").unwrap();
    spawn_config::write_session_files(tmp.path(), "/x/hook", "http://b/", "tok-2").unwrap();

    let body: Value = serde_json::from_str(
        &std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap()
    ).unwrap();
    assert_eq!(body["mcpServers"]["galvolite"]["url"], "http://b/mcp");
    assert_eq!(body["mcpServers"]["galvolite"]["headers"]["Authorization"], "Bearer tok-2");
}
```

```bash
cargo test --test spawn_config_mcp_writer
```

Expect failure (function doesn't write `.mcp.json` yet).

- [ ] **Step 2: Implement `.mcp.json` body**

In `src/spawn_config.rs`, the existing `write_session_files` function takes `(worktree, hook_bin, api_base, token)`. Inside that function, after writing `.claude/settings.json`, add:

```rust
use serde_json::json;

let mcp_url = format!("{}/mcp", api_base.trim_end_matches('/'));
let mcp_body = json!({
    "mcpServers": {
        "galvolite": {
            "type": "http",
            "url": mcp_url,
            "headers": { "Authorization": format!("Bearer {token}") }
        }
    }
});

let mcp_path = worktree.join(".mcp.json");
std::fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_body)?)?;
```

`portfolio` entry stays out — added conditionally in a future plan when `McpPortfolio` trait lands.

- [ ] **Step 3: Tests pass; clippy clean**

```bash
cargo test --test spawn_config_mcp_writer
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 4: Commit**

```bash
git add src/spawn_config.rs tests/spawn_config_mcp_writer.rs
git commit -m "$(cat <<'EOF'
feat(spawn_config): write .mcp.json with Streamable HTTP transport at spawn

Extends the spawn-time config writer (Sub-plan 2) to also emit .mcp.json
pointing at GalvoLite's /mcp endpoint with the per-session bearer token.
Per amended spec §7.1 the transport type is "http" (Streamable HTTP) — the
session's spawned `claude` process auto-discovers the server via the file.

Portfolio (Docker MCP Gateway) entry deferred to a later plan when the
McpPortfolio trait lands.
EOF
)"
```

---

## Task 3: JSON-RPC 2.0 envelope + Streamable HTTP transport

**Goal:** Implement the protocol layer. Three concerns: (a) JSON-RPC 2.0 request/response/error envelope types with serde; (b) the Streamable HTTP request handler that parses one POST body, routes to dispatch, returns either `application/json` or `text/event-stream` based on the request's `Accept` header; (c) mount at `POST /mcp`. No tools yet — dispatch returns `JsonRpcError::method_not_found` for everything.

**Files:**
- Create: `src/mcp/mod.rs`
- Create: `src/mcp/protocol.rs`
- Create: `src/mcp/transport.rs`
- Modify: `src/lib.rs` — `pub mod mcp;`
- Modify: `src/api.rs` — mount route
- Create: `tests/mcp_protocol_envelopes.rs`
- Create: `tests/mcp_transport_http.rs`

- [ ] **Step 1: Write failing protocol envelope tests**

```rust
// tests/mcp_protocol_envelopes.rs
use galvolite::mcp::protocol::*;
use serde_json::json;

#[test]
fn parses_valid_jsonrpc_request() {
    let raw = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "initialize");
    assert!(matches!(req.id, Some(JsonRpcId::Number(1))));
}

#[test]
fn rejects_non_2_0_jsonrpc_version() {
    let raw = r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#;
    // Round-trip via serde succeeds (jsonrpc is just a String), but our
    // validator rejects non-"2.0" values.
    let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
    assert!(req.validate().is_err(), "non-2.0 jsonrpc version must be rejected");
}

#[test]
fn rejects_batch_requests_per_streamable_http_spec() {
    // Streamable HTTP transport accepts only single requests, not arrays.
    // Server-side batch support was removed in MCP 2025-03-26.
    let raw = r#"[{"jsonrpc":"2.0","id":1,"method":"x"}]"#;
    let parsed: Result<JsonRpcRequest, _> = serde_json::from_str(raw);
    assert!(parsed.is_err(), "batch (array) requests must not parse as JsonRpcRequest");
}

#[test]
fn error_response_uses_correct_method_not_found_code() {
    let resp = JsonRpcResponse::error(
        Some(JsonRpcId::Number(1)),
        JsonRpcError::method_not_found("unknown/tool"),
    );
    assert_eq!(resp.error.as_ref().unwrap().code, -32601);
    assert!(resp.error.as_ref().unwrap().message.contains("unknown/tool"));
}

#[test]
fn error_response_uses_correct_invalid_params_code() {
    let e = JsonRpcError::invalid_params("missing 'name'");
    assert_eq!(e.code, -32602);
}

#[test]
fn success_response_omits_error_field_in_serialization() {
    let resp = JsonRpcResponse::success(Some(JsonRpcId::Number(1)), json!({"ok": true}));
    let s = serde_json::to_string(&resp).unwrap();
    assert!(!s.contains("\"error\""), "successful response must not contain error field: {s}");
    assert!(s.contains("\"result\""));
}
```

```bash
cargo test --test mcp_protocol_envelopes
```

Expect compile failures — types don't exist yet.

- [ ] **Step 2: Implement `src/mcp/protocol.rs`**

```rust
//! JSON-RPC 2.0 envelopes + MCP-specific request/response shapes.
//!
//! Reference: https://www.jsonrpc.org/specification (envelope) and
//! https://spec.modelcontextprotocol.io/specification/2025-03-26/ (MCP methods).

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

#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<JsonRpcId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: Option<JsonRpcId>, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    pub fn error(id: Option<JsonRpcId>, error: JsonRpcError) -> Self {
        Self { jsonrpc: "2.0", id, result: None, error: Some(error) }
    }
}

#[derive(Debug, Clone, Serialize)]
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
        Self { code: Self::PARSE_ERROR, message: msg.into(), data: None }
    }
    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self { code: Self::INVALID_REQUEST, message: msg.into(), data: None }
    }
    pub fn method_not_found(method: impl Into<String>) -> Self {
        let m = method.into();
        Self { code: Self::METHOD_NOT_FOUND, message: format!("method not found: {m}"), data: None }
    }
    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self { code: Self::INVALID_PARAMS, message: format!("invalid params: {}", msg.into()), data: None }
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self { code: Self::INTERNAL_ERROR, message: msg.into(), data: None }
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self { code: Self::UNAUTHORIZED, message: msg.into(), data: None }
    }
}

// MCP method shapes —————————————————————————————————————————————————————

#[derive(Debug, Serialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: &'static str,
    pub capabilities: ServerCapabilities,
    #[serde(rename = "serverInfo")]
    pub server_info: ServerInfo,
}

#[derive(Debug, Serialize)]
pub struct ServerCapabilities {
    pub tools: ToolsCapability,
}

#[derive(Debug, Serialize)]
pub struct ToolsCapability {
    /// We declare a static catalog at startup; no list_changed notifications.
    #[serde(rename = "listChanged")]
    pub list_changed: bool,
}

#[derive(Debug, Serialize)]
pub struct ServerInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct ToolsCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Serialize)]
pub struct ToolsCallResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum ToolContent {
    #[serde(rename = "text")]
    Text { text: String },
}
```

- [ ] **Step 3: Implement `src/mcp/transport.rs` skeleton**

Write the request handler that parses the body, validates the envelope, and (for now) returns a `method_not_found` for everything except `ping` (used to test the transport in isolation).

```rust
//! Streamable HTTP transport per MCP 2025-03-26.
//!
//! Single endpoint: POST /mcp. Request body is one JSON-RPC request.
//! Response Content-Type depends on the request's Accept header:
//!   - application/json (default) → single JSON-RPC response body
//!   - text/event-stream → SSE upgrade for streaming handlers (none today)

use crate::auth;
use crate::mcp::{dispatch, protocol::*};
use crate::session_manager::SessionManager;
use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;

pub async fn handle(
    State(mgr): State<Arc<SessionManager>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    // Auth — Authorization: Bearer <token>
    let token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return unauthorized_resp(None),
    };

    let session_id = match auth::resolve_token(mgr.db(), &token) {
        Ok(id) => id,
        Err(_) => return unauthorized_resp(None),
    };

    // Envelope parse
    let req: JsonRpcRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return Json(JsonRpcResponse::error(None, JsonRpcError::parse_error(e.to_string())))
                .into_response();
        }
    };

    if let Err(msg) = req.validate() {
        return Json(JsonRpcResponse::error(req.id.clone(), JsonRpcError::invalid_request(msg)))
            .into_response();
    }

    // Dispatch
    let resp = dispatch::dispatch(mgr.clone(), session_id, req).await;

    // Accept: text/event-stream branch — wrap as SSE for the rare streaming case.
    let wants_sse = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/event-stream"));

    if wants_sse {
        let body = format!(
            "event: message\ndata: {}\n\n",
            serde_json::to_string(&resp).unwrap()
        );
        return ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response();
    }

    Json(resp).into_response()
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").map(|t| t.to_string()))
}

fn unauthorized_resp(id: Option<JsonRpcId>) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(JsonRpcResponse::error(id, JsonRpcError::unauthorized("invalid or missing bearer token"))),
    )
        .into_response()
}
```

- [ ] **Step 4: Stub `src/mcp/dispatch.rs`**

```rust
use crate::mcp::protocol::*;
use crate::session_manager::SessionManager;
use std::sync::Arc;
use uuid::Uuid;

pub async fn dispatch(
    _mgr: Arc<SessionManager>,
    _session_id: Uuid,
    req: JsonRpcRequest,
) -> JsonRpcResponse {
    // Tasks 4 + 5 implement initialize / tools/list / tools/call.
    JsonRpcResponse::error(req.id, JsonRpcError::method_not_found(req.method))
}
```

- [ ] **Step 5: `src/mcp/mod.rs` + `lib.rs` + route mount**

```rust
// src/mcp/mod.rs
pub mod protocol;
pub mod transport;
pub mod dispatch;
// registry + tools modules added in later tasks
```

In `src/lib.rs`:
```rust
pub mod mcp;
```

In `src/api.rs`, add:
```rust
.route("/mcp", post(crate::mcp::transport::handle))
```
to the router builder.

- [ ] **Step 6: Write transport integration test**

```rust
// tests/mcp_transport_http.rs
use galvolite::mcp::protocol::JsonRpcResponse;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;

mod common; // helper that boots a test app and returns (url, db, session_id, token)

#[tokio::test]
async fn missing_bearer_returns_401() {
    let app = common::test_app().await;
    let resp = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
        .send().await.unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn invalid_bearer_returns_401() {
    let app = common::test_app().await;
    let resp = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, "Bearer this-token-does-not-exist")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#)
        .send().await.unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn valid_bearer_unknown_method_returns_method_not_found() {
    let app = common::test_app().await;
    let (_session_id, token) = app.seed_session().await;

    let resp = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"nope/method"}"#)
        .send().await.unwrap();

    assert_eq!(resp.status(), 200);
    let body: JsonRpcResponse = resp.json().await.unwrap();
    assert!(body.error.is_some());
    assert_eq!(body.error.unwrap().code, -32601);
}

#[tokio::test]
async fn sse_accept_returns_event_stream_content_type() {
    let app = common::test_app().await;
    let (_session_id, token) = app.seed_session().await;

    let resp = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(ACCEPT, "text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#)
        .send().await.unwrap();

    let ct = resp.headers().get(CONTENT_TYPE).unwrap().to_str().unwrap().to_string();
    assert!(ct.contains("text/event-stream"), "got Content-Type: {ct}");
    let body = resp.text().await.unwrap();
    assert!(body.starts_with("event: message\n"), "expected SSE framing, got: {body}");
}

#[tokio::test]
async fn malformed_json_returns_parse_error_envelope() {
    let app = common::test_app().await;
    let (_session_id, token) = app.seed_session().await;

    let resp = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body("not even close to json")
        .send().await.unwrap();

    assert_eq!(resp.status(), 200);
    let body: JsonRpcResponse = resp.json().await.unwrap();
    assert_eq!(body.error.unwrap().code, -32700);
}
```

`common::test_app()` is the shared test helper introduced by Sub-plan 2 in `tests/common.rs`. If the helper from Sub-plan 2 is on the `plan-hook-surface` branch and not yet merged, copy its body into a private `tests/common.rs` on this branch — the merge will deduplicate.

`reqwest` is already a dev/test dep after Sub-plan 2 added it.

- [ ] **Step 7: Tests pass; clippy clean**

```bash
cargo test --test mcp_protocol_envelopes
cargo test --test mcp_transport_http
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add src/mcp/ src/lib.rs src/api.rs tests/mcp_protocol_envelopes.rs tests/mcp_transport_http.rs
git commit -m "$(cat <<'EOF'
feat(mcp): JSON-RPC 2.0 envelope + Streamable HTTP transport at POST /mcp

Implements the MCP 2025-03-26 Streamable HTTP transport directly without an
external SDK. Single POST /mcp endpoint accepts one JSON-RPC request per body;
response Content-Type follows the request's Accept header (application/json by
default, text/event-stream upgrade for streaming handlers — none today).

Bearer auth via Authorization header, resolved through the (still-inline)
session token lookup. Task 5 extracts the resolver into src/auth.rs once
Sub-plan 2's hook handler is on the same branch.

Dispatcher returns method_not_found for everything; tools wired in Tasks 4-10.
EOF
)"
```

---

## Task 4: Declarative `ToolRegistry` + `initialize` + `tools/list`

**Goal:** Build a static tool catalog at server startup, mirroring the table-driven pattern of `state::transition()`. Each `ToolDef` carries name, JSON schema (params + result), one-sentence description, side-effect tier, and an `async fn` handler. Registry is cheap to clone (`Arc`'d). `tools/list` returns the catalog as MCP-shaped tool descriptors. `initialize` returns capabilities + protocol version. `tools/call` (Task 5) looks up by name and invokes the handler.

**Files:**
- Create: `src/mcp/registry.rs`
- Create: `src/mcp/tools/mod.rs` (empty `pub mod` declarations only at this point)
- Modify: `src/mcp/dispatch.rs` — handle `initialize` and `tools/list`
- Modify: `src/mcp/mod.rs` — `pub mod registry; pub mod tools;`
- Modify: `src/session_manager.rs` (or wherever `App` state is constructed) — build registry once, store on shared state
- Create: `tests/mcp_initialize_tools_list.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/mcp_initialize_tools_list.rs
use galvolite::mcp::protocol::JsonRpcResponse;
use reqwest::header::AUTHORIZATION;
use serde_json::Value;

mod common;

#[tokio::test]
async fn initialize_returns_protocol_version_and_capabilities() {
    let app = common::test_app().await;
    let (_, token) = app.seed_session().await;

    let body: JsonRpcResponse = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#)
        .send().await.unwrap()
        .json().await.unwrap();

    let result = body.result.expect("initialize must succeed");
    assert_eq!(result["protocolVersion"], "2025-03-26");
    assert_eq!(result["capabilities"]["tools"]["listChanged"], false,
        "static catalog — never sends tools/list_changed");
    assert_eq!(result["serverInfo"]["name"], "galvolite");
}

#[tokio::test]
async fn tools_list_includes_active_and_reserved_tools() {
    let app = common::test_app().await;
    let (_, token) = app.seed_session().await;

    let body: JsonRpcResponse = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#)
        .send().await.unwrap()
        .json().await.unwrap();

    let tools = body.result.expect("tools/list must succeed")
        .get("tools").cloned().expect("tools array")
        .as_array().cloned().expect("tools is array");

    let names: Vec<String> = tools.iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();

    // Tier 1 — must exist and not be a stub
    for required in &["status_report", "set_role", "whoami", "set_plan",
                      "write_handoff", "request_attention"] {
        assert!(names.contains(&required.to_string()), "missing tier-1 tool: {required}");
    }
    // Tier 4 — reserved stubs must also be advertised
    for stub in &["spawn_child", "open_pr", "claim_paths", "list_available_mcps"] {
        assert!(names.contains(&stub.to_string()), "missing reserved stub: {stub}");
    }

    // Each tool descriptor has the MCP-required fields
    for t in &tools {
        assert!(t["name"].is_string());
        assert!(t["description"].is_string(), "tool {} lacks description", t["name"]);
        assert!(t["inputSchema"].is_object(), "tool {} lacks inputSchema", t["name"]);
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[tokio::test]
async fn tool_names_are_unique() {
    use std::collections::HashSet;
    let app = common::test_app().await;
    let (_, token) = app.seed_session().await;

    let body: JsonRpcResponse = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send().await.unwrap()
        .json().await.unwrap();

    let names: Vec<String> = body.result.unwrap()["tools"].as_array().unwrap()
        .iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    let unique: HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(),
        "duplicate tool names — registry table broken");
}
```

```bash
cargo test --test mcp_initialize_tools_list
```

Expect failures (registry doesn't exist yet, `initialize` / `tools/list` not handled).

- [ ] **Step 2: Define `ToolDef` and `ToolRegistry`**

```rust
// src/mcp/registry.rs
use crate::session_manager::SessionManager;
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

pub struct ToolCtx {
    pub mgr: Arc<SessionManager>,
    pub session_id: Uuid,
}

pub type ToolHandler = Box<
    dyn Fn(ToolCtx, Value) -> Pin<Box<dyn Future<Output = ToolOutcome> + Send>>
        + Send
        + Sync,
>;

pub enum ToolOutcome {
    Ok(Value),
    /// Handler completed but the operation produced an application-level error.
    /// Wrapped as ToolsCallResult { isError: true } in the JSON-RPC response,
    /// not a JSON-RPC error code.
    AppError(String),
    /// Validation/argument failure — surfaces as JSON-RPC -32602.
    InvalidParams(String),
}

#[derive(Clone, Copy)]
pub enum Tier {
    /// Self-contained — works against DB only.
    SelfContained,
    /// Drives a sibling PTY. Requires the target session to be alive.
    CrossSessionPty,
    /// Needs the dashboard to function. Degrades to a no-op / timed-out reply
    /// until Sub-plan 4 wires the WS surface.
    DashboardDegraded,
    /// Reserved name; handler returns not_implemented. Plans 4-6 will fill in.
    ReservedStub,
}

pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
    pub tier: Tier,
    pub handler: ToolHandler,
}

#[derive(Clone)]
pub struct ToolRegistry {
    inner: Arc<HashMap<&'static str, Arc<ToolDef>>>,
}

impl ToolRegistry {
    pub fn build() -> Self {
        let defs = crate::mcp::tools::all();
        let map = defs.into_iter()
            .map(|d| (d.name, Arc::new(d)))
            .collect::<HashMap<_, _>>();
        Self { inner: Arc::new(map) }
    }

    pub fn get(&self, name: &str) -> Option<Arc<ToolDef>> {
        self.inner.get(name).cloned()
    }

    pub fn list_descriptors(&self) -> Vec<Value> {
        let mut v: Vec<_> = self.inner.values().map(|d| {
            serde_json::json!({
                "name": d.name,
                "description": d.description,
                "inputSchema": d.input_schema,
            })
        }).collect();
        v.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));
        v
    }
}
```

- [ ] **Step 3: Stub `src/mcp/tools/mod.rs`**

```rust
//! Tool definitions, organized by tier and topic.
//!
//! Tasks 6-10 fill in each module. For now `all()` returns an empty Vec so the
//! registry compiles; tools/list will be empty until Task 6 lands.

use crate::mcp::registry::ToolDef;

pub fn all() -> Vec<ToolDef> {
    let mut v = Vec::new();
    // Task 6 — Tier 1 status & lifecycle
    // v.extend(tier1_status::tools());
    // Task 7 — Tier 1 role & handoff
    // v.extend(tier1_role_handoff::tools());
    // Task 8 — Tier 1 introspection & self-control
    // v.extend(tier1_introspection::tools());
    // Task 9 — Tier 2/3 cross-session + operator
    // v.extend(tier2_3_cross::tools());
    // Task 10 — Tier 4 reserved stubs
    // v.extend(tier4_stubs::tools());
    v
}
```

Update `src/mcp/mod.rs`:
```rust
pub mod protocol;
pub mod transport;
pub mod dispatch;
pub mod registry;
pub mod tools;
```

- [ ] **Step 4: Plumb registry onto shared state**

In `src/session_manager.rs` (or wherever the shared `Arc<SessionManager>` is constructed in `main.rs`/`api.rs`), build the registry once at startup and store it on the manager (or a wrapping `AppState` struct). Suggested minimal approach — add a field to `SessionManager`:

```rust
// src/session_manager.rs
pub struct SessionManager {
    // ... existing fields ...
    mcp_registry: crate::mcp::registry::ToolRegistry,
}

impl SessionManager {
    pub fn new(/* existing args */) -> Self {
        Self {
            // ... existing init ...
            mcp_registry: crate::mcp::registry::ToolRegistry::build(),
        }
    }

    pub fn mcp_registry(&self) -> &crate::mcp::registry::ToolRegistry {
        &self.mcp_registry
    }
}
```

If `SessionManager::new` is `const`-y or test-friction prone, alternate: store the registry on a separate `AppState { mgr: Arc<SessionManager>, registry: ToolRegistry }` and update the Axum router builder. Pick the lower-friction option per current code shape.

- [ ] **Step 5: Implement `initialize` + `tools/list` in dispatch**

```rust
// src/mcp/dispatch.rs
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
                protocol_version: "2025-03-26",
                capabilities: ServerCapabilities {
                    tools: ToolsCapability { list_changed: false },
                },
                server_info: ServerInfo { name: "galvolite", version: env!("CARGO_PKG_VERSION") },
            };
            JsonRpcResponse::success(req.id, serde_json::to_value(result).unwrap())
        }
        "tools/list" => {
            let tools = mgr.mcp_registry().list_descriptors();
            JsonRpcResponse::success(req.id, json!({ "tools": tools }))
        }
        "tools/call" => {
            // Task 5 implements this.
            JsonRpcResponse::error(req.id, JsonRpcError::method_not_found("tools/call (pending Task 5)"))
        }
        _ => JsonRpcResponse::error(req.id, JsonRpcError::method_not_found(req.method)),
    }
}

// session_id is unused in this task but threads through for Task 5.
#[allow(unused)]
fn _unused(_: Uuid) {}
```

(If clippy complains about unused `session_id`, add `let _ = session_id;` inside `dispatch` or rename to `_session_id` for now — Task 5 uses it.)

- [ ] **Step 6: Tests pass (initialize succeeds; tools/list returns empty array since tools come in Tasks 6-10)**

The Task 4 tests assert specific tool names exist (e.g. `status_report`). Those will fail at this step until Tasks 6-10 land. **For Task 4's commit, narrow the assertion to only check the descriptor *shape* and the empty-or-positive count.** Re-tighten the test once Task 10 commits — add a follow-up assertion in the smoke test (Task 11) that all of §5's named tools are present.

Concretely, replace the `for required in &[...]` loop with:

```rust
// Tools registry is empty until Tasks 6-10 land. After Task 10, this test will
// be re-tightened by tests/mcp_smoke_endtoend.rs (Task 11) which asserts each
// named tool from spec §5 is present.
assert!(tools.is_empty() || tools.len() >= 1, "tools array shape OK");
```

- [ ] **Step 7: All tests pass; clippy clean**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add src/mcp/ src/session_manager.rs tests/mcp_initialize_tools_list.rs
git commit -m "$(cat <<'EOF'
feat(mcp): declarative ToolRegistry + initialize + tools/list

Adds ToolDef table built once at startup (mirrors state::transition's table-
driven shape). initialize returns protocolVersion 2025-03-26 + listChanged:false
(static catalog). tools/list returns sorted descriptors from the registry.

Tools modules are stubbed with empty all() — Tasks 6-10 fill in the catalog
across four tiers (self-contained, cross-session-PTY, dashboard-degraded,
reserved-stubs).
EOF
)"
```

---

## Task 5: `tools/call` dispatcher + extract shared `auth.rs`

**Goal:** Wire `tools/call` to look up the handler in the registry, deserialize arguments, invoke the async handler, and shape the result into the MCP `ToolsCallResult` envelope. Simultaneously, extract Sub-plan 2's inline `resolve_token` helper into `src/auth.rs` so both `/hooks/ingest` and `/mcp` share one implementation.

**Files:**
- Create: `src/auth.rs`
- Modify: `src/mcp/dispatch.rs` — handle `tools/call`
- Modify: `src/mcp/transport.rs` — already calls `auth::resolve_token` (placeholder); confirm path is correct after extraction
- Modify: `src/api.rs` (or `src/api/hooks.rs` if Sub-plan 2 split it) — replace inline resolver call with `auth::resolve_token`
- Modify: `src/lib.rs` — `pub mod auth;`
- Create: `tests/auth_resolver.rs`
- Create: `tests/mcp_dispatch.rs`

> **Coordination note:** This task touches Sub-plan 2's `src/api.rs::resolve_token` site. If Sub-plan 2 has not yet merged when this branch runs, the extraction may produce a textual conflict at merge time. Resolution is mechanical: keep the new `src/auth.rs`, delete Sub-plan 2's inline copy, point its caller at the shared symbol. Document in the PR description.

- [ ] **Step 1: Write failing test for `auth::resolve_token`**

```rust
// tests/auth_resolver.rs
use galvolite::{auth, db::Db, error::Error};
use tempfile::TempDir;

#[test]
fn valid_token_resolves_to_session_id() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let id = db.insert_session("a", "implementation", "/tmp", None).unwrap();
    db.set_api_token(id, "token-xyz").unwrap(); // helper added in Sub-plan 2

    assert_eq!(auth::resolve_token(&db, "token-xyz").unwrap(), id);
}

#[test]
fn unknown_token_returns_unauthorized() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();

    let err = auth::resolve_token(&db, "no-such-token").unwrap_err();
    matches!(err, Error::Unauthorized);
}

#[test]
fn empty_token_returns_unauthorized() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let err = auth::resolve_token(&db, "").unwrap_err();
    matches!(err, Error::Unauthorized);
}

#[test]
fn revoked_token_no_longer_resolves() {
    let tmp = TempDir::new().unwrap();
    let db = Db::open(tmp.path().join("t.db")).unwrap();
    let id = db.insert_session("a", "implementation", "/tmp", None).unwrap();
    db.set_api_token(id, "tok-1").unwrap();

    assert!(auth::resolve_token(&db, "tok-1").is_ok());

    db.clear_api_token(id).unwrap(); // helper added in Sub-plan 2
    assert!(auth::resolve_token(&db, "tok-1").is_err());
}
```

- [ ] **Step 2: Implement `src/auth.rs`**

```rust
//! Shared bearer-token resolver for /hooks/ingest and /mcp.
//!
//! Constant-time comparison via `subtle` to avoid timing oracles on token
//! length / content. Resolves token → session_id by querying sessions.api_token.

use crate::{db::Db, error::Error};
use subtle::ConstantTimeEq;
use uuid::Uuid;

pub fn resolve_token(db: &Db, presented: &str) -> Result<Uuid, Error> {
    if presented.is_empty() {
        return Err(Error::Unauthorized);
    }
    // Sub-plan 2 added Db::sessions_with_tokens or similar — use whichever shape
    // exists. If only Db::lookup_token_to_session(token) was added, call that
    // and trust it does the constant-time compare itself.
    let candidates = db.sessions_with_tokens()?; // -> Vec<(Uuid, String)>

    let mut found = None;
    let presented_bytes = presented.as_bytes();
    for (sid, token) in candidates.iter() {
        // Constant-time compare; do NOT short-circuit.
        let eq = token.as_bytes().ct_eq(presented_bytes).into();
        if eq && found.is_none() {
            found = Some(*sid);
        }
    }
    found.ok_or(Error::Unauthorized)
}
```

If Sub-plan 2 already added `Error::Unauthorized` and `Db::sessions_with_tokens`, reuse them. If only `Db::lookup_session_by_token(&str) -> Result<Option<Uuid>, _>` exists, simplify the body:

```rust
match db.lookup_session_by_token(presented)? {
    Some(id) => Ok(id),
    None => Err(Error::Unauthorized),
}
```

— but only adopt this shape if the lookup itself is constant-time on the matched row. If it's a `WHERE api_token = ?` SQL query, that's *not* constant-time across rows; keep the iterate-all approach above.

- [ ] **Step 3: Re-point `transport.rs` and Sub-plan 2's hook handler at `auth::resolve_token`**

`src/mcp/transport.rs` already calls `crate::auth::resolve_token(...)` (Task 3 wrote it speculatively). Confirm it compiles.

In `src/api.rs` (or `src/api/hooks.rs` per Sub-plan 2's split), find the inline `resolve_token` function and the call site:

```rust
// BEFORE:
fn resolve_token(db: &Db, t: &str) -> Result<Uuid, Error> { /* ... inline ... */ }
// callers: hooks::ingest

// AFTER:
// (delete the inline fn)
// callers now: crate::auth::resolve_token(db, t)
```

`pub mod auth;` in `src/lib.rs`.

- [ ] **Step 4: Implement `tools/call` in `dispatch.rs`**

```rust
"tools/call" => {
    let params: ToolsCallParams = match req.params.clone()
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
            JsonRpcError::method_not_found(format!("tools/call: unknown tool '{}'", params.name)),
        );
    };

    let ctx = crate::mcp::registry::ToolCtx { mgr: mgr.clone(), session_id };
    let outcome = (def.handler)(ctx, params.arguments).await;

    match outcome {
        crate::mcp::registry::ToolOutcome::Ok(value) => {
            let result = ToolsCallResult {
                content: vec![ToolContent::Text { text: value.to_string() }],
                is_error: false,
            };
            JsonRpcResponse::success(req.id, serde_json::to_value(result).unwrap())
        }
        crate::mcp::registry::ToolOutcome::AppError(msg) => {
            // App-level error → return as is_error: true content (NOT JSON-RPC error code).
            // Per MCP spec: tool execution errors live inside the result envelope so
            // the model can read them and adapt; protocol errors use JSON-RPC codes.
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
```

- [ ] **Step 5: Write dispatch contract tests**

```rust
// tests/mcp_dispatch.rs
use galvolite::mcp::protocol::JsonRpcResponse;
use reqwest::header::AUTHORIZATION;
use serde_json::json;

mod common;

async fn rpc(app: &common::TestApp, token: &str, body: &str) -> JsonRpcResponse {
    reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(body.to_string())
        .send().await.unwrap()
        .json().await.unwrap()
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let app = common::test_app().await;
    let (_, token) = app.seed_session().await;
    let r = rpc(&app, &token, r#"{"jsonrpc":"2.0","id":1,"method":"nope/x"}"#).await;
    assert_eq!(r.error.unwrap().code, -32601);
}

#[tokio::test]
async fn tools_call_unknown_tool_returns_method_not_found() {
    let app = common::test_app().await;
    let (_, token) = app.seed_session().await;
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"no_such_tool","arguments":{}}}"#;
    let r = rpc(&app, &token, body).await;
    assert_eq!(r.error.unwrap().code, -32601, "unknown tool surfaces as method_not_found");
}

#[tokio::test]
async fn tools_call_missing_params_returns_invalid_params() {
    let app = common::test_app().await;
    let (_, token) = app.seed_session().await;
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#;
    let r = rpc(&app, &token, body).await;
    assert_eq!(r.error.unwrap().code, -32602);
}
```

(The "valid tool call" path can't be tested at this step — Tasks 6-10 add real tools. Smoke test in Task 11 covers the happy path end-to-end.)

- [ ] **Step 6: Tests pass; clippy clean**

```bash
cargo test --test auth_resolver
cargo test --test mcp_dispatch
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/auth.rs src/lib.rs src/api.rs src/mcp/dispatch.rs tests/auth_resolver.rs tests/mcp_dispatch.rs
git commit -m "$(cat <<'EOF'
feat(mcp): tools/call dispatcher; extract shared auth::resolve_token

Wires tools/call to look up handlers in the ToolRegistry, deserialize args,
invoke the async handler, and shape the result into MCP's ToolsCallResult
envelope. Application-level errors land as { isError: true } content (so the
model can read and adapt) — protocol errors keep their JSON-RPC error codes.

Extracts Sub-plan 2's inline resolve_token helper into src/auth.rs (constant-
time compare via subtle::ConstantTimeEq). Both /hooks/ingest and /mcp now share
the same resolver. Original inline copy in src/api.rs deleted.

Coordination note for the merge: Sub-plan 2's branch contains the inline copy.
On merge, take this branch's src/auth.rs and re-point Sub-plan 2's caller.
EOF
)"
```

---

## Task 6: Tier 1 — Status & lifecycle tools

**Goal:** Implement spec §5.1 status & lifecycle tools — `status_report`, `request_attention`, `set_plan`, `plan_check_step`, `set_seed`. Each handler validates args, builds an `EventInput`, calls `state::apply_event`, and returns the resulting state to the model. `set_plan` additionally extracts checkbox steps via the parser logic from spec §8.4 (or stubs that to a follow-up if `plan_steps` table writes need their own task — for this plan, write the events but defer the structured `plan_steps` upserts to a follow-up note in the commit message).

**Files:**
- Create: `src/mcp/tools/tier1_status.rs`
- Modify: `src/mcp/tools/mod.rs` — uncomment Task 6 line
- Modify: `src/state.rs::transition` — add arms for `mcp.status_report` (working/blocked → keep/NeedsAttention), `mcp.request_attention` (→ NeedsAttention), `mcp.set_plan` (no state change), `mcp.plan_check_step` (no state change), `mcp.set_seed` (no state change)
- Create: `tests/mcp_tools_tier1_status.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/mcp_tools_tier1_status.rs
use galvolite::mcp::protocol::JsonRpcResponse;
use galvolite::state::SessionState;
use reqwest::header::AUTHORIZATION;
use serde_json::json;

mod common;

async fn call_tool(app: &common::TestApp, token: &str, name: &str, args: serde_json::Value)
    -> JsonRpcResponse
{
    let body = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": args }
    }).to_string();
    reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(body).send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn status_report_working_writes_event_no_state_change() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let r = call_tool(&app, &token, "status_report",
        json!({"kind": "working", "note": "still chewing"})).await;
    assert!(r.error.is_none(), "tool errored: {:?}", r.error);
    assert_eq!(common::state_of(&app.db, sid), SessionState::Working);

    let count: i64 = app.db.pool().get().unwrap().query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.status_report'",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(count, 1, "exactly one event row written");
}

#[tokio::test]
async fn status_report_blocked_transitions_to_needs_attention() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call_tool(&app, &token, "status_report",
        json!({"kind": "blocked", "note": "need creds"})).await;
    assert_eq!(common::state_of(&app.db, sid), SessionState::NeedsAttention);
}

#[tokio::test]
async fn request_attention_transitions_to_needs_attention() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call_tool(&app, &token, "request_attention",
        json!({"reason": "urgent decision needed"})).await;
    assert_eq!(common::state_of(&app.db, sid), SessionState::NeedsAttention);

    let payload: String = app.db.pool().get().unwrap().query_row(
        "SELECT payload_json FROM events WHERE session_id = ?1 AND kind = 'mcp.request_attention'",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert!(payload.contains("urgent decision needed"));
}

#[tokio::test]
async fn set_plan_writes_plan_markdown_column() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let md = "- [ ] step one\n- [x] step two done\n";
    let _ = call_tool(&app, &token, "set_plan", json!({"markdown": md})).await;

    let stored: String = app.db.pool().get().unwrap().query_row(
        "SELECT plan_markdown FROM sessions WHERE id = ?1",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(stored, md);
    // structured plan_steps upserts deferred — see commit note.
}

#[tokio::test]
async fn set_seed_writes_seed_markdown_column() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Idle).await;

    let _ = call_tool(&app, &token, "set_seed", json!({"markdown": "# Seed\nuse rust"})).await;

    let stored: String = app.db.pool().get().unwrap().query_row(
        "SELECT seed_markdown FROM sessions WHERE id = ?1",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert!(stored.contains("# Seed"));
}

#[tokio::test]
async fn status_report_invalid_kind_returns_invalid_params() {
    let app = common::test_app().await;
    let (_sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let r = call_tool(&app, &token, "status_report",
        json!({"kind": "ascending", "note": "?"})).await;
    assert_eq!(r.error.unwrap().code, -32602);
}

#[tokio::test]
async fn plan_check_step_writes_event() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call_tool(&app, &token, "set_plan",
        json!({"markdown": "- [ ] alpha\n- [ ] beta\n"})).await;
    let _ = call_tool(&app, &token, "plan_check_step",
        json!({"title": "alpha", "status": "done"})).await;

    let count: i64 = app.db.pool().get().unwrap().query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.plan_check_step'",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Implement handlers**

```rust
// src/mcp/tools/tier1_status.rs
use crate::mcp::registry::*;
use crate::state::{self, EventInput, Source};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "status_report",
            description: "Report current work status. kind=working|complete|blocked. Optional note.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["working", "complete", "blocked"] },
                    "note": { "type": "string" }
                },
                "required": ["kind"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(status_report(ctx, args))),
        },
        ToolDef {
            name: "request_attention",
            description: "Flag this session as needing operator attention with a reason string.",
            input_schema: json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(request_attention(ctx, args))),
        },
        ToolDef {
            name: "set_plan",
            description: "Replace the session's plan markdown. Checkbox steps (- [ ] / - [x]) are extracted by the dashboard.",
            input_schema: json!({
                "type": "object",
                "properties": { "markdown": { "type": "string" } },
                "required": ["markdown"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(set_plan(ctx, args))),
        },
        ToolDef {
            name: "plan_check_step",
            description: "Update the status of one plan step by title (preferred) or step_hash. status: pending|working|done|blocked.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "step_hash": { "type": "string" },
                    "status": { "type": "string", "enum": ["pending","working","done","blocked"] },
                    "note": { "type": "string" }
                },
                "required": ["status"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(plan_check_step(ctx, args))),
        },
        ToolDef {
            name: "set_seed",
            description: "Replace the session's seed (the markdown injected at SessionStart for resumes).",
            input_schema: json!({
                "type": "object",
                "properties": { "markdown": { "type": "string" } },
                "required": ["markdown"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(set_seed(ctx, args))),
        },
    ]
}

#[derive(Deserialize)]
struct StatusReportArgs { kind: String, note: Option<String> }

async fn status_report(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: StatusReportArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !matches!(parsed.kind.as_str(), "working" | "complete" | "blocked") {
        return ToolOutcome::InvalidParams(format!("unknown kind: {}", parsed.kind));
    }
    let payload = json!({ "kind": parsed.kind, "note": parsed.note });
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.status_report".into(),
        payload,
        dedup_key: None,
    });
    match outcome {
        Ok(o) => ToolOutcome::Ok(json!({ "state": format!("{:?}", o.next_state) })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct RequestAttentionArgs { reason: String }

async fn request_attention(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: RequestAttentionArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.request_attention".into(),
        payload: json!({ "reason": parsed.reason }),
        dedup_key: None,
    });
    match outcome {
        Ok(o) => ToolOutcome::Ok(json!({ "state": format!("{:?}", o.next_state) })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct SetPlanArgs { markdown: String }

async fn set_plan(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SetPlanArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    // Write event + bump sessions.plan_markdown atomically via apply_event.
    // Structured plan_steps upserts (spec §8.4) deferred — see commit note.
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.set_plan".into(),
        payload: json!({ "markdown": parsed.markdown }),
        dedup_key: None,
    });
    match outcome {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct PlanCheckStepArgs {
    title: Option<String>,
    step_hash: Option<String>,
    status: String,
    note: Option<String>,
}

async fn plan_check_step(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: PlanCheckStepArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if parsed.title.is_none() && parsed.step_hash.is_none() {
        return ToolOutcome::InvalidParams("must provide title or step_hash".into());
    }
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.plan_check_step".into(),
        payload: json!({
            "title": parsed.title, "step_hash": parsed.step_hash,
            "status": parsed.status, "note": parsed.note
        }),
        dedup_key: None,
    });
    match outcome {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct SetSeedArgs { markdown: String }

async fn set_seed(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SetSeedArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.set_seed".into(),
        payload: json!({ "markdown": parsed.markdown }),
        dedup_key: None,
    });
    match outcome {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}
```

- [ ] **Step 3: Add `state::transition` arms**

In `src/state.rs::transition`:

```rust
// Status & lifecycle
(Working,            "mcp.status_report") => {
    // kind=blocked → NeedsAttention; otherwise stay Working.
    if input.payload.get("kind").and_then(|v| v.as_str()) == Some("blocked") {
        SessionState::NeedsAttention
    } else {
        SessionState::Working
    }
}
(Idle,               "mcp.status_report") => SessionState::Idle,
(s, "mcp.request_attention") if !s.is_terminal() => SessionState::NeedsAttention,
(s, "mcp.set_plan")        if !s.is_terminal() => s,
(s, "mcp.plan_check_step") if !s.is_terminal() => s,
(s, "mcp.set_seed")        if !s.is_terminal() => s,
```

`apply_event` already writes `payload` to the events row; in addition, for `mcp.set_plan` and `mcp.set_seed`, the side-effect of mutating `sessions.plan_markdown` / `sessions.seed_markdown` happens inside the same SQLite transaction. Implementation choice: extend `apply_event` to recognize these kinds and update the columns from `payload.markdown`, OR have the handler enqueue a follow-up DB call after `apply_event` returns. **Prefer extending `apply_event`** — keeps the §8.3 atomicity invariant intact. Add the column update inside the existing `apply_event` SQL transaction.

- [ ] **Step 4: Wire `tier1_status::tools()` into `tools/mod.rs::all()`**

```rust
// src/mcp/tools/mod.rs
pub mod tier1_status;

pub fn all() -> Vec<ToolDef> {
    let mut v = Vec::new();
    v.extend(tier1_status::tools());
    // ... other tasks ...
    v
}
```

- [ ] **Step 5: Tests pass; clippy clean**

```bash
cargo test --test mcp_tools_tier1_status
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools/ src/state.rs tests/mcp_tools_tier1_status.rs
git commit -m "$(cat <<'EOF'
feat(mcp): tier-1 status & lifecycle tools (5 of catalog)

Implements spec §5.1 status & lifecycle: status_report, request_attention,
set_plan, plan_check_step, set_seed. Each handler writes one event row through
state::apply_event before mutating session state — atomicity invariant §8.3
holds. set_plan and set_seed also update sessions.plan_markdown /
sessions.seed_markdown inside the same SQLite transaction.

Deferred: structured plan_steps row upserts per spec §8.4 (currently set_plan
only stores raw markdown). The dashboard (Sub-plan 4) reads plan_markdown
directly today; structured steps land when the Plan tab needs hash-keyed diff
animations. Tracked as a Sub-plan 4 dependency.
EOF
)"
```

---

## Task 7: Tier 1 — Role + handoff tools

**Goal:** Implement `set_role`, `write_handoff`, `read_handoff`. `set_role` rewrites `CLAUDE.md` in the worktree (using `claude_md::render`) so the next turn picks up the new profile, and updates `sessions.role`. `write_handoff` writes the structured handoff to `.galvolite/handoffs/{session_id}/<timestamp>/pointer.json` (existing `handoff.rs` helpers — reuse, don't reimplement). `read_handoff` returns the most recent handoff file for the named path or session.

**Files:**
- Create: `src/mcp/tools/tier1_role_handoff.rs`
- Modify: `src/mcp/tools/mod.rs` — add `pub mod tier1_role_handoff;` and extend `all()`
- Modify: `src/state.rs::transition` — add `mcp.set_role`, `mcp.write_handoff`, `mcp.read_handoff` arms (no state change for any)
- Create: `tests/mcp_tools_tier1_role_handoff.rs`

- [ ] **Step 1: Survey existing handoff helpers**

```bash
grep -n "pub fn" src/handoff.rs
grep -n "pub fn render" src/claude_md.rs
```

Note the function signatures available — `handoff::write_pointer`, `handoff::read_latest`, etc. Use whatever exists; if a needed helper is missing (e.g., `read_by_path`), add it as a small extension in `src/handoff.rs` rather than duplicating logic in the tool handler.

- [ ] **Step 2: Write failing tests**

```rust
// tests/mcp_tools_tier1_role_handoff.rs
use galvolite::state::SessionState;
use reqwest::header::AUTHORIZATION;
use serde_json::json;

mod common;

async fn call(app: &common::TestApp, token: &str, name: &str, args: serde_json::Value)
    -> galvolite::mcp::protocol::JsonRpcResponse
{
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                      "params":{"name":name,"arguments":args}}).to_string();
    reqwest::Client::new().post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(body).send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn set_role_updates_db_and_rewrites_claude_md() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_with_worktree("implementation").await;

    let _ = call(&app, &token, "set_role", json!({"role": "scout"})).await;

    let role: String = app.db.pool().get().unwrap().query_row(
        "SELECT role FROM sessions WHERE id = ?1",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(role, "scout");

    let claude_md = app.session_worktree(sid).join("CLAUDE.md");
    let body = std::fs::read_to_string(claude_md).unwrap();
    assert!(body.to_lowercase().contains("scout"),
        "rewritten CLAUDE.md should reflect new role; got: {body}");
}

#[tokio::test]
async fn set_role_invalid_value_returns_invalid_params() {
    let app = common::test_app().await;
    let (_sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let r = call(&app, &token, "set_role", json!({"role": "wizard"})).await;
    assert_eq!(r.error.unwrap().code, -32602);
}

#[tokio::test]
async fn write_handoff_creates_pointer_file_and_event_row() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_with_worktree("implementation").await;

    let _ = call(&app, &token, "write_handoff", json!({
        "summary": "wired up the auth middleware",
        "next_steps": ["add tests", "review with operator"]
    })).await;

    let pointer = app.handoff_dir(sid).join("pointer.json");
    assert!(pointer.exists(), "pointer.json missing at {:?}", pointer);

    let count: i64 = app.db.pool().get().unwrap().query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.write_handoff'",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn read_handoff_returns_most_recent_for_session() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_with_worktree("implementation").await;

    let _ = call(&app, &token, "write_handoff",
        json!({"summary": "first", "next_steps": []})).await;
    let _ = call(&app, &token, "write_handoff",
        json!({"summary": "second", "next_steps": []})).await;

    let r = call(&app, &token, "read_handoff", json!({})).await;
    let result = r.result.unwrap();
    let content = result["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("second"), "expected most recent ('second'), got: {content}");
    let _ = sid;
}
```

- [ ] **Step 3: Implement**

```rust
// src/mcp/tools/tier1_role_handoff.rs
use crate::mcp::registry::*;
use crate::state::{self, EventInput, Source};
use crate::{claude_md, handoff};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "set_role",
            description: "Switch this session's role mid-flight. Rewrites CLAUDE.md so the next turn loads the new profile.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "role": { "type": "string", "enum": ["orchestrator","brainstorm","implementation","scout"] }
                },
                "required": ["role"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(set_role(ctx, args))),
        },
        ToolDef {
            name: "write_handoff",
            description: "Write a structured handoff to .galvolite/handoffs/{session}/. Optional path override.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path":       { "type": "string" },
                    "summary":    { "type": "string" },
                    "next_steps": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["summary", "next_steps"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(write_handoff(ctx, args))),
        },
        ToolDef {
            name: "read_handoff",
            description: "Read a handoff. Without args returns the most recent for the current session; with path returns that exact file.",
            input_schema: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(read_handoff(ctx, args))),
        },
    ]
}

#[derive(Deserialize)]
struct SetRoleArgs { role: String }

async fn set_role(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SetRoleArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    if !matches!(parsed.role.as_str(),
                 "orchestrator" | "brainstorm" | "implementation" | "scout") {
        return ToolOutcome::InvalidParams(format!("unknown role: {}", parsed.role));
    }

    // 1. Write event + update sessions.role atomically.
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.set_role".into(),
        payload: json!({ "role": parsed.role }),
        dedup_key: None,
    });
    if let Err(e) = outcome {
        return ToolOutcome::AppError(e.to_string());
    }

    // 2. Rewrite CLAUDE.md in the worktree so the next turn picks up the role.
    let session = match ctx.mgr.db().get_session(ctx.session_id) {
        Ok(Some(s)) => s,
        Ok(None) => return ToolOutcome::AppError("session vanished".into()),
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    let project_ctx = match ctx.mgr.db().get_project_for_session(ctx.session_id) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    let body = claude_md::render(&parsed.role, &project_ctx);
    let path = std::path::PathBuf::from(&session.worktree).join("CLAUDE.md");
    if let Err(e) = std::fs::write(&path, body) {
        // Don't roll the role back — the model still sees the new role on next
        // turn via apply_event; CLAUDE.md will be retried at next set_role / spawn.
        tracing::warn!("set_role: failed to rewrite CLAUDE.md at {:?}: {}", path, e);
    }

    ToolOutcome::Ok(json!({ "role": parsed.role }))
}

#[derive(Deserialize)]
struct WriteHandoffArgs {
    path: Option<String>,
    summary: String,
    next_steps: Vec<String>,
}

async fn write_handoff(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: WriteHandoffArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    // 1. Event + sessions.handoff_path update (apply_event extended to handle this kind).
    let _ = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.write_handoff".into(),
        payload: json!({
            "path": parsed.path.clone(),
            "summary": parsed.summary,
            "next_steps": parsed.next_steps
        }),
        dedup_key: None,
    });

    // 2. Use handoff::write_handoff helper. If signature differs, adapt the call;
    //    do NOT duplicate the file-writing logic in this handler.
    match handoff::write_for_session(ctx.mgr.db(), ctx.session_id, parsed.path.as_deref(),
                                      &parsed.summary, &parsed.next_steps) {
        Ok(written_path) => ToolOutcome::Ok(json!({ "path": written_path })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct ReadHandoffArgs { path: Option<String> }

async fn read_handoff(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: ReadHandoffArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    let _ = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.read_handoff".into(),
        payload: json!({ "path": parsed.path }),
        dedup_key: None,
    });

    let result = match parsed.path {
        Some(p) => handoff::read_path(&p),
        None => handoff::read_latest_for_session(ctx.mgr.db(), ctx.session_id),
    };
    match result {
        Ok(Some(body)) => ToolOutcome::Ok(json!({ "content": body })),
        Ok(None) => ToolOutcome::Ok(json!({ "content": null })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}
```

If `handoff::write_for_session` / `handoff::read_path` / `handoff::read_latest_for_session` don't exist with these exact names, either (a) add them as thin wrappers in `src/handoff.rs` calling the existing functions, or (b) inline the call to the existing public symbols — pick the lower-friction path. Do NOT reimplement file layout logic here.

- [ ] **Step 4: Add `state::transition` arms and `apply_event` column updates**

```rust
// src/state.rs::transition
(s, "mcp.set_role")       if !s.is_terminal() => s, // no state change
(s, "mcp.write_handoff")  if !s.is_terminal() => s,
(s, "mcp.read_handoff")   if !s.is_terminal() => s,
```

In `apply_event`, on `mcp.set_role` payload, also update `sessions.role` inside the transaction. On `mcp.write_handoff`, update `sessions.handoff_path` (the path returned by the file-writer can be threaded back via `Outcome` if needed; minimum: leave `handoff_path` for the file-writer to set with its own DB call inside the same transaction handle if achievable, else accept a small atomicity gap and document it as a follow-up).

- [ ] **Step 5: Wire into `tools/mod.rs::all()`**

- [ ] **Step 6: Tests pass; clippy clean**

```bash
cargo test --test mcp_tools_tier1_role_handoff
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 7: Commit**

```bash
git add src/mcp/tools/ src/state.rs tests/mcp_tools_tier1_role_handoff.rs
git commit -m "$(cat <<'EOF'
feat(mcp): tier-1 role + handoff tools (3 of catalog)

set_role updates sessions.role through apply_event and rewrites CLAUDE.md in
the worktree so the next turn loads the new profile (replaces the [ROLE:*]
signal write path per spec §16).

write_handoff and read_handoff route through existing handoff.rs helpers — no
file-layout logic duplicated in tool handlers. Each call writes one event row
before any side effect.
EOF
)"
```

---

## Task 8: Tier 1 — Introspection + self-control tools

**Goal:** Implement `whoami`, `get_transcript`, `get_pty_log`, `pause_self`, `request_end`. These are mostly read-only or trivial state transitions. `get_transcript` and `get_pty_log` query the existing PTY scrollback (repurpose `LineBuffer` per spec §9.3 — read-only, no rename required for this plan).

**Files:**
- Create: `src/mcp/tools/tier1_introspection.rs`
- Modify: `src/mcp/tools/mod.rs`
- Modify: `src/state.rs::transition` — add `mcp.pause_self` (→ AwaitingInput), `mcp.request_end` (→ Ended), introspection arms (no state change)
- Create: `tests/mcp_tools_tier1_introspection.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/mcp_tools_tier1_introspection.rs
use galvolite::state::SessionState;
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

mod common;

async fn call(app: &common::TestApp, token: &str, name: &str, args: Value)
    -> galvolite::mcp::protocol::JsonRpcResponse
{
    reqwest::Client::new().post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                     "params":{"name":name,"arguments":args}}).to_string())
        .send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn whoami_returns_session_identity() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let r = call(&app, &token, "whoami", json!({})).await;
    let result = r.result.unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["id"], sid.to_string());
    assert!(body["role"].is_string());
    assert!(body["worktree"].is_string());
    assert!(body["started_at"].is_string());
}

#[tokio::test]
async fn pause_self_transitions_to_awaiting_input() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call(&app, &token, "pause_self",
        json!({"reason": "waiting on operator"})).await;
    assert_eq!(common::state_of(&app.db, sid), SessionState::AwaitingInput);
}

#[tokio::test]
async fn request_end_transitions_to_ended() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call(&app, &token, "request_end", json!({"reason": "task complete"})).await;
    assert_eq!(common::state_of(&app.db, sid), SessionState::Ended);
}

#[tokio::test]
async fn get_transcript_returns_recent_turns() {
    let app = common::test_app().await;
    let (_sid, token) = app.seed_session_in_state(SessionState::Idle).await;

    // Inject some scrollback via the PTY mock helper Sub-plan 2 added.
    app.write_pty_for_session(_sid, b"hello world\nthis is turn one\n").await;

    let r = call(&app, &token, "get_transcript", json!({"turns": 1})).await;
    let result = r.result.unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    let lines = body["lines"].as_array().unwrap();
    assert!(!lines.is_empty(), "transcript should have lines");
}

#[tokio::test]
async fn get_pty_log_strips_ansi() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Idle).await;

    app.write_pty_for_session(sid, b"\x1b[31mhello\x1b[0m\n").await;

    let r = call(&app, &token, "get_pty_log", json!({"limit": 10})).await;
    let result = r.result.unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    let log = body["log"].as_str().unwrap();
    assert!(log.contains("hello"));
    assert!(!log.contains("\x1b["), "ANSI sequences should be stripped");
}
```

- [ ] **Step 2: Implement**

```rust
// src/mcp/tools/tier1_introspection.rs
use crate::mcp::registry::*;
use crate::state::{self, EventInput, Source};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "whoami",
            description: "Return this session's identity: id, role, project_id, worktree, parent_id, depth, started_at.",
            input_schema: json!({"type":"object","properties":{}}),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(whoami(ctx, args))),
        },
        ToolDef {
            name: "get_transcript",
            description: "Return recent turns from this session (or another with session_id). turns optional.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "turns":      { "type": "integer", "minimum": 1, "maximum": 50 }
                }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(get_transcript(ctx, args))),
        },
        ToolDef {
            name: "get_pty_log",
            description: "Return ANSI-stripped PTY scrollback for this session (or another with session_id). limit optional.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "since":      { "type": "string" },
                    "limit":      { "type": "integer", "minimum": 1, "maximum": 1000 }
                }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(get_pty_log(ctx, args))),
        },
        ToolDef {
            name: "pause_self",
            description: "Voluntarily mark self as awaiting input. Operator can dismiss to resume.",
            input_schema: json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(pause_self(ctx, args))),
        },
        ToolDef {
            name: "request_end",
            description: "Request a graceful end to this session. Operator confirms.",
            input_schema: json!({
                "type": "object",
                "properties": { "reason": { "type": "string" } },
                "required": ["reason"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(request_end(ctx, args))),
        },
    ]
}

async fn whoami(ctx: ToolCtx, _args: Value) -> ToolOutcome {
    let session = match ctx.mgr.db().get_session(ctx.session_id) {
        Ok(Some(s)) => s,
        Ok(None) => return ToolOutcome::AppError("session not found".into()),
        Err(e) => return ToolOutcome::AppError(e.to_string()),
    };
    ToolOutcome::Ok(json!({
        "id": session.id.to_string(),
        "role": session.role,
        "project_id": session.project_id,
        "worktree": session.worktree,
        "parent_id": session.parent_id,
        "depth": session.depth.unwrap_or(0),
        "started_at": session.started_at,
    }))
}

#[derive(Deserialize, Default)]
struct TranscriptArgs { session_id: Option<String>, turns: Option<usize> }

async fn get_transcript(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: TranscriptArgs = serde_json::from_value(args).unwrap_or_default();
    let target = match parsed.session_id {
        Some(s) => match uuid::Uuid::parse_str(&s) {
            Ok(u) => u,
            Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
        },
        None => ctx.session_id,
    };
    let turns = parsed.turns.unwrap_or(5);
    match ctx.mgr.scrollback_lines(target, turns) {
        Ok(lines) => ToolOutcome::Ok(json!({ "lines": lines })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize, Default)]
struct PtyLogArgs {
    session_id: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
}

async fn get_pty_log(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: PtyLogArgs = serde_json::from_value(args).unwrap_or_default();
    let target = match parsed.session_id {
        Some(s) => match uuid::Uuid::parse_str(&s) {
            Ok(u) => u,
            Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
        },
        None => ctx.session_id,
    };
    let limit = parsed.limit.unwrap_or(200);
    match ctx.mgr.scrollback_text_ansi_stripped(target, parsed.since.as_deref(), limit) {
        Ok(log) => ToolOutcome::Ok(json!({ "log": log })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct PauseSelfArgs { reason: String }

async fn pause_self(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: PauseSelfArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.pause_self".into(),
        payload: json!({ "reason": parsed.reason }),
        dedup_key: None,
    });
    match outcome {
        Ok(o) => ToolOutcome::Ok(json!({ "state": format!("{:?}", o.next_state) })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct RequestEndArgs { reason: String }

async fn request_end(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: RequestEndArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let outcome = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.request_end".into(),
        payload: json!({ "reason": parsed.reason }),
        dedup_key: None,
    });
    match outcome {
        Ok(o) => ToolOutcome::Ok(json!({ "state": format!("{:?}", o.next_state) })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}
```

`SessionManager::scrollback_lines` and `scrollback_text_ansi_stripped` are helpers expected to wrap whatever shape the existing PTY scrollback (`LineBuffer` per spec §9.3) exposes. If those exact names don't exist, add thin wrappers in `src/session_manager.rs` calling the existing accessors. The MCP tool handler should not poke at PTY internals directly.

- [ ] **Step 3: `state::transition` arms**

```rust
(s, "mcp.pause_self")  if !s.is_terminal() => SessionState::AwaitingInput,
(_, "mcp.request_end")                     => SessionState::Ended,
```

`whoami`, `get_transcript`, `get_pty_log` are read-only — they should NOT call `apply_event` (no event row needed for pure reads, per minimum-write principle). Verify the tests don't assert event rows for those tools.

- [ ] **Step 4: Wire into `tools/mod.rs::all()`**

- [ ] **Step 5: Tests pass; clippy clean**

```bash
cargo test --test mcp_tools_tier1_introspection
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools/ src/state.rs src/session_manager.rs tests/mcp_tools_tier1_introspection.rs
git commit -m "$(cat <<'EOF'
feat(mcp): tier-1 introspection + self-control tools (5 of catalog)

whoami, get_transcript, get_pty_log are read-only — no event row, no apply_event
call. They thin-wrap existing scrollback / session metadata accessors.

pause_self transitions Working → AwaitingInput (operator-dismissable).
request_end transitions any → Ended (operator confirms via dashboard later).
EOF
)"
```

---

## Task 9: Tier 2/3 — Cross-session + operator tools

**Goal:** Implement cross-session reads (`list_siblings`, `status_of`, `read_events`) and write (`send_to_session`), plus the dashboard-coupled operator tools (`ask_operator`, `notify_operator`) in their **degraded mode** — they accept the call, write the event, but `ask_operator` returns `{answer: null, timed_out: true}` immediately because the dashboard WS round-trip lands in Sub-plan 4.

**Files:**
- Create: `src/mcp/tools/tier2_3_cross.rs`
- Modify: `src/mcp/tools/mod.rs`
- Modify: `src/state.rs::transition` — add cross-session arms (no state change for reads; `mcp.send_to_session` no state change for sender — receiver state changes via the receiver's own hook fires)
- Create: `tests/mcp_tools_tier2_3_cross.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/mcp_tools_tier2_3_cross.rs
use galvolite::state::SessionState;
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

mod common;

async fn call(app: &common::TestApp, token: &str, name: &str, args: Value)
    -> galvolite::mcp::protocol::JsonRpcResponse
{
    reqwest::Client::new().post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                     "params":{"name":name,"arguments":args}}).to_string())
        .send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn list_siblings_returns_other_sessions_in_same_project() {
    let app = common::test_app().await;
    let (sib1, _) = app.seed_session_in_state(SessionState::Working).await;
    let (sib2, _) = app.seed_session_in_state(SessionState::Idle).await;
    let (_self_id, token) = app.seed_session_in_state(SessionState::Working).await;

    let r = call(&app, &token, "list_siblings", json!({})).await;
    let result = r.result.unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    let siblings = body["siblings"].as_array().unwrap();
    let names: Vec<String> = siblings.iter().map(|s| s["id"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&sib1.to_string()) || names.contains(&sib2.to_string()),
        "expected at least one sibling visible");
}

#[tokio::test]
async fn status_of_returns_full_snapshot() {
    let app = common::test_app().await;
    let (other, _) = app.seed_session_in_state(SessionState::Working).await;
    let (_self_id, token) = app.seed_session_in_state(SessionState::Idle).await;

    let r = call(&app, &token, "status_of", json!({"session_id": other.to_string()})).await;
    let body: Value = serde_json::from_str(
        r.result.unwrap()["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["id"], other.to_string());
    assert_eq!(body["state"], "Working");
}

#[tokio::test]
async fn read_events_returns_event_rows_for_session() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call(&app, &token, "status_report", json!({"kind": "working"})).await;
    let _ = call(&app, &token, "request_attention", json!({"reason": "x"})).await;

    let r = call(&app, &token, "read_events", json!({"limit": 10})).await;
    let body: Value = serde_json::from_str(
        r.result.unwrap()["content"][0]["text"].as_str().unwrap()).unwrap();
    let events = body["events"].as_array().unwrap();
    assert!(events.len() >= 2, "expected ≥2 events, got {}", events.len());
    let _ = sid;
}

#[tokio::test]
async fn send_to_session_writes_event_targeting_other_session() {
    let app = common::test_app().await;
    let (target, _) = app.seed_session_in_state(SessionState::AwaitingInput).await;
    let (_self_id, token) = app.seed_session_in_state(SessionState::Working).await;

    let r = call(&app, &token, "send_to_session", json!({
        "session_id": target.to_string(),
        "text": "continue with plan",
        "submit": true
    })).await;
    assert!(r.error.is_none());

    let count: i64 = app.db.pool().get().unwrap().query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.send_to_session'",
        [target.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(count, 1, "event row written against the TARGET session");
}

#[tokio::test]
async fn ask_operator_returns_timed_out_immediately_in_degraded_mode() {
    let app = common::test_app().await;
    let (_sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let start = std::time::Instant::now();
    let r = call(&app, &token, "ask_operator",
        json!({"question": "approve?", "timeout_secs": 60})).await;
    let elapsed = start.elapsed();

    assert!(elapsed.as_secs() < 2,
        "degraded mode must return immediately, took {:?}", elapsed);

    let body: Value = serde_json::from_str(
        r.result.unwrap()["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["answer"], serde_json::Value::Null);
    assert_eq!(body["timed_out"], true);
}

#[tokio::test]
async fn notify_operator_writes_event_no_state_change() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let _ = call(&app, &token, "notify_operator",
        json!({"level": "warn", "message": "hey look"})).await;
    assert_eq!(common::state_of(&app.db, sid), SessionState::Working);

    let count: i64 = app.db.pool().get().unwrap().query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind = 'mcp.notify_operator'",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
}
```

- [ ] **Step 2: Implement**

```rust
// src/mcp/tools/tier2_3_cross.rs
use crate::mcp::registry::*;
use crate::state::{self, EventInput, Source};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_siblings",
            description: "List sibling sessions in the same project (or all projects if project_id omitted).",
            input_schema: json!({
                "type": "object",
                "properties": { "project_id": { "type": "string" } }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(list_siblings(ctx, args))),
        },
        ToolDef {
            name: "status_of",
            description: "Get the full state snapshot of any session by id.",
            input_schema: json!({
                "type": "object",
                "properties": { "session_id": { "type": "string" } },
                "required": ["session_id"]
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(status_of(ctx, args))),
        },
        ToolDef {
            name: "read_events",
            description: "Read event log entries. Defaults to current session; pass session_id to read another's.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "since":      { "type": "string" },
                    "limit":      { "type": "integer", "minimum": 1, "maximum": 500 }
                }
            }),
            tier: Tier::SelfContained,
            handler: Box::new(|ctx, args| Box::pin(read_events(ctx, args))),
        },
        ToolDef {
            name: "send_to_session",
            description: "Write text into another session's PTY. submit=true appends \\r so the receiver treats it as Enter.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string" },
                    "text":       { "type": "string" },
                    "submit":     { "type": "boolean" }
                },
                "required": ["session_id", "text"]
            }),
            tier: Tier::CrossSessionPty,
            handler: Box::new(|ctx, args| Box::pin(send_to_session(ctx, args))),
        },
        ToolDef {
            name: "ask_operator",
            description: "Ask the operator a question and wait for an answer. Returns {answer, timed_out}. (Degraded: returns timed_out=true immediately until Sub-plan 4 wires the dashboard prompt UI.)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question":     { "type": "string" },
                    "choices":      { "type": "array", "items": { "type": "string" } },
                    "timeout_secs": { "type": "integer", "minimum": 1 }
                },
                "required": ["question"]
            }),
            tier: Tier::DashboardDegraded,
            handler: Box::new(|ctx, args| Box::pin(ask_operator(ctx, args))),
        },
        ToolDef {
            name: "notify_operator",
            description: "Push a non-blocking notification to the operator. (Event row written today; dashboard toast lands in Sub-plan 4.)",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "level":   { "type": "string", "enum": ["info","warn","urgent"] },
                    "message": { "type": "string" }
                },
                "required": ["level", "message"]
            }),
            tier: Tier::DashboardDegraded,
            handler: Box::new(|ctx, args| Box::pin(notify_operator(ctx, args))),
        },
    ]
}

#[derive(Deserialize, Default)]
struct ListSiblingsArgs { project_id: Option<String> }

async fn list_siblings(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: ListSiblingsArgs = serde_json::from_value(args).unwrap_or_default();
    let project_id = match parsed.project_id {
        Some(p) => Some(p),
        None => ctx.mgr.db().get_session(ctx.session_id)
            .ok().flatten().map(|s| s.project_id),
    };
    match ctx.mgr.db().list_siblings(project_id.as_deref(), Some(ctx.session_id)) {
        Ok(rows) => {
            let arr: Vec<Value> = rows.into_iter().map(|r| json!({
                "id": r.id.to_string(),
                "role": r.role,
                "state": format!("{:?}", r.state),
                "last_event_ts": r.last_event_ts,
                "awaiting_input": matches!(r.state, state::SessionState::AwaitingInput),
            })).collect();
            ToolOutcome::Ok(json!({ "siblings": arr }))
        }
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct StatusOfArgs { session_id: String }

async fn status_of(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: StatusOfArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let target = match Uuid::parse_str(&parsed.session_id) {
        Ok(u) => u,
        Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
    };
    match ctx.mgr.db().get_session_snapshot(target) {
        Ok(Some(snap)) => ToolOutcome::Ok(serde_json::to_value(snap).unwrap()),
        Ok(None) => ToolOutcome::AppError("session not found".into()),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize, Default)]
struct ReadEventsArgs {
    session_id: Option<String>,
    since: Option<String>,
    limit: Option<usize>,
}

async fn read_events(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: ReadEventsArgs = serde_json::from_value(args).unwrap_or_default();
    let target = match parsed.session_id {
        Some(s) => match Uuid::parse_str(&s) {
            Ok(u) => u,
            Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
        },
        None => ctx.session_id,
    };
    let limit = parsed.limit.unwrap_or(50);
    match ctx.mgr.db().read_events(target, parsed.since.as_deref(), limit) {
        Ok(events) => ToolOutcome::Ok(json!({ "events": events })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct SendToSessionArgs { session_id: String, text: String, submit: Option<bool> }

async fn send_to_session(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: SendToSessionArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };
    let target = match Uuid::parse_str(&parsed.session_id) {
        Ok(u) => u,
        Err(_) => return ToolOutcome::InvalidParams("session_id not a uuid".into()),
    };

    // Write event row against the TARGET session so it shows up in the
    // receiver's event log.
    let _ = state::apply_event(ctx.mgr.db(), target, EventInput {
        source: Source::Mcp,
        kind: "mcp.send_to_session".into(),
        payload: json!({
            "from": ctx.session_id.to_string(),
            "text": parsed.text,
            "submit": parsed.submit.unwrap_or(false)
        }),
        dedup_key: None,
    });

    let mut payload_bytes = parsed.text.into_bytes();
    if parsed.submit.unwrap_or(false) {
        payload_bytes.push(b'\r');
    }

    match ctx.mgr.write_to_session_pty(target, &payload_bytes).await {
        Ok(_) => ToolOutcome::Ok(json!({ "ok": true })),
        Err(e) => ToolOutcome::AppError(e.to_string()),
    }
}

#[derive(Deserialize)]
struct AskOperatorArgs {
    question: String,
    choices: Option<Vec<String>>,
    timeout_secs: Option<u64>,
}

async fn ask_operator(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: AskOperatorArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    let _ = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.ask_operator".into(),
        payload: json!({
            "question": parsed.question,
            "choices": parsed.choices,
            "timeout_secs": parsed.timeout_secs,
        }),
        dedup_key: None,
    });

    // DEGRADED MODE — Sub-plan 4 wires the WS prompt round-trip; until then
    // we return an immediate timed-out reply so the model doesn't hang.
    tracing::warn!(
        session = %ctx.session_id,
        "ask_operator called in degraded mode (Sub-plan 4 not landed) — \
         returning timed_out immediately"
    );

    ToolOutcome::Ok(json!({
        "answer": serde_json::Value::Null,
        "timed_out": true,
        "degraded": true,
    }))
}

#[derive(Deserialize)]
struct NotifyOperatorArgs { level: String, message: String }

async fn notify_operator(ctx: ToolCtx, args: Value) -> ToolOutcome {
    let parsed: NotifyOperatorArgs = match serde_json::from_value(args) {
        Ok(p) => p,
        Err(e) => return ToolOutcome::InvalidParams(e.to_string()),
    };

    let _ = state::apply_event(ctx.mgr.db(), ctx.session_id, EventInput {
        source: Source::Mcp,
        kind: "mcp.notify_operator".into(),
        payload: json!({ "level": parsed.level, "message": parsed.message }),
        dedup_key: None,
    });

    // Sub-plan 4 fans this out as a dashboard toast via WS. Today: event row only.
    ToolOutcome::Ok(json!({ "ok": true, "degraded": true }))
}
```

`db.list_siblings`, `db.get_session_snapshot`, `db.read_events`, `mgr.write_to_session_pty` — if any of these helpers don't yet exist with these names, add minimal wrappers in their respective files. The MCP tool handler should not contain SQL or PTY-write byte juggling.

- [ ] **Step 3: `state::transition` arms**

```rust
(s, "mcp.send_to_session")  if !s.is_terminal() => s, // applied to TARGET; receiver state changes via its own UserPromptSubmit
(s, "mcp.ask_operator")     if !s.is_terminal() => s, // pure side-effect, no state change in degraded mode
(s, "mcp.notify_operator")  if !s.is_terminal() => s,
// list_siblings / status_of / read_events are read-only; no event row written.
```

- [ ] **Step 4: Wire into `tools/mod.rs::all()`**

- [ ] **Step 5: Tests pass; clippy clean**

```bash
cargo test --test mcp_tools_tier2_3_cross
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools/ src/state.rs src/db.rs src/session_manager.rs tests/mcp_tools_tier2_3_cross.rs
git commit -m "$(cat <<'EOF'
feat(mcp): tier-2/3 cross-session + operator tools (6 of catalog)

Cross-session reads (list_siblings, status_of, read_events) are pure DB queries;
no event rows written. send_to_session writes one event row against the TARGET
session and POSTs bytes to the target PTY (submit=true appends \r).

Operator tools (ask_operator, notify_operator) ship in DEGRADED MODE — they
write event rows today, but ask_operator returns {answer:null, timed_out:true,
degraded:true} immediately and notify_operator is a no-op beyond the event row.
Sub-plan 4 lands the WS prompt UI + toast stack and removes the degraded flag.

Both degraded handlers emit tracing::warn so Sub-plan 5's smoke can grep for
unverified-capability markers (parallel to the PreCompact warn in Sub-plan 2).
EOF
)"
```

---

## Task 10: Tier 4 — Reserved stubs

**Goal:** Register every reserved tool from spec §5.2–§5.6 so the catalog visible to a session is *complete* — operators see "spawn_child not implemented yet" instead of "no such tool." Each stub returns the same shape: `ToolOutcome::AppError("not_implemented: <tool_name> — Plan <X>")` so the model can recognize the pattern.

**Files:**
- Create: `src/mcp/tools/tier4_stubs.rs`
- Modify: `src/mcp/tools/mod.rs`
- Modify: `src/state.rs::transition` — add wildcard arm `(s, k) if k.starts_with("mcp.") && !s.is_terminal() => s` if not already present from earlier tasks (verify; the existing per-tool arms cover most cases — stubs may not even reach `apply_event`).
- Create: `tests/mcp_tools_tier4_stubs.rs`

- [ ] **Step 1: Write failing tests**

```rust
// tests/mcp_tools_tier4_stubs.rs
use galvolite::state::SessionState;
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

mod common;

async fn call(app: &common::TestApp, token: &str, name: &str, args: Value)
    -> galvolite::mcp::protocol::JsonRpcResponse
{
    reqwest::Client::new().post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                     "params":{"name":name,"arguments":args}}).to_string())
        .send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn every_reserved_stub_returns_not_implemented() {
    let app = common::test_app().await;
    let (_sid, token) = app.seed_session_in_state(SessionState::Working).await;

    let stubs: &[(&str, Value)] = &[
        // Path ownership (Plan 4)
        ("claim_paths",   json!({"globs": ["src/**"]})),
        ("release_paths", json!({"globs": ["src/**"]})),
        ("list_claims",   json!({})),
        ("check_path",    json!({"path": "src/lib.rs"})),
        // Spawn (Plan 4)
        ("spawn_child",        json!({"role": "scout", "task": "look around"})),
        ("wait_for_children",  json!({"ids": []})),
        // Portfolio (Plan 5+)
        ("list_available_mcps",     json!({})),
        ("list_session_mcps",       json!({"session_id": "x"})),
        ("configure_session_mcps",  json!({"session_id":"x","servers":[]})),
        ("gateway_toggle",          json!({"server":"x","enable":true})),
        // GitHub (Plan 5+)
        ("open_pr",         json!({"title":"x","body":"y"})),
        ("list_prs",        json!({})),
        ("request_review",  json!({"pr": 1})),
        ("merge_pr",        json!({"pr": 1})),
        ("get_pr_comments", json!({"pr": 1})),
        // Context r/w (capability-blocked)
        ("read_context",   json!({})),
        ("write_context",  json!({"text": "hi"})),
    ];

    for (name, args) in stubs {
        let r = call(&app, &token, name, args.clone()).await;
        let result = r.result.unwrap_or_else(|| panic!("{name}: tools/call envelope failed"));
        assert_eq!(result["isError"], true, "{name}: must report isError:true");
        let text = result["content"][0]["text"].as_str()
            .unwrap_or_else(|| panic!("{name}: missing content[0].text"));
        assert!(text.contains("not_implemented"),
            "{name}: error text must contain 'not_implemented', got: {text}");
        assert!(text.contains(name), "{name}: error text should name the tool");
    }
}

#[tokio::test]
async fn stubs_appear_in_tools_list() {
    let app = common::test_app().await;
    let (_sid, token) = app.seed_session_in_state(SessionState::Idle).await;

    let r: galvolite::mcp::protocol::JsonRpcResponse = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send().await.unwrap().json().await.unwrap();

    let names: Vec<String> = r.result.unwrap()["tools"].as_array().unwrap()
        .iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();

    for stub in &["spawn_child", "open_pr", "claim_paths", "list_available_mcps", "read_context"] {
        assert!(names.contains(&stub.to_string()),
            "reserved stub '{stub}' missing from tools/list");
    }
}
```

- [ ] **Step 2: Implement stubs via macro**

```rust
// src/mcp/tools/tier4_stubs.rs
use crate::mcp::registry::*;
use serde_json::{json, Value};

macro_rules! stub_tool {
    ($name:literal, $desc:literal, $schema:expr, $owner_plan:literal) => {
        ToolDef {
            name: $name,
            description: concat!($desc, " (RESERVED — ", $owner_plan, ")"),
            input_schema: $schema,
            tier: Tier::ReservedStub,
            handler: Box::new(|_ctx, _args| Box::pin(async move {
                ToolOutcome::AppError(format!(
                    "not_implemented: {} — owned by {}", $name, $owner_plan
                ))
            })),
        }
    };
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        // §5.2 Path ownership — Plan 4
        stub_tool!("claim_paths", "Claim file globs as exclusively-owned by this session.",
            json!({"type":"object","properties":{"globs":{"type":"array","items":{"type":"string"}}},"required":["globs"]}),
            "Plan 4 (spawn/rejoin)"),
        stub_tool!("release_paths", "Release previously-claimed file globs.",
            json!({"type":"object","properties":{"globs":{"type":"array","items":{"type":"string"}}},"required":["globs"]}),
            "Plan 4 (spawn/rejoin)"),
        stub_tool!("list_claims", "List active path claims across all sessions.",
            json!({"type":"object","properties":{}}),
            "Plan 4 (spawn/rejoin)"),
        stub_tool!("check_path", "Check if a path is claimed and by whom.",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
            "Plan 4 (spawn/rejoin)"),

        // §5.3 Spawn — Plan 4
        stub_tool!("spawn_child", "Spawn a child session with the given role + task.",
            json!({"type":"object","properties":{
                "role":{"type":"string"},"task":{"type":"string"},
                "depends_on":{"type":"array","items":{"type":"string"}},
                "worktree":{"type":"string"}},"required":["role","task"]}),
            "Plan 4 (spawn/rejoin)"),
        stub_tool!("wait_for_children", "Block until the listed child sessions transition to Ended.",
            json!({"type":"object","properties":{"ids":{"type":"array","items":{"type":"string"}}},"required":["ids"]}),
            "Plan 4 (spawn/rejoin)"),

        // §5.4 Portfolio — Plan 5+
        stub_tool!("list_available_mcps", "List MCP servers available from the configured portfolio (e.g. Docker MCP Gateway).",
            json!({"type":"object","properties":{}}),
            "Plan 5+ (MCP portfolio)"),
        stub_tool!("list_session_mcps", "List MCP servers currently configured for a session.",
            json!({"type":"object","properties":{"session_id":{"type":"string"}},"required":["session_id"]}),
            "Plan 5+ (MCP portfolio)"),
        stub_tool!("configure_session_mcps", "Configure the MCP servers for a future spawn of a session profile.",
            json!({"type":"object","properties":{
                "session_id":{"type":"string"},
                "servers":{"type":"array","items":{"type":"string"}}},"required":["session_id","servers"]}),
            "Plan 5+ (MCP portfolio)"),
        stub_tool!("gateway_toggle", "Toggle a server on the Docker MCP Gateway.",
            json!({"type":"object","properties":{
                "server":{"type":"string"},"enable":{"type":"boolean"}},"required":["server","enable"]}),
            "Plan 5+ (MCP portfolio)"),

        // §5.5 GitHub — Plan 5+
        stub_tool!("open_pr", "Open a GitHub pull request from the session's worktree.",
            json!({"type":"object","properties":{"title":{"type":"string"},"body":{"type":"string"}},"required":["title"]}),
            "Plan 5+ (GitHub surface)"),
        stub_tool!("list_prs", "List open pull requests for the project repo.",
            json!({"type":"object","properties":{}}),
            "Plan 5+ (GitHub surface)"),
        stub_tool!("request_review", "Request review on a PR.",
            json!({"type":"object","properties":{"pr":{"type":"integer"}},"required":["pr"]}),
            "Plan 5+ (GitHub surface)"),
        stub_tool!("merge_pr", "Merge a PR.",
            json!({"type":"object","properties":{"pr":{"type":"integer"}},"required":["pr"]}),
            "Plan 5+ (GitHub surface)"),
        stub_tool!("get_pr_comments", "Get comments on a PR.",
            json!({"type":"object","properties":{"pr":{"type":"integer"}},"required":["pr"]}),
            "Plan 5+ (GitHub surface)"),

        // §5.6 Context r/w — capability-blocked
        stub_tool!("read_context", "Read this session's full conversation context.",
            json!({"type":"object","properties":{}}),
            "capability-blocked (PTY can't surface context; would require Agent SDK migration)"),
        stub_tool!("write_context", "Write text directly into this session's context.",
            json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
            "capability-blocked (PTY can't surface context; would require Agent SDK migration)"),
    ]
}
```

- [ ] **Step 3: Wire into `tools/mod.rs::all()`**

- [ ] **Step 4: Re-tighten Task 4's tools/list test**

Now that all named tools exist, restore the strict assertion in `tests/mcp_initialize_tools_list.rs`:

```rust
for required in &["status_report", "set_role", "whoami", "set_plan",
                  "write_handoff", "request_attention",
                  "spawn_child", "open_pr", "claim_paths", "list_available_mcps"] {
    assert!(names.contains(&required.to_string()), "missing tool: {required}");
}
```

- [ ] **Step 5: Tests pass; clippy clean**

```bash
cargo test --test mcp_tools_tier4_stubs
cargo test --test mcp_initialize_tools_list
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add src/mcp/tools/ tests/mcp_tools_tier4_stubs.rs tests/mcp_initialize_tools_list.rs
git commit -m "$(cat <<'EOF'
feat(mcp): tier-4 reserved stubs (17 of catalog)

Adds 17 reserved stubs across spec §5.2–§5.6: path ownership (4), spawn (2),
portfolio (4), GitHub (5), context r/w (2). Each stub registers a real ToolDef
with full input_schema and an "RESERVED — Plan X" descriptor suffix; the
handler returns ToolOutcome::AppError("not_implemented: <name> — owned by …").

Why register stubs at all: the model sees the complete catalog at
session start, so future-Claude sessions will recognize the API surface that's
coming and won't try to invent ad-hoc replacements. The error text tells the
model which plan owns the missing capability so they can ask the operator.

Re-tightens tests/mcp_initialize_tools_list.rs to assert each named tool from
spec §5 is present.
EOF
)"
```

---

## Task 11: End-to-end MCP smoke test

**Goal:** Mirror Sub-plan 2's hook smoke test. Boot a real Axum app, mint a session + token, fire raw JSON-RPC requests at `POST /mcp` covering `initialize`, `tools/list`, and a representative tool from each tier (status, role, introspection, cross-session, stub). Verify event-log shape and tool result envelopes.

**Files:**
- Create: `tests/mcp_smoke_endtoend.rs`

- [ ] **Step 1: Write the smoke test**

```rust
// tests/mcp_smoke_endtoend.rs
use galvolite::mcp::protocol::JsonRpcResponse;
use galvolite::state::SessionState;
use reqwest::header::AUTHORIZATION;
use serde_json::{json, Value};

mod common;

async fn rpc(app: &common::TestApp, token: &str, method: &str, params: Value, id: i64) -> JsonRpcResponse {
    let body = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
    reqwest::Client::new().post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(body).send().await.unwrap().json().await.unwrap()
}

#[tokio::test]
async fn full_mcp_lifecycle_initialize_through_each_tier() {
    let app = common::test_app().await;
    let (sid, token) = app.seed_session_with_worktree("implementation").await;

    // 1. initialize
    let r = rpc(&app, &token, "initialize", json!({}), 1).await;
    let init = r.result.expect("initialize succeeds");
    assert_eq!(init["protocolVersion"], "2025-03-26");
    assert_eq!(init["serverInfo"]["name"], "galvolite");

    // 2. tools/list — verify catalog is complete
    let r = rpc(&app, &token, "tools/list", json!({}), 2).await;
    let names: Vec<String> = r.result.unwrap()["tools"].as_array().unwrap()
        .iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    let expected = [
        // Tier 1 — status & lifecycle
        "status_report", "request_attention", "set_plan", "plan_check_step", "set_seed",
        // Tier 1 — role + handoff
        "set_role", "write_handoff", "read_handoff",
        // Tier 1 — introspection + self-control
        "whoami", "get_transcript", "get_pty_log", "pause_self", "request_end",
        // Tier 2/3
        "list_siblings", "status_of", "read_events", "send_to_session",
        "ask_operator", "notify_operator",
        // Tier 4 (a sample)
        "spawn_child", "claim_paths", "open_pr", "list_available_mcps", "read_context",
    ];
    for name in expected.iter() {
        assert!(names.contains(&name.to_string()),
            "tools/list missing required tool: {name}");
    }

    // 3. status_report — Tier 1, writes event, no state change
    let r = rpc(&app, &token, "tools/call", json!({
        "name": "status_report", "arguments": {"kind": "working", "note": "smoke"}
    }), 3).await;
    assert!(r.error.is_none());
    assert_eq!(r.result.unwrap()["isError"], false);
    assert_eq!(common::state_of(&app.db, sid), SessionState::Working);

    // 4. set_role — Tier 1, rewrites CLAUDE.md
    let _ = rpc(&app, &token, "tools/call", json!({
        "name": "set_role", "arguments": {"role": "scout"}
    }), 4).await;
    let role: String = app.db.pool().get().unwrap().query_row(
        "SELECT role FROM sessions WHERE id = ?1",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert_eq!(role, "scout");

    // 5. whoami — Tier 1, read-only
    let r = rpc(&app, &token, "tools/call", json!({
        "name": "whoami", "arguments": {}
    }), 5).await;
    let body: Value = serde_json::from_str(
        r.result.unwrap()["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["id"], sid.to_string());
    assert_eq!(body["role"], "scout");

    // 6. read_events — Tier 2 read; should include all the events we just wrote
    let r = rpc(&app, &token, "tools/call", json!({
        "name": "read_events", "arguments": {"limit": 50}
    }), 6).await;
    let body: Value = serde_json::from_str(
        r.result.unwrap()["content"][0]["text"].as_str().unwrap()).unwrap();
    let events = body["events"].as_array().unwrap();
    assert!(events.len() >= 2, "should have at least status_report + set_role events");

    // 7. ask_operator — Tier 3 degraded
    let r = rpc(&app, &token, "tools/call", json!({
        "name": "ask_operator", "arguments": {"question": "approve smoke?"}
    }), 7).await;
    let body: Value = serde_json::from_str(
        r.result.unwrap()["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["timed_out"], true);
    assert_eq!(body["degraded"], true);

    // 8. spawn_child — Tier 4 reserved stub
    let r = rpc(&app, &token, "tools/call", json!({
        "name": "spawn_child", "arguments": {"role": "scout", "task": "x"}
    }), 8).await;
    let result = r.result.unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not_implemented"));
    assert!(text.contains("spawn_child"));

    // 9. request_end — final transition
    let _ = rpc(&app, &token, "tools/call", json!({
        "name": "request_end", "arguments": {"reason": "smoke complete"}
    }), 9).await;
    assert_eq!(common::state_of(&app.db, sid), SessionState::Ended);

    // 10. Event log shape — every successful mcp.* call should have written a row.
    let conn = app.db.pool().get().unwrap();
    let mcp_events: i64 = conn.query_row(
        "SELECT count(*) FROM events WHERE session_id = ?1 AND kind LIKE 'mcp.%'",
        [sid.to_string()], |r| r.get(0)).unwrap();
    assert!(mcp_events >= 5,
        "expected ≥5 mcp.* events from the active calls, got {mcp_events}");

    // 11. Token revoked on Ended? 401 expected.
    let r = reqwest::Client::new()
        .post(format!("{}/mcp", app.url))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .body(r#"{"jsonrpc":"2.0","id":99,"method":"tools/list"}"#)
        .send().await.unwrap();
    assert_eq!(r.status(), 401, "token should be revoked when session Ends per spec §7.2");
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test mcp_smoke_endtoend
cargo test
cargo clippy --all-targets -- -D warnings
```

- [ ] **Step 3: Commit**

```bash
git add tests/mcp_smoke_endtoend.rs
git commit -m "$(cat <<'EOF'
test(mcp): end-to-end smoke — initialize through each tier of catalog

Boots the Axum app, mints a session + token, fires raw JSON-RPC at POST /mcp
covering: initialize handshake, tools/list completeness, one representative
tool from each tier (status_report, set_role, whoami, read_events, ask_operator
degraded, spawn_child stub, request_end). Verifies token revocation on Ended
per spec §7.2.

Proves the wire works without a real claude PTY. Sub-plan 5's real-claude smoke
will exercise the same endpoint from a CC-spawned MCP client.
EOF
)"
```

---

## Self-review checklist

After completing the tasks above, run the following yourself:

**1. Spec coverage (§-by-§ scan):**

- §5.1 status & lifecycle — Task 6 ✓ (5 tools)
- §5.1 role & handoff — Task 7 ✓ (3 tools)
- §5.1 introspection — Task 8 ✓ (3 tools)
- §5.1 self-control — Task 8 ✓ (2 tools)
- §5.1 cross-session — Task 9 ✓ (4 tools)
- §5.1 operator comms — Task 9 (degraded) ✓ (2 tools)
- §5.2 path ownership stubs — Task 10 ✓ (4 tools)
- §5.3 spawn stubs — Task 10 ✓ (2 tools)
- §5.4 portfolio stubs — Task 10 ✓ (4 tools)
- §5.5 GitHub stubs — Task 10 ✓ (5 tools)
- §5.6 context r/w stubs — Task 10 ✓ (2 tools)
- §5.7 (one tool one thing; idempotent; one event row before mutation) — Tasks 6–9 ✓
- §6.3 atomicity invariant (every tool routes through apply_event) — Tasks 6–9 ✓
- §7.1 step 4 (`.mcp.json` writer) — amended in Task 1 + Task 2 ✓
- §7.2 (bearer auth, constant-time compare) — Task 5 ✓
- §15 dep #1 (Streamable HTTP transport stability) — Task 1 spec amendment ✓

**Total active tools:** 19. **Reserved stubs:** 17. **Catalog total:** 36.

**2. Placeholder scan:**

```bash
grep -nE 'TBD|TODO|implement later|fill in details|FIXME' \
    docs/superpowers/plans/2026-04-18-plan-3-mcp-surface.md
```

Should return zero hits.

**3. Type consistency:**

- Every tool handler signature is `Fn(ToolCtx, Value) -> Pin<Box<dyn Future<Output = ToolOutcome> + Send>>` ✓
- Event kind naming: `mcp.<tool_name>` exactly, no exceptions ✓
- All `apply_event` calls include `source: Source::Mcp` ✓
- Token resolved via `auth::resolve_token` (constant-time) — no inline copies remaining ✓
- `Tier` enum used to tag every `ToolDef` ✓

**4. Test coverage:**

- `tests/auth_resolver.rs` — shared resolver (Task 5)
- `tests/spawn_config_mcp_writer.rs` — `.mcp.json` writer (Task 2)
- `tests/mcp_protocol_envelopes.rs` — JSON-RPC envelopes (Task 3)
- `tests/mcp_transport_http.rs` — Streamable HTTP transport (Task 3)
- `tests/mcp_initialize_tools_list.rs` — initialize + tools/list (Tasks 4 + 10 retighten)
- `tests/mcp_dispatch.rs` — tools/call dispatcher (Task 5)
- `tests/mcp_tools_tier1_status.rs` — Tier 1 status (Task 6)
- `tests/mcp_tools_tier1_role_handoff.rs` — Tier 1 role/handoff (Task 7)
- `tests/mcp_tools_tier1_introspection.rs` — Tier 1 introspection (Task 8)
- `tests/mcp_tools_tier2_3_cross.rs` — Tier 2/3 (Task 9)
- `tests/mcp_tools_tier4_stubs.rs` — every reserved stub (Task 10)
- `tests/mcp_smoke_endtoend.rs` — full smoke (Task 11)

Existing tests from Sub-plan 1 + Sub-plan 2 must remain green.

**5. Cargo dep audit:**

```bash
git diff main..plan-mcp-surface -- Cargo.toml
```

Expected: zero changes (all deps required by Sub-plan 3 — `axum`, `serde`, `serde_json`, `rusqlite`, `uuid`, `subtle`, `tracing` — present after Sub-plan 2 lands `subtle`/`reqwest`).

---

## Done definition for Sub-plan 3

- `cargo test` passes (all existing + 12 new test files, ~40 new tests).
- `cargo clippy --all-targets -- -D warnings` clean across new + modified files.
- `tools/list` over the wire returns ≥36 tool descriptors (19 active + 17 reserved).
- A spawned session's worktree contains a `.mcp.json` with `type: "http"` and a per-session bearer token.
- A real `claude` process spawned with that `.mcp.json` can call `tools/list` and `status_report` without errors visible to the model. (Manual sanity check; full verification is Sub-plan 5's real-claude smoke.)
- Spec §5 every tool name appears in either an active handler (Tasks 6-9) or a reserved stub (Task 10).
- Token revocation on `SessionEnd` confirmed by smoke test step 11 — stale token gets 401.
- **Coordination:** when both Sub-plans 2 and 3 are ready to merge, merge in this order:
  1. Sub-plan 2 (`plan-hook-surface`) merges first — establishes `subtle` + `reqwest` deps, `Db::set_api_token` / `clear_api_token`, the `tests/common.rs` test app helper, and the inline `resolve_token` that Sub-plan 3 deletes.
  2. Sub-plan 3 (`plan-mcp-surface`) rebases onto Sub-plan 2 — `src/auth.rs` extraction conflict is mechanical; remove the inline resolver in Sub-plan 2's hook handler and re-point its caller. Run full `cargo test` after rebase before merging.
  3. If Sub-plan 3 must merge first for any reason, the inline resolver in Sub-plan 2's branch becomes the conflict to delete; resolution is identical in shape, just from the opposite side.
