# Storage schema 7 to 8

Schema 8 adds `document_job_options`, a one-to-one table for validated, non-secret
document translation parameters. It stores optional source locale, target locale,
model identifier, provider profile identifier, and an optional bounded glossary JSON
document. The job foreign key cascades on deletion and the migration is additive.

No endpoint, filesystem path, credential value, session secret, or privacy-mode state
is stored. Resume and retry require the active runtime provider and model to match the
saved identifiers; older jobs without options must be started again once to populate
the record.

`Storage::open` applies this migration transactionally and rejects a database with a
newer schema. Downgrading a live schema-8 database to an earlier Core binary is not
supported.
