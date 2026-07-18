# Architecture

The workspace separates stable domain types, protocol envelopes, provider contracts, the
OpenAI-compatible adapter, persistence, application orchestration, test infrastructure, C ABI, and
CLI. Dependencies point inward toward domain and provider abstractions. Native clients consume the
engine through platform wrappers over the stable C ABI, except Linux where direct Rust integration
is preferred. The current prerelease wrappers are maintained source; reproducible wrapper
generation remains future work.

`linguamesh-application` connects a canonical non-secret `ProviderProfile` to a concrete adapter.
Before resolving a secret, it validates the adapter and endpoint policy. A bounded channel emits a
correlated `SecretRequired` lease to a native host; the host may provide one zeroizing `SecretValue`
or one of the closed safe failure categories. Cancellation drops the response receiver so a late
secret cannot be accepted. `ProviderManager` enforces one active provider per application instance;
switching succeeds atomically, and `disconnect` closes the old provider session, cancels retained
or in-flight operations, and clears its sole cached credential. A header already transmitted to the
configured provider cannot be revoked, so session closure drops the HTTP future as soon as the
runtime can observe cancellation. Platform secure storage and session fallback policy remain
native-host responsibilities.

SQLite schema version 2 adds provider preset/adapter/enabled state, active-provider selection, and
per-profile last-model selection. The migration is transactional, preserves schema-1 profile
metadata while clearing untrusted legacy secret references, enables WAL, secure deletion, and
foreign-key enforcement for every connection, and never defines a credential-value column. Every
supported on-disk open retries the truncating checkpoint so a busy post-migration attempt fails
closed without abandoning cleanup. On Linux's default Unix VFS, SQLite's no-follow open flag
rejects any symbolic-link component in the database path before migrations. Other VFS
implementations require platform-specific enforcement and tests; native hosts still enforce
private directories and leaf-file metadata. See
[`Storage schema 1 to 2`](migrations/storage-1-to-2.md).

`linguamesh-engine::core_compatibility` reports Core semantic version, ABI major, protocol version,
bundled provider-catalog version, and stable enabled-feature identifiers. Clients compare every
version dimension and require their feature subset before provider work; exact prerelease version
matching is intentional until a compatible range policy is defined.

The OpenAI-compatible translation path automatically protects common structured source spans before
prompt construction. URLs, email addresses, Markdown code spans, fenced code, and common placeholder
forms become collision-checked opaque markers; an incremental restorer validates marker identity and
emits the original spans across split SSE deltas. Missing, duplicated, or unknown markers fail the
operation as a typed malformed-response error. This is a safety boundary, not a user glossary or
custom protected-term system; those require explicit translation options and additional validation.

The ABI owns a Tokio runtime, a bounded event channel, and at most one active operation per opaque
engine handle. A submitted `translate_text` envelope is decoded into the same `TranslationEngine`
used by Rust callers. A forwarding task encodes ordered domain events back into Protobuf envelopes;
native polling never invokes arbitrary callbacks. See [native-sdk.md](native-sdk.md) for the wire
contract and current host-service limitations. The typed Rust secret broker is not yet projected
through `lm_engine_send_host_response`, so non-Linux wrappers cannot claim that capability.
