use galvolite::signals::{parse_signal, Signal};

#[test]
fn parses_status_working() {
    assert_eq!(parse_signal("[STATUS:WORKING]"), Some(Signal::StatusWorking));
}

#[test]
fn parses_status_complete() {
    assert_eq!(parse_signal("[STATUS:COMPLETE]"), Some(Signal::StatusComplete));
}

#[test]
fn parses_handoff_ready() {
    assert_eq!(parse_signal("[HANDOFF:READY]"), Some(Signal::HandoffReady));
}

#[test]
fn parses_handoff_blocked_with_reason() {
    let s = parse_signal(r#"[HANDOFF:BLOCKED reason="file missing"]"#);
    assert_eq!(s, Some(Signal::HandoffBlocked { reason: "file missing".into() }));
}

#[test]
fn parses_attention_needed() {
    let s = parse_signal(r#"[ATTENTION:NEEDED reason="ambiguous"]"#);
    assert_eq!(s, Some(Signal::AttentionNeeded { reason: "ambiguous".into() }));
}

#[test]
fn parses_attention_context() {
    assert_eq!(parse_signal("[ATTENTION:CONTEXT]"), Some(Signal::AttentionContext));
}

#[test]
fn parses_plan_ready() {
    assert_eq!(parse_signal("[PLAN:READY]"), Some(Signal::PlanReady));
}

#[test]
fn parses_seed_new() {
    let s = parse_signal(r#"[SEED:NEW id="SEED-007"]"#);
    assert_eq!(s, Some(Signal::SeedNew { id: "SEED-007".into() }));
}

#[test]
fn ignores_non_signal_lines() {
    assert_eq!(parse_signal("regular output"), None);
    assert_eq!(parse_signal(""), None);
    assert_eq!(parse_signal("[NOT:A:SIGNAL]"), None);
}

#[test]
fn requires_signal_to_be_alone_on_line() {
    assert_eq!(parse_signal("prefix [STATUS:WORKING] suffix"), None);
    assert_eq!(parse_signal("  [STATUS:WORKING]"), Some(Signal::StatusWorking),
        "leading whitespace is ok");
    assert_eq!(parse_signal("[STATUS:WORKING]  "), Some(Signal::StatusWorking),
        "trailing whitespace is ok");
}

#[test]
fn reason_handles_escaped_quote() {
    let s = parse_signal(r#"[ATTENTION:NEEDED reason="he said \"hi\""]"#);
    assert_eq!(s, Some(Signal::AttentionNeeded { reason: r#"he said "hi""#.into() }));
}

#[test]
fn signal_kind_string_for_db() {
    assert_eq!(Signal::StatusWorking.kind(), "STATUS:WORKING");
    assert_eq!(Signal::HandoffReady.kind(), "HANDOFF:READY");
    assert_eq!(
        Signal::AttentionNeeded { reason: "x".into() }.kind(),
        "ATTENTION:NEEDED"
    );
}

use galvolite::signals::LineBuffer;

#[test]
fn line_buffer_accumulates_partial_reads() {
    let mut buf = LineBuffer::new();
    let a = buf.feed("hello");
    assert!(a.is_empty());
    let b = buf.feed(" world\n");
    assert_eq!(b, vec!["hello world"]);
}

#[test]
fn line_buffer_multiple_lines_in_one_feed() {
    let mut buf = LineBuffer::new();
    let lines = buf.feed("a\nb\nc\n");
    assert_eq!(lines, vec!["a", "b", "c"]);
}

#[test]
fn line_buffer_handles_crlf() {
    let mut buf = LineBuffer::new();
    let lines = buf.feed("one\r\ntwo\r\n");
    assert_eq!(lines, vec!["one", "two"]);
}

#[test]
fn line_buffer_keeps_trailing_incomplete_line() {
    let mut buf = LineBuffer::new();
    let a = buf.feed("done\nstill here");
    assert_eq!(a, vec!["done"]);
    let b = buf.feed("\n");
    assert_eq!(b, vec!["still here"]);
}
