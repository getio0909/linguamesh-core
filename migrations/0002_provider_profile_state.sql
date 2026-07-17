ALTER TABLE provider_profiles
ADD COLUMN preset_id TEXT NOT NULL DEFAULT 'generic-openai-compatible';

ALTER TABLE provider_profiles
ADD COLUMN adapter_type TEXT NOT NULL DEFAULT 'openai_chat_completions';

ALTER TABLE provider_profiles
ADD COLUMN enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1));

UPDATE provider_profiles
SET secret_ref = NULL
WHERE secret_ref IS NOT NULL;

CREATE TABLE IF NOT EXISTS provider_model_selection (
    provider_id TEXT PRIMARY KEY REFERENCES provider_profiles(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS active_provider_selection (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    provider_id TEXT NOT NULL REFERENCES provider_profiles(id) ON DELETE CASCADE
);
