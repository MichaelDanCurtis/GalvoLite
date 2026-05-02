-- Plan 8 — Overseer + Plan Executor schema delta (schema v0004)
-- See docs/superpowers/specs/2026-05-02-plan-8-executor-overseer-design.md §3.

CREATE TABLE recommendations (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL,
    kind            TEXT NOT NULL,
    severity        TEXT NOT NULL CHECK (severity IN ('info','warn','urgent')),
    title           TEXT NOT NULL,
    body            TEXT NOT NULL,
    rationale       TEXT NOT NULL,
    evidence_json   TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('pending','acted','dismissed','deferred')),
    created_at      TEXT NOT NULL,
    deferred_until  TEXT NULL,
    acted_at        TEXT NULL,
    content_hash    TEXT NOT NULL UNIQUE
);
CREATE INDEX recommendations_project_status_idx
    ON recommendations(project_id, status, created_at);

ALTER TABLE sessions ADD COLUMN pending_overseer_recs TEXT NULL;
