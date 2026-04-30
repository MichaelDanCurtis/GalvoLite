use galvolite::error::Error;

#[test]
fn terminal_state_variant_displays_with_session_and_kind() {
    let e = Error::TerminalState {
        session_id: "abc-123".into(),
        kind: "hook.UserPromptSubmit".into(),
    };
    let msg = e.to_string();
    assert!(msg.contains("abc-123"), "session id missing in: {msg}");
    assert!(msg.contains("UserPromptSubmit"), "kind missing in: {msg}");
    assert!(
        msg.contains("terminal"),
        "should mention terminal state: {msg}"
    );
}

#[test]
fn event_payload_variant_wraps_serde_error() {
    let inner: serde_json::Error =
        serde_json::from_str::<serde_json::Value>("not-json").unwrap_err();
    let e = Error::EventPayload(inner);
    let msg = e.to_string();
    assert!(
        msg.to_lowercase().contains("payload") || msg.to_lowercase().contains("json"),
        "should mention payload/json: {msg}"
    );
}
