//! Per-session `.claude/settings.json` writer.
//!
//! Spawned at session-start so the worktree's hook table dispatches all seven
//! Claude Code hook entries through the resolved `galvolite-hook` binary.
//! The `.mcp.json` writer (companion to this one) is Sub-plan 3's concern.

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
    // CC's settings.json hook format — confirm exact shape from CC docs at
    // integration time (Sub-plan 5 real-claude smoke). Today's shape mirrors
    // what spec §7.1 describes.
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
