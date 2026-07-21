# Storage Schema 1 to 2

Change identifier: `LM-CHANGE-2026-07-LINUX-SECURE-PROVIDER-1`

Schema 2 extends the existing non-secret `provider_profiles` table with provider preset, adapter,
and enabled state. It also adds active-provider and per-profile last-model tables. Existing schema-1
profiles receive the generic OpenAI-compatible defaults and remain enabled.

Alpha 1 did not enforce the new random-UUID `SecretRef` format, so the migration clears every
legacy `secret_ref` while preserving the rest of each provider row. The native client must ask the
user to attach the secure credential again. New session-only references use `session:<uuid>` and
are rejected by every storage write path; they exist only for the lifetime of the native client
process.

`Storage::open` detects the recorded version and applies each missing migration in one transaction.
It rejects a database newer than the current Core instead of guessing compatibility. Foreign-key
enforcement is enabled for every connection, so deleting a profile also removes its active and
last-model references. On-disk databases use WAL with `synchronous=FULL` and secure deletion. A
successful migration performs a truncating checkpoint before the database is returned. If the
checkpoint is busy, `Storage::open` fails closed. Every later supported on-disk open retries the
checkpoint even when schema version 2 is already committed, so a transient reader cannot
permanently strand a cleared alpha-1 credential value in the WAL or main database.

WAL is the current Linux desktop choice because the client needs concurrent UI reads and worker
writes, and the sidecar files can be exercised directly by the credential-leakage gate. Broader
platform measurements remain a later release gate; this checkpoint does not claim them.

No credential value is introduced. `secret_ref` remains the only credential-related column. The
native client must remove the corresponding platform-secret item separately when a user confirms
profile deletion.

Rollback means restoring the application and database together from a user-controlled backup.
Downgrading a live schema-2 database to an alpha-1 binary is not supported. Alpha 1 did not enforce
future-schema rejection and can ignore schema-2 selection state, so it must not open a live
schema-2 database.
