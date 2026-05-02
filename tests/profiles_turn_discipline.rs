//! Regression: every executor profile (IMPL, FIX) carries the
//! turn-discipline clause from Sub-plan 7. The clause is the contract
//! that makes orchestrator-as-router safe — every executor turn must
//! end in a state-machine-advancing tool call rather than prose.
//!
//! The ORCH and BRAINSTORM profile assertions live in their own tests
//! (Sub-plan 7 Tasks 8 and 9 respectively) so each task's red-then-green
//! cycle stays self-contained.

use galvolite::claude_md::{fix_profile, implementation_profile, orchestrator_profile};

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

#[test]
fn orch_profile_forbids_decisions() {
    let p = orchestrator_profile();
    assert!(
        p.contains("## Inputs"),
        "ORCH_PROFILE missing '## Inputs' heading"
    );
    assert!(
        p.contains("## You Do Not Decide"),
        "ORCH_PROFILE missing '## You Do Not Decide' heading"
    );
    assert!(
        p.contains("## Turn Discipline"),
        "ORCH_PROFILE missing '## Turn Discipline' heading"
    );
    assert!(
        p.contains(".galvolite/plan.md"),
        "ORCH_PROFILE missing '.galvolite/plan.md' reference"
    );
    assert!(
        p.contains(".galvolite/directive.md"),
        "ORCH_PROFILE missing '.galvolite/directive.md' reference"
    );
    assert!(
        p.contains("only humans edit"),
        "ORCH_PROFILE missing 'only humans edit' constraint"
    );
    assert!(
        p.contains("MUST call `ask_operator`"),
        "ORCH_PROFILE missing 'MUST call `ask_operator`' directive"
    );
    assert!(
        p.contains("explicitly forbidden"),
        "ORCH_PROFILE missing 'explicitly forbidden' clause"
    );
    assert!(
        p.contains("Choosing your own phase order"),
        "ORCH_PROFILE missing 'Choosing your own phase order' prohibition"
    );
    assert!(
        p.contains("Never prose-only"),
        "ORCH_PROFILE missing 'Never prose-only' turn-discipline closer"
    );
}
