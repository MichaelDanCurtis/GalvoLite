# GalvoLite

Solo continuity engine for Claude Code. See `docs/superpowers/specs/2026-04-15-galvolite-design.md` for the product spec.

## Status

- **Plan 1a** (Ritual Engine CLI): DONE — 44 tests, clippy clean
- **Plan 1b** (Tauri Desktop Shell + Dashboard): DONE
- **Plan 2** (Dashboard CRUD Parity): DONE — every API action reachable from the GUI

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

### Plan 2 additions

- Every API action is accessible from the dashboard. Add/Remove projects, Start sessions, Send text, Handoff, End session, and Autopilot toggle all live in modals reachable within two clicks. The CLI remains available as a debugging/scripting escape hatch.
- Click any session preview on the dashboard to open a near-fullscreen focused view with a collapsible action rail (ESC or backdrop click collapses).
- New endpoints:
  - `DELETE /api/projects/:id` — cascade-deletes the project, its sessions, cycles, and signals. The directory on disk is not touched.
  - `PUT /api/sessions/:id/autopilot` — toggles the `autopilot` flag on a session. State-only in Plan 2; Plan 3 wires behavior.

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
