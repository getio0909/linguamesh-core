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
closed without abandoning cleanup. See
[`Storage schema 1 to 2`](migrations/storage-1-to-2.md).

`linguamesh-engine::core_compatibility` reports Core semantic version, ABI major, protocol version,
bundled provider-catalog version, and stable enabled-feature identifiers. Clients compare every
version dimension and require their feature subset before provider work; exact prerelease version
matching is intentional until a compatible range policy is defined.

The ABI owns a Tokio runtime, a bounded event channel, and at most one active operation per opaque
engine handle. A submitted `translate_text` envelope is decoded into the same `TranslationEngine`
used by Rust callers. A forwarding task encodes ordered domain events back into Protobuf envelopes;
native polling never invokes arbitrary callbacks. See [native-sdk.md](native-sdk.md) for the wire
contract and current host-service limitations. The typed Rust secret broker is not yet projected
through `lm_engine_send_host_response`, so non-Linux wrappers cannot claim that capability.
