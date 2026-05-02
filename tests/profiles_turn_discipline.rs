//! Regression: every executor profile (IMPL, FIX) carries the
//! turn-discipline clause from Sub-plan 7. The clause is the contract
//! that makes orchestrator-as-router safe — every executor turn must
//! end in a state-machine-advancing tool call rather than prose.
//!
//! The ORCH and BRAINSTORM profile assertions live in their own tests
//! (Sub-plan 7 Tasks 8 and 9 respectively) so each task's red-then-green
//! cycle stays self-contained.

use galvolite::claude_md::{fix_profile, implementation_profile};

#[test]
fn impl_profile_requires_turn_discipline() {
    let p = implementation_profile();
    assert!(
        p.contains("## Turn Discipline"),
        "IMPL_PROFILE missing '## Turn Discipline' heading"
    );
    assert!(
        p.contains("Never end a turn with prose only"),
        "IMPL_PROFILE missing the prose-only prohibition"
    );
    // The five listed terminal calls.
    assert!(p.contains("plan_check_step"));
    assert!(p.contains("status_report"));
    assert!(p.contains("ask_operator"));
    assert!(p.contains("request_attention"));
    assert!(p.contains("write_handoff"));
}

#[test]
fn fix_profile_requires_turn_discipline() {
    let p = fix_profile();
    assert!(
        p.contains("## Turn Discipline"),
        "FIX_PROFILE missing '## Turn Discipline' heading"
    );
    assert!(
        p.contains("Never end a turn with prose only"),
        "FIX_PROFILE missing the prose-only prohibition"
    );
    assert!(p.contains("plan_check_step"));
    assert!(p.contains("status_report"));
}
