-- Plan 9 Task 2 — add 'archived' status + status_changed_at column.
-- SQLite has no ALTER for CHECK constraints, so recreate the table.

CREATE TABLE recommendations_new (
    id              TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL,
    kind            TEXT NOT NULL,
    severity        TEXT NOT NULL CHECK (severity IN ('info','warn','urgent')),
    title           TEXT NOT NULL,
    body            TEXT NOT NULL,
    rationale       TEXT NOT NULL,
    evidence_json   TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('pending','acted','dismissed','deferred','archived')),
    created_at      TEXT NOT NULL,
    deferred_until  TEXT NULL,
    acted_at        TEXT NULL,
    status_changed_at TEXT NOT NULL,
    content_hash    TEXT NOT NULL UNIQUE
);

-- Backfill: existing acted/dismissed rows use acted_at (best available
-- proxy); pending/deferred rows use created_at. This is monotonic with
-- age, which is all the sweep predicate cares about.
INSERT INTO recommendations_new
    SELECT id, project_id, kind, severity, title, body, rationale,
           evidence_json, status, created_at, deferred_until, acted_at,
           COALESCE(acted_at, created_at) AS status_changed_at, content_hash
    FROM recommendations;

DROP TABLE recommendations;
ALTER TABLE recommendations_new RENAME TO recommendations;

-- Preserve the existing list-query index (orders by created_at DESC).
CREATE INDEX recommendations_project_status_idx
    ON recommendations(project_id, status, created_at);

-- New index for the archive sweep predicate.
CREATE INDEX recommendations_status_changed_idx
    ON recommendations(status, status_changed_at);
