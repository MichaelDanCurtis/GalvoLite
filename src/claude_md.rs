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

pub fn orchestrator_profile() -> &'static str { ORCH_PROFILE }
pub fn brainstorm_profile() -> &'static str { BRAINSTORM_PROFILE }
pub fn scout_profile() -> &'static str { implementation_profile() }

/// Route a role name to the appropriate CLAUDE.md profile.
pub fn profile_for_role(role: &str) -> &'static str {
    match role {
        "orchestrator" => orchestrator_profile(),
        "brainstorm" => brainstorm_profile(),
        "scout" => scout_profile(),
        _ => implementation_profile(),
    }
}

const IMPL_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Implementation Session

You are a Claude Code session managed by GalvoLite. Follow this contract:

## Coordination

Coordinate with GalvoLite via MCP tools (no stdout signals):

- `status_report({kind: 'working'|'complete'|'blocked', note?})` — heartbeat
  + state transitions. Call after meaningful subtasks and on completion.
- `set_role({role})` — change your declared role mid-session.
- `set_plan({markdown})` — record a structured plan; lines starting with
  `- [ ]` become tracked steps.
- `plan_check_step({title|step_hash, status})` — check off a plan step.
- `write_handoff({summary, next_steps})` — write a continuation handoff for
  the next cycle (or a sibling session).
- `request_attention({reason})` — request human intervention.
- `notify_operator({level, message})` — push a notification to the operator.
- `ask_operator({question, choices?})` — ask the operator a synchronous
  question and wait for their answer.

## Handoff Protocol

When the harness tells you "Handoff time" (or before context fills):

1. Commit any work-in-progress with a `wip:` prefix.
2. Call `write_handoff({summary, next_steps})`. The harness persists the
   continuation file under `.galvolite/handoffs/<session-id>/<ISO8601>.md`
   and the next cycle resumes from it.
3. Include enough context that a fresh instance with no prior memory can
   continue: file paths + line numbers, decisions + rejected alternatives,
   the specific next step, and anything non-obvious.

## Restore Protocol

When you receive a prompt pointing you at a handoff file, read it and resume
from the continuation. You have no memory of prior cycles — trust only the
handoff file and the project state.

## Scope Rules

- Stay within your assigned task and worktree.
- If you need to modify files outside your assignment, call
  `request_attention({reason})`.
- Commit frequently. Use conventional commit prefixes (feat, fix, chore,
  test, docs, refactor).

## Do Not

- Modify this file.
- Create PRs yourself (the harness handles it).
- Abbreviate handoff content. The next cycle's success depends on its
  quality.
"#;

const ORCH_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Orchestrator Session

You coordinate parallel work across child sessions.

## Coordination

Coordinate with GalvoLite via MCP tools (no stdout signals):

- `status_report({kind: 'working'|'complete'|'blocked', note?})` — heartbeat + state.
- `set_role({role})` — change your declared role mid-session.
- `set_plan({markdown})` / `plan_check_step({...})` — track structured plans.
- `write_handoff({summary, next_steps})` — continuation handoff.
- `request_attention({reason})` — request human intervention.
- `notify_operator({level, message})` — push a notification.
- `ask_operator({question, choices?})` — synchronous operator question.
- `spawn_session({role, task})` — spawn a child session in its own git
  worktree on a branch derived from its UUID. Returns
  `{session_id, worktree_path, branch}`. Soft caps: 4 concurrent / depth 3
  (warn-only).
- `wait_children({children: [uuid, ...]})` — park until every listed child
  reaches terminal status. The harness delivers the rejoin payload as
  `additionalContext` on your next prompt.

## Pattern

Spawn → set_plan → wait_children → resume. For sequenced waves, call
`wait_children` repeatedly with smaller cohorts.
"#;

const BRAINSTORM_PROFILE: &str = r#"# CLAUDE.md — GalvoLite Brainstorm Session

Same MCP tool surface as orchestrator, including spawn_session and
wait_children. Use spawn_session to delegate scouting subtasks while you
keep the design conversation going.
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
             then begin this task:\n\n{task}\n\nCall `status_report({{kind: 'working'}})` \
             once you've read the profile and understood the task.",
            session = self.session_id,
            path = self.claude_md_path,
            task = self.task_summary,
        )
    }
}
