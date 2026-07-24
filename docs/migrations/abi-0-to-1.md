# ABI 0 to ABI 1 Migration

ABI major 0 was a published prerelease source skeleton recorded by the central manifest. No binary
SDK or compatible client release used that ABI, so there is no binary artifact to replace or
maintain.

ABI major 1 makes returned-buffer ownership engine-bound and removes the process-global allocation
registry. Native clients must make these changes:

1. Expect `LM_ABI_VERSION_MAJOR` and `lm_engine_get_abi_version()` to return `1`.
2. Rebuild against the ABI 1 header. `LmBuffer` now includes `allocation_id`; initialize the whole
   descriptor to zero and treat every returned field as read-only.
3. Replace `lm_buffer_free(&buffer)` with `lm_engine_buffer_free(engine, &buffer)`.
4. Pass the same live engine that returned the buffer. Release remains valid after shutdown, but
   every buffer must be released before destroying that engine.
5. Handle `LM_RESULT_RESOURCE_EXHAUSTED` when an engine already owns 64 outstanding buffers. Release
   a buffer before polling again.
6. Treat `LM_RESULT_INVALID_ARGUMENT` after destroy as the expected result for stale or repeated
   engine-handle calls. ABI 1 keeps opaque handle tombstones so rejected calls never dereference a
   freed engine address; callers must still coordinate worker shutdown before destroy.

The Protobuf protocol remains at version 1. Command and event payloads are unchanged by this ABI
migration.
