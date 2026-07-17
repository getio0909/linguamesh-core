CREATE TABLE IF NOT EXISTS schema_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    version INTEGER NOT NULL
);

INSERT INTO schema_metadata (singleton, version)
VALUES (1, 1)
ON CONFLICT(singleton) DO UPDATE SET version = MAX(version, excluded.version);

CREATE TABLE IF NOT EXISTS provider_profiles (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    base_endpoint TEXT NOT NULL,
    secret_ref TEXT
);

CREATE TABLE IF NOT EXISTS model_descriptors (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('discovered', 'catalog', 'manual'))
);

CREATE TABLE IF NOT EXISTS active_model_selection (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    model_id TEXT NOT NULL REFERENCES model_descriptors(id)
);
