//! Per-hook binary executed by Claude Code.
//!
//! Reads payload from stdin, POSTs to `/hooks/ingest`, writes the response's
//! `inject_text` (if any) to stdout. On UserPromptSubmit, intercepts the
//! `[GALVOLITE:REJOIN <uuid>]` marker by fetching the rejoin markdown and
//! emitting `{"decision":"block","additionalContext":<md>}` so Claude resumes
//! with the children-resolved payload instead of the literal marker. All
//! errors log to stderr and the process exits 0 — fail-soft so claude never
//! sees infra failures.

use galvolite::hooks::{client, marker, types::HookKind};
use std::io::{self, Read, Write};

fn main() {
    if let Err(e) = run() {
        eprintln!("[galvolite-hook] error: {e}");
    }
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

    if matches!(hook, HookKind::UserPromptSubmit) {
        let prompt = payload.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(parent_id) = marker::detect_rejoin_marker(prompt) {
            if parent_id.to_string() == env.session_id {
                match client::fetch_pending_rejoin(&env)? {
                    Some(md) => {
                        let response = serde_json::json!({
                            "decision": "block",
                            "additionalContext": md,
                        });
                        io::stdout().write_all(response.to_string().as_bytes())?;
                        return Ok(());
                    }
                    None => {
                        eprintln!("[galvolite-hook] marker matched but /pending-rejoin returned 404 — falling through");
                    }
                }
            }
        }
    }

    let outcome = client::run(env, hook, payload)?;
    if let Some(text) = outcome.inject_text {
        io::stdout().write_all(text.as_bytes())?;
    }
    Ok(())
}
