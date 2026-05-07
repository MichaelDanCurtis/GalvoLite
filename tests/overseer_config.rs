use galvolite::overseer::config::{OverseerConfig, executor_enabled, overseer_enabled};
use std::sync::Mutex;

static MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn defaults_on_when_env_unset() {
    let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("GALVOLITE_OVERSEER");
    std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
    assert!(overseer_enabled());
    assert!(executor_enabled());
}

#[test]
fn disabled_when_env_set_falsy() {
    let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("GALVOLITE_OVERSEER", "0");
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "off");
    assert!(!overseer_enabled());
    assert!(!executor_enabled());
    std::env::remove_var("GALVOLITE_OVERSEER");
    std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
}

#[test]
fn enabled_when_env_set_truthy() {
    let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("GALVOLITE_OVERSEER", "1");
    std::env::set_var("GALVOLITE_PLAN_EXECUTOR", "true");
    assert!(overseer_enabled());
    assert!(executor_enabled());
    std::env::remove_var("GALVOLITE_OVERSEER");
    std::env::remove_var("GALVOLITE_PLAN_EXECUTOR");
}

#[test]
fn config_from_env_returns_none_when_key_missing() {
    let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("KIMI_API_KEY");
    std::env::remove_var("KIMI_BASE_URL");
    assert!(OverseerConfig::from_env().is_none());
}

#[test]
fn config_from_env_uses_default_base_url_when_only_key_set() {
    let _lock = MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("KIMI_API_KEY", "k");
    std::env::remove_var("KIMI_BASE_URL");
    std::env::remove_var("KIMI_MODEL");
    let c = OverseerConfig::from_env().unwrap();
    assert_eq!(c.api_key, "k");
    assert_eq!(c.base_url, "https://api.kimi.com/coding/v1");
    assert_eq!(c.model, "kimi-for-coding");
    assert_eq!(c.daily_token_budget, 100_000);
    std::env::remove_var("KIMI_API_KEY");
}
