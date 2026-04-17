# Plan 2 — Dashboard CRUD Parity Design

**Status:** Design approved 2026-04-17
**Predecessor:** Plan 1b (Tauri Desktop Shell + Dashboard) — merged `a7fea65`

## Goal

Make the dashboard cover every user-facing API action. After Plan 1b the dashboard is read-only; users still need `curl` or CLI to add projects, start sessions, send text, or remove projects. Plan 2 closes that gap and establishes the interaction patterns (modals, rail, expand-to-focus) that all future plans build on.

**Non-goal:** Plan 2 does NOT implement autopilot behavior. It only adds the toggle + endpoint that records intent. Plan 3 wires the behavior.

## Terminology (load-bearing)

- **Project** — a logical grouping of sessions pointing at a workspace directory. GalvoLite references the directory; it does NOT own or modify it. Removing a project from GalvoLite's list never touches the directory on disk.
- **Session** — a PTY run under a project. `DELETE /api/sessions/:id` (existing) kills the PTY but preserves the row as history.
- **Remove project** (new) — cascade: kill any running sessions under this project → purge session rows (+ signals + cycles) → delete project row.

## Scope

### New GUI controls
1. **Add Project modal** — name + path. Validates path canonicalizes on submit (server-side, already implemented).
2. **Start Session modal** — task + role + command. Three flat fields, no args, no advanced drawer.
3. **Remove Project** action — confirm-modal listing what will be killed/purged.
4. **Session action rail** — right-side collapsible icon strip on the expanded session view:
   - Send text
   - Handoff
   - Autopilot toggle
   - End session
   - Expand/collapse handle

### New backend endpoints
1. `DELETE /api/projects/:id` — cascade delete. Kills running sessions (via `SessionManager::kill_session` for each), then a single transactional DB purge: `DELETE FROM signals`, `DELETE FROM cycles`, `DELETE FROM sessions`, `DELETE FROM projects` scoped to the project_id. Returns 204 on success, 404 if project not found.
2. `PUT /api/sessions/:id/autopilot` — state-only. Body: `{ "enabled": bool }`. Persists the flag on the session row. Plan 3 reads this flag to gate autonomy.

No other DB schema changes. Autopilot flag is a new `autopilot_enabled BOOLEAN NOT NULL DEFAULT 0` column on `sessions` — one migration.

## Interaction pattern

**Modals, not side panels.** Every mutating action opens a modal. Reasoning: modals enforce focus on the single action, match the "pop out" motion the user wants for rail actions, and compose cleanly with the expand-to-focus session view.

**Animation:** Framer Motion `scale` + `originX/Y` from the trigger element's click coordinates. Modals spring out from their trigger. Close via ESC, × button, or click-outside.

## Session detail — expand to focus

Clicking a session preview card on the dashboard scales + translates it into a near-fullscreen focused view (~92% of viewport). Dimmed backdrop covers the dashboard behind. Terminal fills the main area; rail sits on the right edge.

### Rail states
- **Collapsed (default):** 28–32px wide icon strip. Icons: ✎ Send · ⇄ Handoff · ⚡ Autopilot (colored when on) · ⏻ End · ‹ expand handle.
- **Expanded:** ~110px wide with labels beside icons. Toggled by the handle or by clicking an icon (which opens that icon's modal).

Icon click behavior:
- Send → opens Send modal (textarea + send button).
- Handoff → opens Handoff modal (target role + initial message).
- Autopilot → toggles immediately (PUT autopilot endpoint); indicator color reflects state.
- End → opens Confirm-End modal.

## Data flow

```
User clicks session preview
  → React state: expandedSessionId = id
  → Framer Motion animates card → focused frame
  → Session detail view renders
       ├── Terminal (existing TerminalPreview component, full-size mode)
       ├── Rail (new SessionRail component)
       └── Modal slot (conditionally renders active modal)

User clicks rail icon (e.g. Send)
  → React state: activeModal = 'send'
  → Motion animates modal from icon coordinates
  → Modal submits → POST /api/sessions/:id/send
  → Success: modal closes, rail stays visible
  → Error: modal shows inline error, stays open
```

## Component inventory

**New:**
- `AddProjectModal.tsx`
- `StartSessionModal.tsx`
- `RemoveProjectModal.tsx` (confirm)
- `SessionRail.tsx` (collapsible icon strip)
- `SendModal.tsx`
- `HandoffModal.tsx`
- `EndSessionModal.tsx` (confirm)
- `ExpandedSessionView.tsx` (near-fullscreen frame + backdrop)
- `useExpandedSession` hook (manages expand state + ESC/backdrop handlers)

**Modified:**
- `Dashboard.tsx` — mount AddProject + StartSession trigger buttons, wire expand state.
- `SessionRow.tsx` (or equivalent card) — click handler → expand.
- `api.ts` — add `deleteProject`, `setAutopilot`.
- `useProjects`, `useSessions` hooks — mutation helpers.

## Error handling

- **Path validation:** Server canonicalizes. Non-existent path → 400. Modal shows inline error, keeps form state.
- **Duplicate project name:** Server-side uniqueness check (add if missing). 409 → inline modal error.
- **Remove project while a session is running:** Kill sequence is best-effort per session; if a kill times out (>5s), the remove fails with 500 and the modal tells the user. Partial kill state is possible but the next remove retry completes it.
- **Autopilot toggle on a dead session:** 409 SessionNotRunning. Modal shows message; rail icon reverts state.
- **Send on a dead session:** 409. Inline error in send modal.

## Testing

**Backend:**
- Integration test for `DELETE /api/projects/:id` covering: project with no sessions, project with ended sessions, project with a running session (verifies kill + purge).
- Integration test for `PUT /api/sessions/:id/autopilot` covering: toggle on, toggle off, 404 on unknown session, 409 on dead session.
- Migration test: existing sessions default to autopilot_enabled = 0.

**Frontend:**
- RTL tests for each modal: open via trigger, submit happy path, submit error path, cancel via ESC / × / backdrop.
- RTL test for rail: collapse toggle, icon click opens correct modal, autopilot icon toggles without a modal.
- RTL test for ExpandedSessionView: opens on card click, ESC closes, backdrop click closes, state persists across render.

**Visual:** Manual verification — motion feels responsive, rail doesn't flicker on expand/collapse, modals spring from their triggers.

## Out of scope (defer)

- Autopilot behavior (Plan 3).
- Signals/cycles widgets on the rail (placeholder only; Plan 4+).
- Model selection for agents (Plan 2.a brainstorm).
- New-project seeding flows (Plan 5+).
- Multi-agent adapters (Plan 6+).
- Pagination (still deferred from Plan 1b M3).
- Auth/token on loopback HTTP (still deferred from Plan 1b M2).

## Open risks

1. **Cascade-delete atomicity.** Killing sessions is async + per-PTY; the DB purge is transactional. If a kill fails mid-way, some sessions end up killed but the project stays. Acceptable — user retries. Alternative (two-phase with undo) is overkill for solo use.
2. **Motion on low-power machines.** Framer Motion springs are cheap, but if the terminal is actively streaming heavy output during an expand animation, frame drops possible. Test with `mock-claude` streaming at 100 lines/sec.
3. **Rail icon legibility.** 28px strip is tight. Plan to use Lucide icons at 16px with hover tooltips. If legibility suffers, bump to 32px.

## Success criteria

- Every action listed in Plan 1b's README "CLI caveat" section has a GUI equivalent reachable in ≤2 clicks from the dashboard.
- Workspace `cargo test` green including new DELETE + autopilot integration tests.
- `npm test` green in `ui/` for all new modal + rail components.
- Manual smoke: start an empty GalvoLite → add a project → start a session → send text → toggle autopilot → end session → remove project, all via GUI, zero terminal commands.
