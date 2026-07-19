ALTER TABLE document_job_options
    ADD COLUMN routing_profile_id TEXT;

CREATE INDEX IF NOT EXISTS document_job_options_routing_profile_idx
    ON document_job_options (routing_profile_id);
