//! Per-hook binary executed by Claude Code.
//!
//! Reads payload from stdin, POSTs to `/hooks/ingest`, writes the response's
//! `inject_text` (if any) to stdout. All errors log to stderr and the
//! process exits 0 — fail-soft so claude never sees infra failures.

use galvolite::hooks::{client, types::HookKind};
use std::io::{self, Read, Write};

fn main() {
    if let Err(e) = run() {
        eprintln!("[galvolite-hook] error: {e}");
    }
    // Always exit 0: hooks are fail-soft so claude never sees our errors.
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let env = client::ClientEnv {
        api_base: std::env::var("GALVOLITE_API_BASE")?,
        session_id: std::env::var("GALVOLITE_SESSION_ID")?,
        token: std::env::var("GALVOLITE_TOKEN")?,
        timeout_secs: 5,
    };

    let hook_str = std::env::args().nth(1).ok_or("missing hook arg")?;
    let hook: HookKind = serde_json::from_value(serde_json::Value::String(hook_str))?;

    let mut payload_buf = String::new();
    io::stdin().read_to_string(&mut payload_buf)?;
    let payload: serde_json::Value = if payload_buf.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&payload_buf)?
    };

    let outcome = client::run(env, hook, payload)?;
    if let Some(text) = outcome.inject_text {
        io::stdout().write_all(text.as_bytes())?;
    }
    Ok(())
}
