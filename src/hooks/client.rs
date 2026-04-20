//! Pure logic for the `galvolite-hook` binary.
//!
//! Stays free of I/O concerns beyond the single HTTP call so it can be
//! exercised by `mockito`-backed unit tests without spawning the binary.

use crate::hooks::types::*;
use std::time::Duration;
use uuid::Uuid;

/// Environment passed in by the binary entry point.
pub struct ClientEnv {
    pub api_base: String,
    pub session_id: String,
    pub token: String,
    pub timeout_secs: u64,
}

/// Successful outcome: any text the server asked us to inject.
#[derive(Debug, Default)]
pub struct ClientOutcome {
    pub inject_text: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
}

/// POST a hook envelope to `/hooks/ingest` and return the inject_text.
///
/// On non-2xx responses we still return `Ok(default)` rather than `Err`
/// so the binary can keep its fail-soft promise — only network/transport
/// failures become `ClientError`.
pub fn run(
    env: ClientEnv,
    hook: HookKind,
    payload: serde_json::Value,
) -> Result<ClientOutcome, ClientError> {
    let req = HookIngestRequest {
        session_id: env.session_id,
        hook,
        ts: chrono::Utc::now().to_rfc3339(),
        event_id: Uuid::new_v4().to_string(),
        payload,
    };

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(env.timeout_secs))
        .build()?;

    let resp = client
        .post(format!("{}/hooks/ingest", env.api_base))
        .header("X-Galvolite-Token", &env.token)
        .json(&req)
        .send()?;

    if !resp.status().is_success() {
        return Ok(ClientOutcome::default());
    }

    let parsed: HookIngestResponse = resp.json()?;
    Ok(ClientOutcome {
        inject_text: parsed.inject_text,
    })
}
