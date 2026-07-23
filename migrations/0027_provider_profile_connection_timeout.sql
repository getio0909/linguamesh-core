ALTER TABLE provider_profiles
    ADD COLUMN connection_timeout_secs INTEGER NOT NULL DEFAULT 10;
