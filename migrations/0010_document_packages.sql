CREATE TABLE document_jobs_v10 (
    job_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'paused', 'completed', 'cancelled', 'failed')),
    format TEXT NOT NULL CHECK (format IN ('txt', 'markdown', 'srt', 'webvtt', 'csv', 'html', 'json', 'docx')),
    source_name TEXT NOT NULL,
    package_blob BLOB,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

INSERT INTO document_jobs_v10 (job_id, state, format, source_name, created_at, updated_at)
SELECT job_id, state, format, source_name, created_at, updated_at
FROM document_jobs;

CREATE TABLE document_segments_v10 (
    job_id TEXT NOT NULL REFERENCES document_jobs_v10(job_id) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('prose', 'verbatim')),
    source_text TEXT NOT NULL,
    translated_text TEXT,
    line_ending TEXT NOT NULL CHECK (line_ending IN ('', char(10), char(13) || char(10), char(13))),
    PRIMARY KEY (job_id, segment_index)
);

INSERT INTO document_segments_v10 (job_id, segment_index, kind, source_text, translated_text, line_ending)
SELECT job_id, segment_index, kind, source_text, translated_text, line_ending
FROM document_segments;

CREATE TABLE document_job_options_v10 (
    job_id TEXT PRIMARY KEY REFERENCES document_jobs_v10(job_id) ON DELETE CASCADE,
    source_locale TEXT,
    target_locale TEXT NOT NULL,
    model_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    glossary_json TEXT,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

INSERT INTO document_job_options_v10 (job_id, source_locale, target_locale, model_id, provider_id, glossary_json, updated_at)
SELECT job_id, source_locale, target_locale, model_id, provider_id, glossary_json, updated_at
FROM document_job_options;

DROP TABLE document_job_options;
DROP TABLE document_segments;
DROP TABLE document_jobs;
ALTER TABLE document_jobs_v10 RENAME TO document_jobs;
ALTER TABLE document_segments_v10 RENAME TO document_segments;
ALTER TABLE document_job_options_v10 RENAME TO document_job_options;

CREATE INDEX IF NOT EXISTS document_jobs_updated_at_idx
    ON document_jobs (updated_at DESC, job_id DESC);

CREATE INDEX IF NOT EXISTS document_segments_job_idx
    ON document_segments (job_id, segment_index);
