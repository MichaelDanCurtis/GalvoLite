use galvolite::claude_md::{implementation_profile, InjectionPrompt};

#[test]
fn implementation_profile_has_required_sections() {
    let md = implementation_profile();
    assert!(md.contains("# CLAUDE.md"));
    assert!(md.contains("[STATUS:WORKING]"));
    assert!(md.contains("[STATUS:COMPLETE]"));
    assert!(md.contains("[HANDOFF:READY]"));
    assert!(md.contains("[ATTENTION:NEEDED"));
    assert!(md.contains("[ATTENTION:CONTEXT]"));
    assert!(md.contains(".galvolite/handoffs"));
    assert!(md.contains("Continuation Prompt"));
}

#[test]
fn injection_prompt_references_profile_and_task() {
    let p = InjectionPrompt {
        task_summary: "Build the thing",
        session_id: "abc-123",
        claude_md_path: "/tmp/CLAUDE.md",
    };
    let out = p.render();
    assert!(out.contains("Build the thing"));
    assert!(out.contains("abc-123"));
    assert!(out.contains("/tmp/CLAUDE.md"));
    assert!(out.contains("Read"), "prompt must instruct Claude to read the profile");
}
