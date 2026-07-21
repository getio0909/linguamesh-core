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

The provider catalog maps `local-loopback` to the OpenAI-compatible `/v1/` contract, `ollama` to
the native loopback-only `/api/` contract, `anthropic` to the manual-model `/v1/messages`
contract, `azure-openai` to the deployment-scoped Azure Chat Completions contract, and
`openai-responses` to the OpenAI Responses `/v1/responses` contract. Ollama discovers models
through `/api/tags` and streams `/api/chat` as newline-delimited JSON; Anthropic and Azure require
the selected manual model/deployment before any host secret request. Azure uses the `api-key`
header and a pinned API-version query, never a credential query parameter. Responses uses Bearer
authentication and typed SSE event names, including `response.output_text.delta` and
`response.completed`. The adapters share endpoint validation, cancellation, bounded response handling,
protected-span restoration, and redacted credential lifetimes. In-process fixtures verify wire
shapes, not interoperability with independently running third-party services.

SQLite migrations currently reach schema version 18. Schema 2 adds provider preset/adapter/enabled
state, active-provider selection, and per-profile last-model selection; later migrations add bounded
translation history, optional translation-memory policy/entries, and bounded TXT/Markdown/SRT/WebVTT/CSV
document jobs with segment snapshots. The migrations are transactional,
preserve schema-1 profile metadata while clearing untrusted legacy secret references, enable WAL,
secure deletion, and
foreign-key enforcement for every connection, and never defines a credential-value column. Every
supported on-disk open retries the truncating checkpoint so a busy post-migration attempt fails
closed without abandoning cleanup. On Linux's default Unix VFS, SQLite's no-follow open flag
rejects any symbolic-link component in ordinary database paths before migrations. The explicit
`Storage::open_from_trusted_descriptor` path accepts only `/proc/self/fd/<fd>` and is reserved for a
host that has already opened a private regular file with no-follow flags; the Linux client pins the
parent with `openat2(RESOLVE_NO_SYMLINKS)` before creating that descriptor. Other VFS
implementations require platform-specific enforcement and tests; native hosts still enforce
private directories and leaf-file metadata. See
[`Storage schema 1 to 2`](migrations/storage-1-to-2.md).

Schema 15 adds bounded `routing_profiles` JSON persistence. Schema 16 adds an optional
`routing_profile_id` to document-job options so a routed job can reselect its saved profile after a
process restart. Schema 17 adds the validated `quality_mode` (`fast`, `balanced`, or `best`) so
document jobs retain the user's quality policy across pause, restart, and resume. Schema 18 adds
validated `translation_preset_json`; legacy NULL values resolve to `General`, and the serialized
preset is bounded to 8 KiB. The stored payload contains only non-secret identifiers, constraints,
ranking preferences, and bounded style data; endpoints, credentials, and source content are never
stored in this table.

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
or reconnects the saved routing profile when one is recorded. Archive formats and a multi-job GUI
queue remain future work.

`linguamesh-engine::core_compatibility` reports Core semantic version, ABI major, protocol version,
bundled provider-catalog version, and stable enabled-feature identifiers. Clients compare every
version dimension and require their feature subset before provider work; exact prerelease version
matching is intentional until a compatible range policy is defined.

The Rust translation event stream optionally carries a bounded `UsageRecord` on completion. Its
`UsageSource` distinguishes provider-reported counts, conservative local estimates, and unknown
values. The stable C ABI and protobuf projection deliberately exclude this optional field until a
separate compatibility contract is reviewed.

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

`TranslationQualityMode` is part of the provider-neutral request contract. `Fast` asks for one direct
pass, `Balanced` keeps one pass and applies deterministic output validation, and `Best` asks the
model for an internal critique and revision before returning only the final translation. The shared
`translation-prompt-v2` helper versions this wording across all built-in adapters, delimits source
content as untrusted data, and keeps protected-marker instructions explicit. Core rejects empty
output or Unicode replacement characters before marking an operation completed; it never silently
turns one request into multiple paid calls.

`TranslationPreset` is also request-scoped and defaults to `General` for legacy payloads. Its stable
built-ins (`general`, `technical`, and `marketing`) and optional bounded domain, tone, formality,
audience, regional-locale, script, context, and instruction fields are validated before provider
work. The same prompt helper renders these values as escaped data, while translation-memory identity
includes the complete preset so cached output cannot cross user preference boundaries. Document jobs
persist the selected preset through schema 18 and reuse it after restart or retry.

The ABI owns a Tokio runtime, a bounded event channel, and at most one active operation per opaque
engine handle. A submitted `translate_text` envelope is decoded into the same `TranslationEngine`
used by Rust callers. A forwarding task encodes ordered domain events back into Protobuf envelopes;
native polling never invokes arbitrary callbacks. See [native-sdk.md](native-sdk.md) for the wire
contract and current host-service limitations. The typed Rust secret broker is not yet projected
through `lm_engine_send_host_response`, so non-Linux wrappers cannot claim that capability.
