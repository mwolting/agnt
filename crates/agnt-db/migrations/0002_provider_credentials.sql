CREATE TABLE provider_credentials (
    provider_id TEXT PRIMARY KEY,
    credential_value TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
