# Storage schema 6 to 7

Schema 7 adds the `paused` document-job state. The migration rebuilds the bounded
`document_jobs` and `document_segments` tables transactionally so existing snapshots,
timestamps, indexes, and foreign-key cascade behavior are preserved. No source path,
credential value, or session secret is introduced.

Paused jobs remain resumable after process restart. Completed segments stay persisted,
while pending prose segments remain untranslated until an explicit resume or retry.
