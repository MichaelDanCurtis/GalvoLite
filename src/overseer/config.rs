//! Env-config helpers for Overseer + Plan Executor feature flags
//! and Kimi K2 API credentials.

const DEFAULT_KIMI_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const DEFAULT_KIMI_MODEL: &str = "kimi-for-coding";
const DEFAULT_DAILY_TOKEN_BUDGET: u64 = 100_000;

pub fn overseer_enabled() -> bool {
    truthy(std::env::var("GALVOLITE_OVERSEER").ok().as_deref())
}

pub fn executor_enabled() -> bool {
    truthy(std::env::var("GALVOLITE_PLAN_EXECUTOR").ok().as_deref())
}

fn truthy(v: Option<&str>) -> bool {
    matches!(v.map(str::to_ascii_lowercase).as_deref(), Some("1" | "true" | "yes" | "on"))
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
