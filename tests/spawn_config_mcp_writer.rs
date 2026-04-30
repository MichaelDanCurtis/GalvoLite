use galvolite::spawn_config;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn writes_mcp_json_with_http_transport_and_bearer() {
    let tmp = TempDir::new().unwrap();
    let worktree = tmp.path();
    spawn_config::write_mcp_json(worktree, "http://127.0.0.1:3001", "test-token-abc").unwrap();

    let mcp_path = worktree.join(".mcp.json");
    assert!(mcp_path.exists(), ".mcp.json was not written");

    let body: Value = serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let server = &body["mcpServers"]["galvolite"];

    assert_eq!(
        server["type"], "http",
        "transport must be Streamable HTTP per spec §7.1 (amended)"
    );
    assert_eq!(server["url"], "http://127.0.0.1:3001/mcp");
    assert_eq!(
        server["headers"]["Authorization"],
        "Bearer test-token-abc"
    );
}

#[test]
fn mcp_json_omits_portfolio_entry_when_no_gateway() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_mcp_json(tmp.path(), "http://127.0.0.1:3001", "tok").unwrap();

    let body: Value =
        serde_json::from_str(&std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap())
            .unwrap();

    assert!(
        body["mcpServers"].get("portfolio").is_none(),
        "portfolio entry only when Docker MCP Gateway configured (Plan 5+ scope)"
    );
}

#[test]
fn rewriting_overwrites_old_mcp_json() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_mcp_json(tmp.path(), "http://a/", "tok-1").unwrap();
    spawn_config::write_mcp_json(tmp.path(), "http://b/", "tok-2").unwrap();

    let body: Value =
        serde_json::from_str(&std::fs::read_to_string(tmp.path().join(".mcp.json")).unwrap())
            .unwrap();
    assert_eq!(body["mcpServers"]["galvolite"]["url"], "http://b/mcp");
    assert_eq!(
        body["mcpServers"]["galvolite"]["headers"]["Authorization"],
        "Bearer tok-2"
    );
}
