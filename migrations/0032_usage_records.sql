CREATE TABLE IF NOT EXISTS usage_records (
    operation_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    provider_id TEXT,
    model_id TEXT NOT NULL,
    source TEXT NOT NULL CHECK(source IN ('provider_reported', 'locally_estimated', 'unknown')),
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER
);

CREATE INDEX IF NOT EXISTS usage_records_created_at
ON usage_records(created_at DESC, operation_id DESC);
