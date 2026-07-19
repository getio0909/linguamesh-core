# Storage schema 15 to 16

Schema 16 adds the nullable `routing_profile_id` column to
`document_job_options` and indexes it for bounded lookup. Existing jobs remain valid with a
`NULL` value and continue to resume using their persisted provider/model options.

Linux document jobs created through a saved routing profile persist that profile identifier. On
resume or retry, the Linux worker reloads the profile, re-runs deterministic candidate selection,
and reconnects the selected saved provider through the host secret broker. The migration stores
only the non-secret profile identifier; endpoints, credentials, source content, and translated
content remain outside the routing metadata.

The migration is transactional and is applied automatically when a schema-15 database is opened by
Core 0.1.0-alpha.2. Future versions must reject databases with a schema version greater than 16.
