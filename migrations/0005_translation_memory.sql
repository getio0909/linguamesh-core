CREATE TABLE IF NOT EXISTS translation_memory_policy (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
);

INSERT OR IGNORE INTO translation_memory_policy (singleton, enabled) VALUES (1, 1);

CREATE TABLE IF NOT EXISTS translation_memory (
    cache_key TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    source_text TEXT NOT NULL,
    translated_text TEXT NOT NULL,
    source_locale TEXT,
    target_locale TEXT NOT NULL,
    model_id TEXT NOT NULL,
    identity_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS translation_memory_created_at_idx
    ON translation_memory (created_at DESC, cache_key DESC);
