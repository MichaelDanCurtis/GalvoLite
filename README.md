# GalvoLite

Solo continuity engine for Claude Code. See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec.

## Status

- **Plan 1a** (Ritual Engine CLI): DONE — 44 tests, clippy clean
- **Plan 1b** (Tauri Desktop Shell + Dashboard): DONE

## Quick start

### Desktop app (Tauri)

```bash
cd src-tauri
cargo tauri dev
```

Launches Vite on :5173, axum on :3001, and a native webview window.

### HTTP API server only

```bash
cargo run -- serve
# API at http://localhost:3001; static dashboard served from ./ui/dist if present
```

### Development (frontend only, against a running API)

```bash
# Terminal 1: API server
cargo run -- serve --port 3001

# Terminal 2: Vite dev server with HMR
cd ui && npm run dev
# Open http://localhost:5173
```

### CLI (legacy single-session interactive mode)

```bash
cargo run -- --data-dir /tmp/gl add-project /path/to/your/repo --name MyProj
cargo run -- --data-dir /tmp/gl projects
cargo run -- --data-dir /tmp/gl start-session --project <UUID> --task "Refactor the foo"
cargo run -- --data-dir /tmp/gl status <SESSION-UUID>
```

`send`/`handoff`/`kill` work only in-process (they delegate to an in-memory `SessionManager`). For cross-process control use the HTTP API exposed by `serve`.

## Architecture

```
Tauri (native shell)
  └─ React + xterm.js frontend (ui/)
         │ HTTP + WebSocket
  Rust core (tokio + axum)
    ├── SessionManager    (pty, signals, ritual driver)
    ├── SQLite            (operational state)
    └── N long-lived Claude Code processes
```

## Running tests

```bash
cargo test --workspace
```
