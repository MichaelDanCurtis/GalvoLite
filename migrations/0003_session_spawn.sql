-- GalvoLite spawn/fan-out/rejoin schema delta (schema v0003)
-- See docs/superpowers/specs/2026-04-30-session-spawn-fanout-rejoin-design.md §2.
--
-- parent_session_id and worktree_path are already present from 0001_initial.sql;
-- this migration only adds the new wait/rejoin/terminal-status columns and the
-- parent lookup index.

ALTER TABLE sessions ADD COLUMN waiting_on      TEXT NULL;
ALTER TABLE sessions ADD COLUMN pending_rejoin  TEXT NULL;
ALTER TABLE sessions ADD COLUMN terminal_status TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_sessions_parent ON sessions(parent_session_id);
