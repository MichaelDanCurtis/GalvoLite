use galvolite::state::{EventInput, EventOutcome, SessionState, Source};
use std::str::FromStr;

#[test]
fn all_seven_variants_round_trip() {
    let cases = [
        (SessionState::Starting,       "starting"),
        (SessionState::Idle,           "idle"),
        (SessionState::Working,        "working"),
        (SessionState::AwaitingInput,  "awaiting_input"),
        (SessionState::NeedsAttention, "needs_attention"),
        (SessionState::Error,          "error"),
        (SessionState::Ended,          "ended"),
    ];
    for (state, s) in cases {
        assert_eq!(state.to_string(), s);
        assert_eq!(SessionState::from_str(s).unwrap(), state);
    }
}

#[test]
fn unknown_string_is_error() {
    assert!(SessionState::from_str("complete").is_err(),
        "v1 'complete' must not silently parse as a v2 state");
    assert!(SessionState::from_str("").is_err());
    assert!(SessionState::from_str("WORKING").is_err(),
        "casing matters — DB strings are lowercase canonical");
}

#[test]
fn is_terminal_returns_true_for_error_and_ended_only() {
    assert!(SessionState::Error.is_terminal());
    assert!(SessionState::Ended.is_terminal());
    for s in [
        SessionState::Starting,
        SessionState::Idle,
        SessionState::Working,
        SessionState::AwaitingInput,
        SessionState::NeedsAttention,
    ] {
        assert!(!s.is_terminal(), "{s} must not be terminal");
    }
}

#[test]
fn source_round_trip() {
    for (src, s) in [
        (Source::Mcp,      "mcp"),
        (Source::Hook,     "hook"),
        (Source::Operator, "operator"),
        (Source::System,   "system"),
    ] {
        assert_eq!(src.to_string(), s);
        assert_eq!(Source::from_str(s).unwrap(), src);
    }
}

#[test]
fn event_input_payload_is_json_value() {
    let e = EventInput {
        source: Source::Hook,
        kind: "hook.UserPromptSubmit".into(),
        payload: serde_json::json!({ "raw": "user typed something" }),
        dedup_key: Some("h-1".into()),
    };
    assert_eq!(e.kind, "hook.UserPromptSubmit");
    assert_eq!(e.payload["raw"], "user typed something");
}

#[test]
fn event_outcome_carries_event_id_and_states() {
    let o = EventOutcome {
        event_id: 42,
        prev_state: Some(SessionState::Working),
        next_state: SessionState::Idle,
        deduped: false,
    };
    assert_eq!(o.event_id, 42);
    assert_eq!(o.prev_state, Some(SessionState::Working));
    assert_eq!(o.next_state, SessionState::Idle);
    assert!(!o.deduped);
}
