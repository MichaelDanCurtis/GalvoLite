//! Env-config helpers for Overseer + Plan Executor feature flags
//! and Kimi K2 API credentials.

const DEFAULT_KIMI_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const DEFAULT_KIMI_MODEL: &str = "kimi-for-coding";
const DEFAULT_DAILY_TOKEN_BUDGET: u64 = 100_000;

pub fn overseer_enabled() -> bool {
    enabled_with_default(std::env::var("GALVOLITE_OVERSEER").ok().as_deref(), true)
}

pub fn executor_enabled() -> bool {
    enabled_with_default(std::env::var("GALVOLITE_PLAN_EXECUTOR").ok().as_deref(), true)
}

fn enabled_with_default(v: Option<&str>, default: bool) -> bool {
    match v.map(str::to_ascii_lowercase).as_deref() {
        None => default,
        Some("1" | "true" | "yes" | "on") => true,
        Some("0" | "false" | "no" | "off") => false,
        // Unknown values are conservative: treat as off when the user
        // bothered to set the var to something.
        Some(_) => false,
    }
}

#[derive(Debug, Clone)]
pub struct OverseerConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub daily_token_budget: u64,
}

impl OverseerConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("KIMI_API_KEY").ok()?;
        let base_url = std::env::var("KIMI_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_KIMI_BASE_URL.to_string());
        let model = std::env::var("KIMI_MODEL")
            .unwrap_or_else(|_| DEFAULT_KIMI_MODEL.to_string());
        let daily_token_budget = std::env::var("KIMI_DAILY_TOKEN_BUDGET")
            .ok().and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_DAILY_TOKEN_BUDGET);
        Some(Self { api_key, base_url, model, daily_token_budget })
    }
}
