#[test]
fn orch_profile_documents_rec_routing_rule() {
    let p = galvolite::claude_md::orchestrator_profile();
    assert!(p.contains("Overseer recommendations"), "ORCH_PROFILE missing Overseer section");
    assert!(p.contains("≤ 3"), "ORCH_PROFILE missing inline-vs-fanout threshold");
    assert!(p.contains("spawn_child"), "ORCH_PROFILE missing spawn_child fan-out instruction");
}
