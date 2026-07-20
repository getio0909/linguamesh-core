ALTER TABLE document_job_options
    ADD COLUMN quality_mode TEXT NOT NULL DEFAULT 'balanced';
