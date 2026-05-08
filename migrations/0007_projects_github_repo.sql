-- Plan 10a Task 1 — add per-project linked GitHub repo (owner/name).
-- Nullable; an unset value means the project has no linked repo.
-- No CHECK constraint (we validate the format/existence at the API layer).
ALTER TABLE projects ADD COLUMN github_repo TEXT NULL;
