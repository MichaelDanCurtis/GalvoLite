# Plan 2 manual smoke findings (2026-04-18)

Ran full 10-item Task-24 smoke checklist driving the Chrome dashboard against a real `claude` PTY session (not bash). CRUD parity passed. Signal/ritual integration with the interactive `claude` TUI is broken in three distinct ways. All three are Plan 4 scope — Plan 2 ships as-is.

## Finding 1: SignalPump rejects every signal emitted by `claude`'s TUI

**Symptom:** Claude visibly emits `[STATUS:WORKING]` and `[STATUS:COMPLETE]` in the terminal, but zero signals land in the DB. Session state stays "working" forever. No state-driven UI (rail badge, handoff auto-trigger, role updates) fires.

**Root cause:** `src/signals.rs:48-63` requires the trimmed line to be exactly `[<KIND>]` or `[<KIND> key="val"]`. Claude Code's TUI prefixes each reply line with `● ` (U+25CF bullet) and wraps everything in ANSI SGR escapes, so what lands in `LineBuffer` looks like `\x1b[32m● [STATUS:WORKING]\x1b[0m` — parser returns `None`. With bash as the session command, raw stdout prints `[STATUS:WORKING]\n` directly and parses fine. That's what produced the false-positive bash smoke pass earlier.

**Fix sketch:** Strip ANSI SGR escapes before parsing (there's no CSI cursor-movement to worry about on the signal line itself — claude flushes the line as a single write). Also loosen the leading-character check to accept any non-bracket prefix up to the first `[` on the line, so bullets/quoting/indentation don't matter. Add a regression test fixture with actual recorded claude output.

**Blast radius:** Entire signal protocol (STATUS, ATTENTION, HANDOFF, PLAN, SEED, ROLE) unusable with real claude. This is the single biggest blocker to GalvoLite being useful for its stated purpose.

## Finding 2: Send-to-Session modal ships text without a carriage return

**Symptom:** Typing "Say hi" in the Send-to-Session modal and clicking Send inserts `Say hi` into claude's input buffer but does not submit. Claude sits idle with the text pending until the user manually focuses the terminal and presses Enter. Rail input textbox has the same behavior.

**Root cause:** The frontend writes raw text bytes via the terminal WebSocket's `Input` frame. It does not append `\r` (CR) or `\n`. For a shell this is maybe defensible (user might want multi-line input), but for the explicit "Send to session" affordance it's a footgun — the whole point is "deliver this message NOW."

**Fix sketch:** Send-to-Session modal's Send handler should append `\r` to the payload unconditionally. Rail send (`uid_133_9` paper-plane) same. Keep raw terminal-focus typing as-is (that already respects Enter semantics via xterm).

## Finding 3: No "session is waiting for input" detection

**Symptom:** After claude replies with a prompt like "proceed? (y/n)", the rail shows "working" forever. GalvoLite has no way to tell a claude that's actively thinking apart from a claude that's blocked on user input. Autopilot makes this worse — an autopilot session blocked on an interactive prompt will look busy forever.

**Root cause:** State model is binary (working/idle) and only transitions on parsed signals. There's no third state "awaiting-input" and no heuristic for detecting quiescence (no PTY bytes in N seconds + a trailing prompt shape like `? `, `> `, `(y/n)`).

**Fix sketch for Plan 4:**
- Add `SessionState::AwaitingInput` variant.
- SignalPump watchdog: if no PTY output for 3s AND last ~80 bytes match a prompt pattern (regex over stripped ANSI: `[?>:] *$`), transition `Working → AwaitingInput`.
- Any subsequent output bytes transition back to `Working`.
- UI: distinct badge color + attention pulse on the rail row.
- Autopilot: when AwaitingInput fires, either auto-answer "y" (risky) or break into attention-needed state so the operator notices.

## Additional observations (not Plan 4 scope)

- Fs-list error messages now scrub paths (committed 75b5ef7 during this branch's code-review fixes) ✅
- Startup reconciliation of orphaned `working` rows → `Error` committed 4e6ea7b ✅
- `GALVOLITE_KEEP_API` dev keepalive works (committed 7da4a46, gated debug-only in f3afbd8) ✅
- Trust-folder prompt from claude (`Quick safety check:`) appears on first run in a new directory. Our session-prompt injection should probably wait for the prompt to clear before firing, otherwise the task gets sent into the trust-confirm dialog buffer. Minor — user can press Enter.
- Task-24 smoke items 1-3, 6-10 all pass with real claude. Items 4, 5, 7 (role transitions) are blocked by Finding 1.
