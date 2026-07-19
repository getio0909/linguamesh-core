# Architecture

The workspace separates stable domain types, protocol envelopes, provider contracts, the
OpenAI-compatible and native Ollama adapters, persistence, application orchestration, test
infrastructure, C ABI, and CLI. Dependencies point inward toward domain and provider abstractions.
Native clients consume the
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

The provider catalog maps `local-loopback` to the OpenAI-compatible `/v1/` contract and `ollama` to
the native loopback-only `/api/` contract. The latter discovers models through `/api/tags` and
streams `/api/chat` as newline-delimited JSON; both adapters share endpoint validation,
cancellation, bounded response handling, protected-span restoration, and redacted credential
lifetimes. The in-process fixture verifies the wire shape, not interoperability with an
independently running Ollama daemon.

SQLite migrations currently reach schema version 9. Schema 2 adds provider preset/adapter/enabled
state, active-provider selection, and per-profile last-model selection; later migrations add bounded
translation history, optional translation-memory policy/entries, and bounded TXT/Markdown/SRT/WebVTT/CSV
document jobs with segment snapshots. The migrations are transactional,
preserve schema-1 profile metadata while clearing untrusted legacy secret references, enable WAL,
secure deletion, and
foreign-key enforcement for every connection, and never defines a credential-value column. Every
supported on-disk open retries the truncating checkpoint so a busy post-migration attempt fails
closed without abandoning cleanup. On Linux's default Unix VFS, SQLite's no-follow open flag
rejects any symbolic-link component in the database path before migrations. Other VFS
implementations require platform-specific enforcement and tests; native hosts still enforce
private directories and leaf-file metadata. See
[`Storage schema 1 to 2`](migrations/storage-1-to-2.md).

The `linguamesh-document` crate is the first `bounded_text_document_v1` document-codec contract. It recognizes
UTF-8 TXT, Markdown, SRT, WebVTT, and CSV names, enforces a 4 MiB input/output bound, strips an optional
UTF-8 BOM, retains LF/CRLF/CR line endings, and represents Markdown fenced code plus subtitle cue
IDs, headers, and timing lines as verbatim segments. Prose segments can be completed independently
and the job reconstructs the original ordering without allowing untranslated or structural segments
to be overwritten; subtitle reconstruction revalidates timestamps and cue structure, while CSV
preserves delimiters, quoting, variable-width rows, and selected-column boundaries. Core schema
6 persists bounded job metadata and ordered segment snapshots without local paths or credential
values; schema 7 adds a transactionally migrated paused state, and schema 8 adds validated,
non-secret document translation options; schema 9 expands the stored format check for CSV and the
subtitle codecs. Linux worker startup restores pending/running/paused jobs
and exposes explicit segment/state commands; resume/retry reuses the saved provider/model/glossary
only after the active runtime matches. Archive formats and a multi-job GUI queue remain future work.

`linguamesh-engine::core_compatibility` reports Core semantic version, ABI major, protocol version,
bundled provider-catalog version, and stable enabled-feature identifiers. Clients compare every
version dimension and require their feature subset before provider work; exact prerelease version
matching is intentional until a compatible range policy is defined.

The `routing_planner_v1` feature exposes the domain-level `RoutingProfile` contract. Manual and
Ordered modes preserve user intent; Automatic mode produces a stable, explainable ranking. All
candidate filtering is fail-closed against explicit local/privacy, capability, size, locale, and
quality constraints, and fallback candidates are returned only when the profile explicitly permits
them. The planner carries no endpoints, credentials, or source text. Platform clients remain
responsible for binding saved profiles and user-visible routing controls to this contract.

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
