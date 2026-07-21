# Testing

Default tests use only local deterministic fixtures and the loopback fake provider. They must not require commercial credentials or the public internet. Provider contract tests cover fragmented SSE and Ollama NDJSON, split UTF-8, malformed messages, disconnects, errors, response limits, and cancellation. Persistence tests use isolated temporary or in-memory SQLite databases. Document persistence tests cover schema 6-to-9 migration, bounded job/segment snapshots, exact segment updates, structural-segment protection, SRT/WebVTT timestamp validation, CSV quoting and selected-column reconstruction, pause persistence, validated non-secret option round trips, quality-mode persistence, and restart recovery without persisting paths or credential values. Schema 15 routing-profile tests cover bounded JSON persistence, validation, listing, deletion, and the absence of endpoint or credential fields.

OOXML document tests also reject encrypted, symlinked, duplicate, traversal, and suspiciously
compressed ZIP entries before XML inspection. DOCX, PPTX, and XLSX packages enforce the 4 MiB and
512-entry limits plus a bounded 200:1 uncompressed-to-compressed ratio for entries at least 1 KiB;
the compression-ratio fixture uses an in-memory deflated resource and never writes a filesystem path.

Protected-span tests scan URLs, email addresses, Markdown code, and placeholders, split opaque
markers across streamed deltas, restore every original span exactly once, and reject missing,
duplicate, or unknown markers. The OpenAI-compatible provider test captures the outbound request to
confirm the source span is not sent as ordinary model text and then exercises restoration through a
real local SSE response. Anthropic tests capture the Messages headers and JSON body, decode
fragmented UTF-8 SSE events, enforce the completion marker, redact diagnostics, and cancel while
waiting for response headers. Native Ollama tests cover `/api/tags` discovery, fragmented UTF-8
NDJSON, completion markers, and cancellation through the deterministic fixture.
The Gemini fixture covers `/v1beta/models` filtering, fragmented Generate Content SSE candidates,
and the terminal `finishReason` event; the Linux worker regression exercises the same fixture
through `ProviderManager` without a credential.
The Azure fixture covers a deployment-scoped Chat Completions path, `api-key` authentication,
the pinned API-version query, manual deployment listing, streamed fragments, and redacted
credential handling without a commercial credential.
The OpenAI Responses fixture covers `/v1/responses`, Bearer authentication, model discovery,
fragmented typed SSE events, terminal `response.completed`, and redacted credential handling
without a commercial credential.

Quality-mode tests cover the versioned `translation-prompt-v2` wording, stable request names, and
mode-specific Fast/Balanced/Best directives across the shared provider helper. Engine tests verify
that non-empty source text cannot reach `Completed` with empty output or Unicode replacement
characters; received deltas remain available before the typed failure. This validation is
deterministic and does not create hidden additional provider calls.

Usage-record tests cover source preservation, token-count bounds, local estimation, provider-wire
normalization, and JSON backward compatibility for completed events without a usage field. OpenAI
Chat/Responses, Anthropic, Gemini, and Ollama decoder fixtures cover fragmented usage metadata,
partial-count merging, and final-text-plus-usage events. Azure reuses the Chat Completions parser.
These tests do not claim provider billing equivalence or pricing accuracy.

Translation-preset tests cover General/Technical/Marketing stable IDs, legacy-request defaults,
bounded optional fields, control and credential-shaped input rejection, escaped prompt rendering,
and translation-memory identity separation. The engine validates the preset before any provider
request, and provider fixtures verify that all built-in adapters receive the same request contract.

Schema 16 migration tests construct a schema-15 database and verify the routing-profile ID; schema
17 migration tests apply the quality-mode column and round-trip `fast`, `balanced`, and `best`
without introducing endpoint or credential columns. Schema 18 migration tests round-trip the full
translation preset, default legacy NULL values to `General`, enforce the 8 KiB bound, and reject
credential-shaped custom instructions across reopen.

Routing planner tests cover Manual/Ordered/Automatic mode selection, stable quality ranking,
explicit fallback ordering, capability filtering, privacy-sensitive remote rejection, malformed
allow/deny lists, locale tags, zero request limits, and invalid
duplicate or empty profiles. They use only non-secret identifiers and synthetic request metadata.

Native ABI tests submit a real Protobuf translation command to the loopback fake provider, assert
ordered deltas and exactly one terminal event, and verify cancellation. Run Linux C and C++ consumer
smoke tests with `bash tools/test-native-sdk.sh`. The FFI suite also verifies bounded concurrent
polling, isolates allocation ownership between engines, rejects forged or duplicate buffer
descriptors without freeing client memory, permits release after engine shutdown, and proves that
the 65th outstanding-buffer reservation fails without growing the registry beyond 64. The FFI suite
also queries and decodes the five-dimension `CompatibilitySnapshot` through the C ABI before any
translation work. Run
deterministic Linux packaging twice, then verify its outer and per-file manifests, with
`bash tools/verify-linux-sdk-package.sh`. That verification also validates the pkg-config metadata
and links the packaged static library into the C consumer smoke test.

The FFI suite additionally sends 4,096 deterministic pseudo-random byte strings through
`lm_engine_submit`, bounded at the 1 MiB protocol limit, and requires every malformed or unsupported
input to return a controlled result without a panic or provider request. This is a regression stress
corpus, not coverage-guided fuzzing; sanitizer and fuzz-run coverage remain required before a stable
release.

The separate `fuzz/` workspace runs the `protocol_decoders` libFuzzer target over the versioned
Envelope plus command and event payload decoders. It rejects inputs above the 1 MiB protocol bound
before decoding and runs with cargo-fuzz's AddressSanitizer instrumentation. Reproduce the local
smoke with the pinned fuzz-only nightly toolchain:

```sh
rustup toolchain install nightly-2026-07-20 --profile minimal
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly-2026-07-20 fuzz run protocol_decoders -- -runs=2000 -max_total_time=30
```

The `Fuzz and sanitizers` workflow runs the same bounded smoke on every Core push and pull request.
The production workspace remains pinned to stable Rust 1.93.0; nightly is isolated to this fuzz
harness and is not used for release artifacts.

Run `bash tools/test-native-sdk-fake-provider.sh` to verify that the standalone loopback provider
reports a usable endpoint, serves the deterministic model catalog, and shuts down cleanly.

Android AAR, Windows DLL/import-library, and macOS XCFramework builds require their platform jobs.
Do not treat YAML parsing, source review, or a Linux-only build as evidence that those artifacts
compile.

The Linux secure-provider prerequisite is covered by normal workspace tests. They verify every
compatibility dimension, canonical profile validation and redacted `Debug`, schema-1-to-2 migration,
on-disk reopen, active-profile and per-profile model persistence, cascade/disable behavior, and on
Unix, rejection of a symbolic-link database before migration. They also prove the absence of a
synthetic credential-shaped canary from every database-directory artifact after authenticated use.
A Linux-only storage regression additionally opens a descriptor-backed `/proc/self/fd/<fd>` path
after the host pins its private database inode; ordinary paths still require the Core no-follow
flag, and non-descriptor trusted paths are rejected.
A schema-1 fixture starts with a credential value and proves the
secure-delete migration
plus truncating checkpoint removes it from all database artifacts, including after a reader makes
the first checkpoint busy and the next on-disk open retries it. The application tests run an
authenticated loopback provider and prove correlated one-time
secret delivery, strict queue capacity, typed host failure, cancellation of pending secret/model
discovery work, rejection of late secrets, in-flight cancellation and credential clearing on
provider disconnect, and adapter rejection before any secret request. Domain tests reject unsafe
endpoints before a profile can reach the application layer.

The storage suite also keeps a reader transaction open while a writer commits a provider profile,
then closes the writer before reopening the database. The
`wal_replay_preserves_committed_profile_after_writer_disconnect` regression requires the `-wal`
sidecar to exist before the reader closes and verifies that the committed model and SecretRef are
restored by the next `Storage::open`. This is bounded WAL-replay evidence after a writer
disconnect, not a claim that every filesystem power-loss or SQLite VFS failure is covered.

The Core domain suite also covers the `FileLease` lifecycle for desktop, temporary/output, POSIX,
Android ParcelFileDescriptor, and Windows-handle resource shapes. It rejects invalid resource
identifiers before work and proves that an acquired guard cannot read its resource after expiry or
explicit revocation. Native ABI tests additionally cover engine-scoped numeric lease tokens,
bounded registry exhaustion, wrong-engine rejection, expiry/revocation, post-shutdown cleanup,
and the C++ RAII wrapper. Unix ABI tests also copy a bounded document from a registered POSIX
descriptor, consume the lease exactly once, and leave oversized input retryable.

These tests exercise an in-process fake host. Linux Secret Service behavior, session-only fallback,
and native restart restoration remain client-repository gates. The FFI regression additionally
emits a versioned `secret_required` event, accepts one matching `host_secret_response`, rejects a
replay, and completes an authenticated loopback translation without exposing the canary.
