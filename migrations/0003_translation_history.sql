CREATE TABLE IF NOT EXISTS translation_history (
    operation_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    source_text TEXT NOT NULL,
    translated_text TEXT NOT NULL,
    source_locale TEXT,
    target_locale TEXT NOT NULL,
    model_id TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS translation_history_created_at
ON translation_history(created_at DESC, operation_id DESC);
