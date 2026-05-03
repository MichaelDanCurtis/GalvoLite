use galvolite::overseer::observer::ContextSnapshot;
use galvolite::overseer::recommender::{pack_prompt, parse_kimi_response};

#[test]
fn pack_prompt_includes_recent_event_kinds() {
    let mut snap = ContextSnapshot::default();
    snap.events.push(galvolite::overseer::observer::EventRecord {
        session_id: uuid::Uuid::nil(), kind: "mcp.status_report".into(),
        source: "mcp".into(), ts: "t".into(), payload: serde_json::json!({"kind":"stuck"}),
    });
    let (sys, user) = pack_prompt(&snap);
    assert!(sys.contains("Overseer"));
    assert!(user.contains("mcp.status_report"));
}

#[test]
fn pack_prompt_caps_below_8k_chars() {
    let mut snap = ContextSnapshot::default();
    snap.plan_md = Some("x".repeat(20_000));
    let (_, user) = pack_prompt(&snap);
    assert!(user.len() < 8_192, "len={}", user.len());
}

#[test]
fn parse_kimi_well_formed_returns_recs() {
    let json = r#"{
      "recommendations": [
        {"kind":"stuck","severity":"warn","title":"t","body":"b","rationale":"r","evidence_refs":[]}
      ]
    }"#;
    let recs = parse_kimi_response(json).unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].title, "t");
}

#[test]
fn parse_kimi_malformed_returns_empty() {
    assert!(parse_kimi_response("not json").unwrap_or_default().is_empty());
}

#[test]
fn parse_kimi_extracts_from_fenced_block() {
    let s = "Sure, here:\n```json\n{\"recommendations\":[]}\n```";
    let recs = parse_kimi_response(s).unwrap();
    assert!(recs.is_empty());
}
