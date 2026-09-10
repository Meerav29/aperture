CREATE TABLE session_summaries (
    id TEXT PRIMARY KEY,
    data TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE file_cursors (
    path TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    offset INTEGER NOT NULL,
    initial_len INTEGER NOT NULL,
    malformed INTEGER NOT NULL,
    session_id TEXT,
    host TEXT,
    created_ns INTEGER,
    updated_at TEXT NOT NULL
);
