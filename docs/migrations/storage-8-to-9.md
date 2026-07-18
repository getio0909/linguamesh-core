# Storage schema 8 to 9

Schema 9 expands the bounded document-job format constraint to include `srt`, `webvtt`, and
`csv`. The migration transactionally rebuilds the three document-job tables so their foreign keys
continue to target the rebuilt parent table, while preserving existing jobs, segments, options,
timestamps, and indexes.

CSV jobs store field and delimiter segments in the existing snapshot shape; no source path,
credential, endpoint, or provider secret is added. Older Core binaries cannot safely open a live
schema-9 database and must be upgraded before resuming document jobs.
