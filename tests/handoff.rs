use chrono::{TimeZone, Utc};
use galvolite::handoff::{handoff_path, validate_handoff_content, HandoffDir};
use tempfile::TempDir;

#[test]
fn path_is_under_session_subdir() {
    let tmp = TempDir::new().unwrap();
    let dir = HandoffDir::new(tmp.path().to_path_buf());

    let session_id = uuid::Uuid::new_v4();
    let when = Utc.with_ymd_and_hms(2026, 4, 15, 14, 32, 0).unwrap();

    let p = handoff_path(&dir, session_id, when);
    let s = p.to_string_lossy();

    assert!(s.contains(&format!("{}", session_id)));
    assert!(s.ends_with("2026-04-15T14-32-00.md"));
    assert!(s.starts_with(&tmp.path().to_string_lossy().to_string()));
}

#[test]
fn validate_accepts_well_formed_handoff() {
    let body = r#"# Handoff: 2026-04-15 14:32

## Task Assignment
Build the thing.

## Current State
Files touched: src/foo.rs:10-42

## Decisions Made
- Chose X over Y because …

## Modified Files
- src/foo.rs

## Open Questions / Blockers
None.

## Next Steps
1. Wire up bar

## Critical Context
- The legacy foo hates X

## Git State
- Branch: main
- Last commit: abc1234

## Continuation Prompt
You are session XYZ. Read .galvolite/handoffs/XYZ/prev.md. Resume by doing step 1 from Next Steps. The critical context is that legacy foo hates X so avoid touching it directly.
"#;
    assert!(validate_handoff_content(body).is_ok());
}

#[test]
fn validate_rejects_missing_continuation_prompt() {
    let body = "# Handoff\n## Task Assignment\nfoo\n";
    let err = validate_handoff_content(body).unwrap_err();
    assert!(format!("{err}").contains("Continuation Prompt"));
}

#[test]
fn validate_rejects_placeholder_content() {
    let body = r#"# Handoff

## Task Assignment
TODO

## Continuation Prompt
TBD
"#;
    let err = validate_handoff_content(body).unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("TODO") || s.contains("TBD") || s.contains("placeholder"));
}
