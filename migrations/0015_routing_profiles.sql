CREATE TABLE routing_profiles (
    profile_id TEXT PRIMARY KEY,
    profile_json TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX routing_profiles_updated_at_idx
    ON routing_profiles (updated_at DESC, profile_id ASC);
