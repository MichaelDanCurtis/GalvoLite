use galvolite::spawn_config;
use serde_json::Value;
use std::sync::Mutex;
use tempfile::TempDir;

// Env-var tests must not race each other. Other tests in the file don't
// touch the env, so a single mutex around env-var ones is enough.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn writes_settings_json_with_seven_hook_entries() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_settings_json(tmp.path(), "/path/to/galvolite-hook").unwrap();

    let path = tmp.path().join(".claude/settings.json");
    assert!(path.exists());

    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let hooks = v["hooks"].as_object().expect(".hooks must be an object");

    for kind in [
        "SessionStart",
        "SessionEnd",
        "PreCompact",
        "Stop",
        "UserPromptSubmit",
        "PostToolUse",
        "Notification",
    ] {
        assert!(hooks.contains_key(kind), "missing hook entry: {kind}");
    }

    // Each entry must point at the real binary path.
    let stop = &hooks["Stop"];
    let cmd_str = serde_json::to_string(stop).unwrap();
    assert!(
        cmd_str.contains("/path/to/galvolite-hook"),
        "binary path missing in: {cmd_str}"
    );
}

#[test]
fn writes_into_subdir_creating_it_if_missing() {
    let tmp = TempDir::new().unwrap();
    // No .claude dir yet
    spawn_config::write_settings_json(tmp.path(), "/bin/hook").unwrap();
    assert!(tmp.path().join(".claude").is_dir());
}

#[test]
fn rewrite_on_second_call_overwrites() {
    let tmp = TempDir::new().unwrap();
    spawn_config::write_settings_json(tmp.path(), "/v1/hook").unwrap();
    spawn_config::write_settings_json(tmp.path(), "/v2/hook").unwrap();
    let body = std::fs::read_to_string(tmp.path().join(".claude/settings.json")).unwrap();
    assert!(!body.contains("/v1/"));
    assert!(body.contains("/v2/"));
}

#[test]
fn resolves_binary_path_from_env_var_when_set() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::set_var("GALVOLITE_HOOK_BIN", "/from/env/galvolite-hook");
    let path = spawn_config::resolve_hook_binary().unwrap();
    assert_eq!(path.to_string_lossy(), "/from/env/galvolite-hook");
    std::env::remove_var("GALVOLITE_HOOK_BIN");
}

#[test]
fn resolves_binary_path_from_current_exe_when_env_unset() {
    let _g = ENV_LOCK.lock().unwrap();
    std::env::remove_var("GALVOLITE_HOOK_BIN");
    let path = spawn_config::resolve_hook_binary().unwrap();
    let s = path.to_string_lossy();
    assert!(
        s.ends_with("galvolite-hook") || s.ends_with("galvolite-hook.exe"),
        "should derive from current_exe, got: {s}"
    );
}
