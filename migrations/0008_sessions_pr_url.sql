-- Plan 10b Task 1 — add per-session GitHub PR URL.
-- Nullable; an unset value means no PR has been created for this session.
-- No CHECK constraint (the URL is opaque from the DB's perspective).
ALTER TABLE sessions ADD COLUMN pr_url TEXT NULL;
