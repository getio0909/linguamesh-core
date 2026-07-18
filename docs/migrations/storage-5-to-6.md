# Storage Schema 5 to 6

Schema 6 adds bounded local document-task recovery for the Linux-first TXT/Markdown workflow.

## Tables

- `document_jobs` stores an opaque job ID, lifecycle state, format, source basename, and timestamps.
- `document_segments` stores ordered segment kind, source text, optional translation, and original
  line ending. Rows cascade when a job is deleted.

The migration stores no filesystem path, provider credential, session secret, or archive payload.
Core validates job IDs, source names, segment ordering, line endings, structure-segment immutability,
and 4 MiB/10,000-segment bounds before writing a snapshot. `Storage::resumable_document_jobs` returns
only `pending` and `running` jobs for restart recovery; completed, cancelled, and failed snapshots
remain inspectable until explicitly deleted.

`Storage::open` applies this migration transactionally and rejects a database with a newer schema.
Downgrading a live schema-6 database to an earlier Core binary is not supported.
