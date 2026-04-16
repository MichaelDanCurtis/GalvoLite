# GalvoLite

Solo continuity engine for Claude Code. See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec.

## Plan 1a Status: Ritual Engine (CLI)

What works:
- Register projects, spawn long-lived Claude sessions via pty, parse signal protocol
- Drive the save→/clear→restore ritual end-to-end
- Signal + session state persisted to SQLite
- Integration test proves the ritual with a mock-claude fixture

## Quickstart

```bash
cargo build
./target/debug/galvolite --data-dir /tmp/gl add-project /path/to/your/repo --name MyProj
./target/debug/galvolite --data-dir /tmp/gl projects
./target/debug/galvolite --data-dir /tmp/gl start-session --project <UUID> --task "Refactor the foo"
# In another terminal:
./target/debug/galvolite --data-dir /tmp/gl status <SESSION-UUID>
./target/debug/galvolite --data-dir /tmp/gl send <SESSION-UUID> "some input" --newline
./target/debug/galvolite --data-dir /tmp/gl handoff <SESSION-UUID>
```

## Running tests

```bash
cargo test --workspace
```

## What's next

Plan 1b: Tauri desktop shell wrapping this ritual engine with a dashboard UI.
