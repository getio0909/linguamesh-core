CREATE TABLE IF NOT EXISTS document_jobs (
    job_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'completed', 'cancelled', 'failed')),
    format TEXT NOT NULL CHECK (format IN ('txt', 'markdown')),
    source_name TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS document_segments (
    job_id TEXT NOT NULL REFERENCES document_jobs(job_id) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('prose', 'verbatim')),
    source_text TEXT NOT NULL,
    translated_text TEXT,
    line_ending TEXT NOT NULL CHECK (line_ending IN ('', char(10), char(13) || char(10), char(13))),
    PRIMARY KEY (job_id, segment_index)
);

CREATE INDEX IF NOT EXISTS document_jobs_updated_at_idx
    ON document_jobs (updated_at DESC, job_id DESC);

CREATE INDEX IF NOT EXISTS document_segments_job_idx
    ON document_segments (job_id, segment_index);
