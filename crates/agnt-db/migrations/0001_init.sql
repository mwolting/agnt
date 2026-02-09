CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    root_dir TEXT NOT NULL UNIQUE,
    name TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT,
    root_turn_id TEXT,
    current_turn_id TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_sessions_project_updated
    ON sessions(project_id, updated_at_ms DESC);

CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    parent_turn_id TEXT REFERENCES turns(id) ON DELETE SET NULL,
    user_parts_json TEXT NOT NULL CHECK (json_valid(user_parts_json)),
    assistant_parts_json TEXT NOT NULL CHECK (json_valid(assistant_parts_json)),
    conversation_state_json TEXT NOT NULL CHECK (json_valid(conversation_state_json)),
    usage_json TEXT CHECK (usage_json IS NULL OR json_valid(usage_json)),
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_turns_session_created
    ON turns(session_id, created_at_ms ASC);

CREATE INDEX idx_turns_session_parent
    ON turns(session_id, parent_turn_id);

CREATE TABLE session_ops (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    op_type TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_session_ops_session_seq
    ON session_ops(session_id, seq);
