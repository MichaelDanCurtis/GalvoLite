use galvolite::hooks::marker::detect_rejoin_marker;

#[test]
fn detects_well_formed_marker() {
    let id = "0a1b2c3d-4e5f-6789-abcd-ef0123456789";
    let s = format!("[GALVOLITE:REJOIN {}]", id);
    assert_eq!(detect_rejoin_marker(&s).map(|u| u.to_string()), Some(id.into()));
}

#[test]
fn ignores_unrelated_text() {
    assert!(detect_rejoin_marker("hello world").is_none());
    assert!(detect_rejoin_marker("[GALVOLITE:REJOIN not-a-uuid]").is_none());
    assert!(detect_rejoin_marker("[OTHER:REJOIN 0a1b2c3d-4e5f-6789-abcd-ef0123456789]").is_none());
}

#[test]
fn allows_trailing_whitespace() {
    let id = "0a1b2c3d-4e5f-6789-abcd-ef0123456789";
    let s = format!("[GALVOLITE:REJOIN {}]   \n", id);
    assert_eq!(detect_rejoin_marker(&s).map(|u| u.to_string()), Some(id.into()));
}
