ALTER TABLE provider_profiles
    ADD COLUMN request_timeout_secs INTEGER NOT NULL DEFAULT 30;
