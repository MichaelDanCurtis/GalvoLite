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

#[test]
fn scout_profile_is_read_only_and_omits_spawn() {
    let p = profile_for_role("scout");
    assert!(p.contains("Scout"), "scout profile must self-identify");
    assert!(p.contains("write_handoff"), "scout must know write_handoff");
    // Scout is a leaf role: no spawn, no wait_children, no editing/committing.
    assert!(!p.contains("spawn_session"), "scout must not advertise spawn");
    assert!(!p.contains("wait_children"), "scout must not advertise wait_children");
}

#[test]
fn fix_profile_requires_repro_first_and_omits_spawn() {
    let p = profile_for_role("fix");
    assert!(p.contains("Fix"), "fix profile must self-identify");
    assert!(p.contains("Reproduce"), "fix profile must require reproduction");
    assert!(p.contains("failing test"), "fix profile must require failing test before fix");
    // Leaf role.
    assert!(!p.contains("spawn_session"), "fix must not advertise spawn");
    assert!(!p.contains("wait_children"), "fix must not advertise wait_children");
}

#[test]
fn fix_role_routes_to_distinct_profile() {
    // `fix` was previously absent from match arms and fell through to the
    // implementation default. Ensure routing now picks up the dedicated body.
    let fix = profile_for_role("fix");
    let impl_ = profile_for_role("implementation");
    assert_ne!(fix, impl_, "fix profile must differ from implementation default");
}
