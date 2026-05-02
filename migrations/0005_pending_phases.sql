-- Plan 8 T4 — add pending_phases_json column to sessions (schema v0005)
-- Stores remaining phases as JSON for the phase reactor (Task 6).

ALTER TABLE sessions ADD COLUMN pending_phases_json TEXT NULL;
