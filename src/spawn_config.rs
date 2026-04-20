//! Per-session configuration files written into a session's worktree at spawn:
//!
//!   * `.claude/settings.json` — full seven-hook dispatch table pointing at the
//!     resolved `galvolite-hook` binary (Sub-plan 2).
//!   * `.mcp.json` — Streamable HTTP transport entry (per amended spec §7.1)
//!     pointing the spawned `claude` process at the running GalvoLite at
//!     `<api_base>/mcp` with the per-session bearer token (Sub-plan 3).
//!
//! The `portfolio` MCP server entry is intentionally omitted — it lands in a
//! later plan when the `McpPortfolio` trait + Docker MCP Gateway integration
//! are in place.

use crate::error::{Error, Result};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Resolve the path to the `galvolite-hook` binary.
///
/// Order:
/// 1. `GALVOLITE_HOOK_BIN` env var if set (tests inject this).
/// 2. Sibling of `current_exe()` named `galvolite-hook`.
pub fn resolve_hook_binary() -> std::io::Result<PathBuf> {
    if let Ok(p) = std::env::var("GALVOLITE_HOOK_BIN") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent dir on current_exe"))?;
    Ok(dir.join("galvolite-hook"))
}

/// Write `<worktree>/.claude/settings.json` with all seven hook entries
/// dispatching to `hook_bin`. Overwrites any existing file.
pub fn write_settings_json(
    worktree: &Path,
    hook_bin: impl AsRef<str>,
) -> std::io::Result<()> {
    let dot_claude = worktree.join(".claude");
    std::fs::create_dir_all(&dot_claude)?;

    let bin = hook_bin.as_ref();
    let body = json!({
        "hooks": {
            "SessionStart":     [{"command": bin, "args": ["SessionStart"]}],
            "SessionEnd":       [{"command": bin, "args": ["SessionEnd"]}],
            "PreCompact":       [{"command": bin, "args": ["PreCompact"]}],
            "Stop":             [{"command": bin, "args": ["Stop"]}],
            "UserPromptSubmit": [{"command": bin, "args": ["UserPromptSubmit"]}],
            "PostToolUse":      [{"command": bin, "args": ["PostToolUse"], "tool": "Bash"}],
            "Notification":     [{"command": bin, "args": ["Notification"]}],
        }
    });

    let path = dot_claude.join("settings.json");
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap())?;
    Ok(())
}

/// Write `<worktree>/.mcp.json` with the Streamable HTTP transport entry per
/// amended spec §7.1. Overwrites any existing file.
pub fn write_mcp_json(worktree: &Path, api_base: &str, token: &str) -> Result<()> {
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
    std::fs::write(
        &mcp_path,
        serde_json::to_string_pretty(&mcp_body)
            .map_err(|e| Error::Config(format!(".mcp.json serialize failed: {e}")))?,
    )
    .map_err(|e| Error::io(&mcp_path, e))?;
    Ok(())
}

/// Write both per-session config files. Used by callers that have a resolved
/// hook binary, api base, and bearer token in hand. Equivalent to calling
/// `write_settings_json` followed by `write_mcp_json`.
pub fn write_session_files(
    worktree: &Path,
    hook_bin: &str,
    api_base: &str,
    token: &str,
) -> Result<()> {
    write_settings_json(worktree, hook_bin)
        .map_err(|e| Error::io(worktree.join(".claude/settings.json"), e))?;
    write_mcp_json(worktree, api_base, token)?;
    Ok(())
}
