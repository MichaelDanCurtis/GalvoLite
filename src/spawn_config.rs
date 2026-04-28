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

/// Resolve the GalvoLite HTTP base URL spawned `claude` processes should
/// reach via Streamable HTTP transport. `.mcp.json` puts this in `url`.
///
/// Order:
/// 1. `GALVOLITE_API_BASE` env var if set (overrides everything).
/// 2. `http://127.0.0.1:<GALVOLITE_PORT>` if `GALVOLITE_PORT` is set.
/// 3. Default `http://127.0.0.1:3001` (matches the tauri shell default).
pub fn resolve_api_base() -> String {
    if let Ok(v) = std::env::var("GALVOLITE_API_BASE") {
        return v;
    }
    let port = std::env::var("GALVOLITE_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(3001);
    format!("http://127.0.0.1:{port}")
}

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

/// Shell-quote `bin` for safe inclusion in a `command` string. Claude Code
/// runs `hooks[*].command` via a shell, so paths containing spaces (the
/// common case on macOS with `/Volumes/Main External/...`) must be quoted.
/// Uses POSIX single-quoting: wrap in `'…'` and escape embedded `'` as
/// `'\''`.
fn shell_quote(bin: &str) -> String {
    let escaped = bin.replace('\'', "'\\''");
    format!("'{escaped}'")
}

fn hook_entry(bin: &str, kind: &str) -> serde_json::Value {
    json!({
        "hooks": [
            { "type": "command", "command": format!("{} {kind}", shell_quote(bin)) }
        ]
    })
}

fn hook_entry_matcher(bin: &str, kind: &str, matcher: &str) -> serde_json::Value {
    json!({
        "matcher": matcher,
        "hooks": [
            { "type": "command", "command": format!("{} {kind}", shell_quote(bin)) }
        ]
    })
}

/// Write `<worktree>/.claude/settings.json` with all seven hook entries
/// dispatching to `hook_bin`. Overwrites any existing file.
///
/// Emits Claude Code's documented hook shape:
/// `"<Kind>": [{ "matcher"?: str, "hooks": [{"type": "command", "command": "..." }] }]`
/// — the `{command, args, tool}` shape an earlier draft wrote is NOT what
/// Claude Code parses; it makes every hook type fail validation on startup
/// and poisons the host CLI until settings.json is removed or rewritten.
pub fn write_settings_json(
    worktree: &Path,
    hook_bin: impl AsRef<str>,
) -> std::io::Result<()> {
    let dot_claude = worktree.join(".claude");
    std::fs::create_dir_all(&dot_claude)?;

    let bin = hook_bin.as_ref();
    let body = json!({
        "hooks": {
            "SessionStart":     [hook_entry(bin, "SessionStart")],
            "SessionEnd":       [hook_entry(bin, "SessionEnd")],
            "PreCompact":       [hook_entry(bin, "PreCompact")],
            "Stop":             [hook_entry(bin, "Stop")],
            "UserPromptSubmit": [hook_entry(bin, "UserPromptSubmit")],
            "PostToolUse":      [hook_entry_matcher(bin, "PostToolUse", "Bash")],
            "Notification":     [hook_entry(bin, "Notification")],
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
