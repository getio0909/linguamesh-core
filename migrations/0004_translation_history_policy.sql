CREATE TABLE IF NOT EXISTS translation_history_policy (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    enabled INTEGER NOT NULL CHECK(enabled IN (0, 1))
);

INSERT INTO translation_history_policy (singleton, enabled)
VALUES (1, 1)
ON CONFLICT(singleton) DO NOTHING;
