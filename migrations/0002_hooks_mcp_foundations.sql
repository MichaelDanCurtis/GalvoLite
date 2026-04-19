-- GalvoLite hooks+MCP coordination layer foundations (schema v0002)
-- See docs/superpowers/specs/2026-04-18-hooks-mcp-coordination-layer-design.md §8.2.
--
-- Adds:
--  * sessions: api_token, turn_count, est_tokens, last_status_note,
--    handoff_path, plan_markdown, seed_markdown, last_event_ts
--  * events     — append-only event log (single source of truth for state changes)
--  * plan_steps — structured rows derived from MCP set_plan markdown
--  * path_claims — reserved; populated by Sub-plan 4 spawn/rejoin
--
-- Pre-v2 sessions are reconciled to 'error' per spec §8.7 because the new
-- 7-state SessionState enum is incompatible with the v1 lifecycle.
--
-- All columns are non-NULL with safe defaults so the ALTER cannot fail on
-- pre-existing rows. SQLite's ALTER TABLE only supports column add (not
-- modify), which is fine: the existing `state TEXT NOT NULL` column accepts
-- the new enum strings without schema change.

ALTER TABLE sessions ADD COLUMN api_token        TEXT;
ALTER TABLE sessions ADD COLUMN turn_count       INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN est_tokens       INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sessions ADD COLUMN last_status_note TEXT;
ALTER TABLE sessions ADD COLUMN handoff_path     TEXT;
ALTER TABLE sessions ADD COLUMN plan_markdown    TEXT;
ALTER TABLE sessions ADD COLUMN seed_markdown    TEXT;
ALTER TABLE sessions ADD COLUMN last_event_ts    TEXT;

CREATE TABLE IF NOT EXISTS events (
    id            INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    ts            TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    session_id    TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source        TEXT NOT NULL CHECK(source IN ('mcp','hook','operator','system')),
    kind          TEXT NOT NULL,
    payload_json  TEXT NOT NULL,
    result_json   TEXT,
    prev_state    TEXT,
    next_state    TEXT,
    dedup_key     TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS events_session_ts ON events(session_id, ts);
CREATE INDEX IF NOT EXISTS events_kind_ts    ON events(kind, ts);

CREATE TABLE IF NOT EXISTS plan_steps (
    id           INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    step_hash    TEXT NOT NULL,
    ordinal      INTEGER NOT NULL,
    title        TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'pending'
                 CHECK(status IN ('pending','in_progress','done','blocked')),
    note         TEXT,
    owner_id     TEXT REFERENCES sessions(id),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(session_id, step_hash)
);
CREATE INDEX IF NOT EXISTS plan_steps_session_ord ON plan_steps(session_id, ordinal);

CREATE TABLE IF NOT EXISTS path_claims (
    id         INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    glob       TEXT NOT NULL,
    claimed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    UNIQUE(glob)
);

-- One-time reconciliation: pre-existing v1 sessions to 'error' (spec §8.7).
UPDATE sessions
SET    state            = 'error',
       last_status_note = 'pre-migration; state unknown'
WHERE  ended_at IS NULL;
