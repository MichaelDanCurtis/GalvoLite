# Plan 4: Signal parser + Enter-submit + AwaitingInput

**Status:** Scaffolded 2026-04-18, not yet planned in detail. Start a fresh session with full context and drive this plan through brainstorm → design → plan → execute.

**Goal:** Make the signal/ritual integration actually work with real `claude` sessions (not just with bash). Three defects exposed by Plan 2 manual smoke on 2026-04-18 at the HEAD of merge `5823b50`.

**Source:** See `docs/superpowers/plans/2026-04-18-plan-2-smoke-findings.md` for evidence + fix sketches.

---

## Section A — SignalPump parser survives claude TUI output

**Problem:** `src/signals.rs:parse_signal` requires the trimmed line to be exactly `[<KIND>]` or `[<KIND> key="val"]`. Claude Code v2.1.112 prefixes every reply line with U+25CF bullet + ANSI SGR escapes, so `[STATUS:WORKING]` arrives as `\x1b[32m● [STATUS:WORKING]\x1b[0m` and is silently dropped. **Zero signals land in DB during a real claude session.** This breaks state transitions, handoff auto-trigger, role badges, and everything else downstream.

**Fix sketch:**
- Strip ANSI SGR escapes (`\x1b\[[0-9;]*m`) before parsing.
- Loosen the leading-character check: find the first `[` on the stripped line rather than requiring it at position 0. Anything between line-start and `[` that is pure whitespace or single-non-bracket glyphs (bullet, `>`, `>>`) is allowed.
- Still require the bracketed segment to be the *only* bracketed content on the line (preserve the "signal alone on line" invariant the existing test asserts).
- Add regression fixture with actual recorded claude TUI bytes.

**Backend:**
- `src/signals.rs` (modify) — new `strip_ansi_sgr()` helper, update `parse_signal` to use it, expand tests.
- `src/signals.rs` tests (modify) — add `parses_signal_with_bullet_prefix`, `parses_signal_with_ansi_color`, `rejects_signal_followed_by_text`.

## Section B — Send-to-Session appends carriage return

**Problem:** Send-to-Session modal and rail send both write raw text bytes to the PTY via `{type:"input", data}` frames. No trailing `\r`. Message sits in claude's input buffer until operator focuses terminal and hits Enter.

**Fix sketch:**
- Send-to-Session modal Send handler: append `\r` to payload unconditionally.
- Rail paper-plane button: same.
- Raw xterm typing path (`term.onData`) — unchanged, Enter passes through naturally.

**Frontend:**
- `ui/src/components/modals/SendModal.tsx` (modify) — append `\r` before ws.send.
- `ui/src/components/SessionRail.tsx` or wherever the inline-send lives (modify) — same.
- Existing tests (modify/add) — assert `\r` terminator on the sent payload.

## Section C — AwaitingInput state for blocked sessions

**Problem:** State is binary working/idle + only transitions on parsed signals. When claude prompts `(y/n)` the session shows "working" forever. Autopilot will spin on a session that's actually blocked waiting for human input.

**Fix sketch:**
- Add `SessionState::AwaitingInput`.
- SignalPump watchdog: `tokio::time::sleep(Duration::from_secs(3))` race against `pty.next_line()`. If sleep wins AND last ~80 bytes of scrollback (ANSI-stripped) match prompt regex `[?>:] ?\r?$` → transition `Working → AwaitingInput`.
- Any subsequent PTY output bytes → back to `Working`.
- UI: new badge color + subtle pulse for AwaitingInput. Don't auto-answer — just surface.
- Autopilot policy: when AwaitingInput fires on an autopilot session, raise attention-needed (don't silently guess).

**Backend:**
- `src/db.rs` (modify) — new enum variant `SessionState::AwaitingInput`, DB string `"awaiting-input"`, migration if needed.
- `src/pty.rs` / `src/signals.rs` (modify) — watchdog loop alongside the existing parse loop; helper for ANSI-strip + prompt-shape regex.
- `src/session_manager.rs` (modify) — state transition writes.

**Frontend:**
- `ui/src/components/RoleBadge.tsx` or a new `StateBadge.tsx` (modify/create) — render AwaitingInput distinctly.
- `ui/src/components/SessionRow.tsx`, `ExpandedSessionView.tsx` (modify) — surface new state.

---

## Suggested task order

1. A first (unblocks everything).
2. B second (easy, high user-value).
3. C third (builds on A's ANSI stripping).

## When to invoke superpowers flow

Start a fresh session in `.worktrees/plan-4/`. Invoke `superpowers:brainstorming` if any of the fix sketches feel under-specified (especially C — prompt-shape detection is fiddly). Otherwise jump to `superpowers:planning-a-feature` to turn this into a task-by-task plan, then `superpowers:subagent-driven-development` or `superpowers:executing-plans` to implement.
