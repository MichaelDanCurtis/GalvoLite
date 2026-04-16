//! A scriptable stand-in for the real `claude` CLI, used by integration tests.
//!
//! Behavior:
//!   * Reads lines from stdin.
//!   * If an input line matches a trigger pattern from an internal script,
//!     emits the corresponding response lines to stdout (including signals).
//!   * Supports the `/clear` keystroke: on seeing `/clear`, resets an internal
//!     "cycle counter" and emits a new [STATUS:WORKING] after next input.
//!
//! The script is hardcoded for the Plan 1a integration test scenario:
//!
//!   cycle 1:
//!     on first "begin" prompt → emit [STATUS:WORKING]
//!     on "Handoff time" prompt → write handoff file, emit [HANDOFF:READY]
//!   after /clear:
//!     on next prompt (the restore prompt) → emit [STATUS:WORKING]
//!
//! Handoff file path comes from the GALVO_MOCK_HANDOFF_DIR env var + session id
//! extracted from the restore prompt (simple parse).

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

fn main() {
    let mut cycle = 1u32;
    let mut handoff_pending = false;

    // Flush stdout often.
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    writeln!(out, "mock-claude v1 (cycle {cycle}) ready").ok();
    out.flush().ok();

    let stdin = std::io::stdin();
    let reader = BufReader::new(stdin.lock());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle /clear
        if line.trim() == "/clear" {
            cycle += 1;
            writeln!(out, "(mock-claude cleared; now cycle {cycle})").ok();
            out.flush().ok();
            handoff_pending = false;
            continue;
        }

        // Begin → heartbeat
        if line.contains("begin") || line.contains("GalvoLite session") {
            writeln!(out, "mock: starting task").ok();
            writeln!(out, "[STATUS:WORKING]").ok();
            out.flush().ok();
            continue;
        }

        // Handoff trigger
        if line.contains("Handoff time") {
            // Write a minimal valid handoff file.
            let handoff_dir = std::env::var("GALVO_MOCK_HANDOFF_DIR")
                .expect("GALVO_MOCK_HANDOFF_DIR must be set");
            let session_id = std::env::var("GALVO_MOCK_SESSION_ID")
                .expect("GALVO_MOCK_SESSION_ID must be set");
            let when = chrono_like_timestamp();
            let dir = PathBuf::from(&handoff_dir).join(&session_id);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{when}.md"));
            std::fs::write(&path, MINIMAL_HANDOFF).unwrap();
            writeln!(out, "mock: wrote handoff at {}", path.display()).ok();
            writeln!(out, "[HANDOFF:READY]").ok();
            out.flush().ok();
            handoff_pending = true;
            continue;
        }

        // Restore prompt → heartbeat for new cycle
        if (line.contains("Read") && line.contains(".galvolite/handoffs")) || line.contains("next cycle") {
            writeln!(out, "mock: restored (cycle {cycle})").ok();
            writeln!(out, "[STATUS:WORKING]").ok();
            out.flush().ok();
            continue;
        }

        // Everything else — echo back
        let _ = handoff_pending;
        writeln!(out, "mock: echoed '{line}'").ok();
        out.flush().ok();
    }
}

fn chrono_like_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    format!("mock-{secs}")
}

const MINIMAL_HANDOFF: &str = r#"# Handoff: mock

## Task Assignment
Execute the integration test scenario.

## Current State
Emitted STATUS:WORKING at cycle start. Task is a mock and requires no real work.

## Decisions Made
- Chose to emit minimal handoff to satisfy validator (rejected alternative: richer body, unnecessary for test).

## Modified Files
- (none — mock)

## Open Questions / Blockers
None.

## Next Steps
1. Resume in next cycle and emit STATUS:WORKING

## Critical Context
- This is a mock. The real Claude writes much richer handoffs.

## Git State
- Branch: main
- Last commit: (mock)

## Continuation Prompt
You are the restored cycle of a GalvoLite integration test session. The previous cycle ran a mock-claude binary. Upon receiving your restore prompt, emit [STATUS:WORKING] on its own line to confirm the ritual completed. Do no other work. This prompt is intentionally long enough to pass the validator's 50-character minimum.
"#;
