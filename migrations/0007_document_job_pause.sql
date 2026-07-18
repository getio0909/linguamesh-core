CREATE TABLE document_jobs_v7 (
    job_id TEXT PRIMARY KEY,
    state TEXT NOT NULL CHECK (state IN ('pending', 'running', 'paused', 'completed', 'cancelled', 'failed')),
    format TEXT NOT NULL CHECK (format IN ('txt', 'markdown')),
    source_name TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

INSERT INTO document_jobs_v7 (job_id, state, format, source_name, created_at, updated_at)
SELECT job_id, state, format, source_name, created_at, updated_at
FROM document_jobs;

CREATE TABLE document_segments_v7 (
    job_id TEXT NOT NULL REFERENCES document_jobs_v7(job_id) ON DELETE CASCADE,
    segment_index INTEGER NOT NULL CHECK (segment_index >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('prose', 'verbatim')),
    source_text TEXT NOT NULL,
    translated_text TEXT,
    line_ending TEXT NOT NULL CHECK (line_ending IN ('', char(10), char(13) || char(10), char(13))),
    PRIMARY KEY (job_id, segment_index)
);

INSERT INTO document_segments_v7 (job_id, segment_index, kind, source_text, translated_text, line_ending)
SELECT job_id, segment_index, kind, source_text, translated_text, line_ending
FROM document_segments;

DROP TABLE document_segments;
DROP TABLE document_jobs;
ALTER TABLE document_jobs_v7 RENAME TO document_jobs;
ALTER TABLE document_segments_v7 RENAME TO document_segments;

CREATE INDEX IF NOT EXISTS document_jobs_updated_at_idx
    ON document_jobs (updated_at DESC, job_id DESC);

CREATE INDEX IF NOT EXISTS document_segments_job_idx
    ON document_segments (job_id, segment_index);
