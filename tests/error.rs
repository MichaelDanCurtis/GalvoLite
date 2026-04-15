use galvolite::error::Error;

#[test]
fn error_display_contains_context() {
    let err = Error::SignalParse("bad line".to_string());
    let s = format!("{err}");
    assert!(s.contains("signal parse"), "unexpected display: {s}");
    assert!(s.contains("bad line"), "unexpected display: {s}");
}
