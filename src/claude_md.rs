//! CLAUDE.md profile templates + injection-prompt rendering.
//!
//! At session start, GalvoLite:
//!   1. Writes the appropriate profile to the session's worktree as `CLAUDE.md`
//!      (or a unique path inside `.galvolite/` for test isolation).
//!   2. Sends an initial prompt telling Claude to read the profile and begin.

/// The Implementation profile. See design spec §10.2.
pub fn implementation_profile() -> &'static str {
    IMPL_PROFILE
}

const IMPL_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Implementation Session

You are a Claude Code session managed by GalvoLite. Follow this contract:

## Signals

Emit these on their own line (no surrounding text, no code fences):

- `[STATUS:WORKING]` — heartbeat every 10-15 tool calls or after a meaningful subtask.
- `[STATUS:COMPLETE]` — task done, tests pass, branch pushed, working tree clean.
- `[ATTENTION:NEEDED reason="<brief>"]` — needs human input. Provide context on subsequent lines.
- `[ATTENTION:CONTEXT]` — you notice your context is getting large; handoff soon.
- `[HANDOFF:READY]` — after writing the handoff file and validating it.
- `[HANDOFF:BLOCKED reason="<brief>"]` — cannot complete the handoff.

## Handoff Protocol

When the harness tells you "Handoff time":

1. Commit any work-in-progress with a `wip:` prefix.
2. Append a timestamped entry to `.galvolite/handoffs/<session-id>/<ISO8601>.md`.
   Include these sections, each filled (no placeholders):
   - Task Assignment
   - Current State (file paths, line numbers)
   - Decisions Made (with rejected alternatives and why)
   - Modified Files
   - Open Questions / Blockers
   - Next Steps (numbered)
   - Critical Context
   - Git State
   - **Continuation Prompt** — a verbose, self-contained briefing for a fresh
     instance. The next cycle reads ONLY this prompt. Err on the side of more
     context. Include exact file paths and line numbers, decisions, rejected
     alternatives, the specific next step, and anything non-obvious.
3. Emit `[HANDOFF:READY]` on its own line.
4. Wait. The harness will send `/clear` and then a restore prompt.

## Restore Protocol

When you receive a prompt pointing you at a handoff file, read it and resume
from the Continuation Prompt section. You have no memory of prior cycles —
trust only the handoff file and the project state.

## Scope Rules

- Stay within your assigned task and worktree.
- If you need to modify files outside your assignment, emit `[ATTENTION:NEEDED]`.
- Commit frequently. Use conventional commit prefixes (feat, fix, chore, test, docs, refactor).

## Do Not

- Emit signals inside code blocks or quoted text.
- Modify this file.
- Create PRs yourself (the harness handles it).
- Abbreviate handoff content. The next cycle's success depends on its quality.
"#;

#[derive(Debug, Clone)]
pub struct InjectionPrompt<'a> {
    pub task_summary: &'a str,
    pub session_id: &'a str,
    pub claude_md_path: &'a str,
}

impl<'a> InjectionPrompt<'a> {
    pub fn render(&self) -> String {
        format!(
            "You are GalvoLite session {session}. Read `{path}` for your behavioral contract, \
             then begin this task:\n\n{task}\n\nEmit `[STATUS:WORKING]` once you've read the \
             profile and understood the task.",
            session = self.session_id,
            path = self.claude_md_path,
            task = self.task_summary,
        )
    }
}
