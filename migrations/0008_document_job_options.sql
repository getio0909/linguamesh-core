CREATE TABLE IF NOT EXISTS document_job_options (
    job_id TEXT PRIMARY KEY REFERENCES document_jobs(job_id) ON DELETE CASCADE,
    source_locale TEXT,
    target_locale TEXT NOT NULL,
    model_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    glossary_json TEXT,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
