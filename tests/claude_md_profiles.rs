use galvolite::claude_md::profile_for_role;

#[test]
fn orchestrator_profile_mentions_spawn_tools() {
    let p = profile_for_role("orchestrator");
    assert!(p.contains("spawn_session"), "orchestrator must know spawn_session");
    assert!(p.contains("wait_children"), "orchestrator must know wait_children");
}

#[test]
fn brainstorm_profile_mentions_spawn_tools() {
    let p = profile_for_role("brainstorm");
    assert!(p.contains("spawn_session"));
    assert!(p.contains("wait_children"));
}

#[test]
fn implementation_profile_omits_spawn_tools() {
    let p = profile_for_role("implementation");
    assert!(!p.contains("spawn_session"), "leaf roles should not advertise spawn");
}
