-- Plan 10d Task 1 — track which GitHub issue spawned this session.
ALTER TABLE sessions ADD COLUMN github_issue_url TEXT NULL;
