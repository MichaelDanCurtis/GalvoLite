-- GalvoLite ritual-engine schema v0001
-- All IDs are TEXT (UUID v4). Timestamps are TEXT (RFC3339).

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL PRIMARY KEY,
    applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    remote_url TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT NOT NULL PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('implementation','brainstorm','scout','fix')),
    branch TEXT,
    worktree_path TEXT NOT NULL,
    pty_pid INTEGER,
    state TEXT NOT NULL,
    autopilot INTEGER NOT NULL DEFAULT 0,
    parent_session_id TEXT REFERENCES sessions(id),
    task_summary TEXT NOT NULL,
    created_at TEXT NOT NULL,
    ended_at TEXT
);

CREATE TABLE IF NOT EXISTS cycles (
    id TEXT NOT NULL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    handoff_path TEXT,
    UNIQUE(session_id, ordinal)
);

CREATE TABLE IF NOT EXISTS signals (
    id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    cycle_id TEXT REFERENCES cycles(id),
    kind TEXT NOT NULL,
    payload TEXT,
    at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_project_id ON sessions(project_id);
CREATE INDEX IF NOT EXISTS idx_cycles_session_id ON cycles(session_id);
CREATE INDEX IF NOT EXISTS idx_signals_session_id ON signals(session_id);
CREATE INDEX IF NOT EXISTS idx_signals_at ON signals(at);
