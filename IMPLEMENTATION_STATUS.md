# Implementation Status

## 2026-07-17 — First verified checkpoint

Assumption: "first verified checkpoint" refers to the explicit completion condition in
`PROJECT_GOAL.md` section 29, not completion of every Milestone 1 deliverable.

Verified on Linux with Rust 1.93.0:

- `cargo fmt --all --check` — passed.
- `cargo check --workspace --all-targets --all-features --locked` — passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — passed.
- `cargo test --workspace --all-targets --all-features --locked` — passed: 18 tests,
  0 failed, 0 ignored.
- `cargo build --workspace --locked` — passed.
- `cargo deny check advisories bans licenses sources` — passed all four checks; duplicate
  transitive versions were reported as non-blocking warnings.
- The CI workflow parsed as YAML and its tracked-file credential-signature scan passed locally.
- `cargo run --locked -p linguamesh-cli -- demo --text "Hello, LinguaMesh" --target zh-CN`
  — discovered the two fake models, streamed `你好，LinguaMesh！`, and completed.
- `cargo run --locked -p linguamesh-cli -- demo --text "Hello, LinguaMesh" --target zh-CN
  --model fake-slow-translator --cancel-after-ms 300` — retained the streamed partial output
  `你好` and emitted the cancelled terminal result.

Credential evidence: the demo requires no key, provider credentials use an in-memory redacted
secret type, SQLite stores only `secret_ref`, and the storage test rejects credential-value
columns. The bundled provider catalog uses a closed parsed schema and rejects credential-shaped
unknown fields. The global-goal SHA-256 matches `GLOBAL_GOAL.md`.

Protocol evidence: versioned Protobuf envelopes round-trip and reject incompatible versions. The
C ABI exposes opaque lifecycle, submit, poll, host-response, cancellation, shutdown, destroy, and
engine-scoped buffer release; tests cover invalid inputs, protocol versions, ownership, and repeated
shutdown. `contracts/abi/linguamesh.h` records the current native boundary.

This checkpoint does not claim complete Milestone 1 provider configuration, complete C ABI
behavior, native SDK artifacts, document support, or a stable release.

## 2026-07-17 — Milestone 2 partial checkpoint

Assumption: this partial checkpoint establishes tested wrapper source, verified prerelease SDK
artifacts, deterministic build definitions, and exact source metadata. It does not complete
Milestone 2 without complete host services, broader cross-platform conformance, sanitizer and fuzz
evidence, symbol bundles, release-manifest records, and the remaining platform projections.

Implemented:

- ABI-major and protocol-version negotiation with the legacy version query retained as an alias.
  The breaking engine-bound buffer release contract is ABI major 1; ABI 0 was a published
  prerelease source skeleton with no binary SDK or compatible client release, and has a checked-in
  migration note.
- A real `translate_text` Protobuf command and ordered started, delta, completed, cancelled, and
  failed event envelopes over the C ABI.
- One active operation per engine, a bounded native event queue, concurrent polling whose timeout
  includes receiver-lock waits, initial-request and streaming transport cancellation, and rejection
  of unsupported host responses.
- Explicit buffer ownership backed by per-engine allocation registries and token sequences, with no
  process-global mutable buffer state. Release rejects wrong-engine, forged, and duplicate
  descriptors without dereferencing or freeing client-owned pointers and remains available after
  engine shutdown until destruction. A 64-slot per-engine semaphore returns the stable
  `LM_RESULT_RESOURCE_EXHAUSTED` result before event consumption when every returned buffer remains
  outstanding.
- A generic C++20 RAII wrapper and C/C++ consumer smoke tests. It is usable behind a C++/WinRT
  bridge but is not a generated C++/WinRT projection.
- An Android AAR source layout configured for AGP 9.3 built-in Kotlin, Java-lite Protobuf types,
  typed public events, one isolated JNI bridge, pinned SDK/Gradle/NDK metadata, and cross-compile
  instructions.
- A concurrency-safe Swift lifecycle wrapper, local Swift Package binary target, XCFramework
  module/source-revision metadata, and a macOS build script.
- Linux direct-Rust integration guidance plus a deterministic native source/binary archive script.
- Immutable-commit-pinned GitHub Actions jobs that upload Linux, Android, and Apple artifacts,
  source-revision metadata, and SHA-256 manifests on pull requests, manual runs, and pushes to
  `main`. The existing core CI actions are immutable-commit-pinned as well.
- A long-running, loopback-only fake-provider CLI for desktop and Android-emulator conformance.
- Provider endpoint policy that permits HTTPS and loopback HTTP while rejecting unconfirmed remote
  HTTP, embedded credentials, queries, and fragments.

Verified on this Linux host with Rust 1.93.0, GCC/G++ 14.2.0, and the local fake provider:

- `cargo fmt --all --check` — passed.
- `cargo check --workspace --all-targets --all-features --locked` — passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` — passed.
- `cargo test --workspace --all-targets --all-features --locked` — passed: 27 tests, 0 failed,
  0 ignored. The FFI tests streamed `你好，LinguaMesh！`, observed exactly one completed terminal,
  retained a delta before exactly one cancelled terminal, bounded concurrent poll waits, and safely
  rejected wrong-engine, forged, and duplicate buffers while permitting owner release after
  shutdown. They also proved that 64 outstanding allocations are retained, the 65th reservation is
  rejected, and capacity is restored after release. A stalled-response test proved cancellation
  before HTTP response headers arrive.
- `cargo build --workspace --locked` — passed.
- `cargo deny check advisories bans licenses sources` — passed all checks with only the existing
  allowed duplicate-version and unmatched-license warnings.
- `bash tools/test-native-sdk.sh` — compiled the C11 header consumer and C++20 RAII consumer with
  warnings as errors, linked both to the Rust shared library, and ran both successfully.
- `bash tools/test-native-sdk-fake-provider.sh` — launched the standalone server on a
  system-selected loopback port, fetched both deterministic models, and observed clean shutdown.
- `bash tools/verify-linux-sdk-package.sh` — built the normalized archive twice with identical
  SHA-256 `6e003cfe4cad6639746536b841f205cdd5fe7ae0393c8c69bdb12459ac1703e8`; the outer and
  packaged per-file checksums, pkg-config metadata, and packaged static-library C consumer were
  then verified. This ignored dirty-worktree archive is test evidence, not a release artifact.
- Bash syntax, workflow YAML, immutable 40-character action references resolved from their release
  tags or toolchain branch, the `main` push trigger, three artifact-upload definitions, rendered
  Apple and packaged Linux metadata JSON, `git diff --check`, and the complete intended-worktree
  credential-signature scan passed static validation.

Verified remotely from clean source revision
`1c204f50e73797a77c66b919063071176efcd706`:

- Core CI run `29557243680` passed the Rust formatting, strict Clippy, 27-test, locked-build,
  dependency-policy, and credential-signature gates.
- Native SDK run `29557243604` passed Linux job `87811970748`, Windows job `87811970744`, Android
  job `87811970762`, and Apple job `87811970791`.
- Android artifact `8397932066` recorded the exact source revision, ABI major 1, protocol version 1,
  and prerelease status. Its AAR SHA-256 is
  `e659adbde0de708ea0d7c762545418a9e1d90afc88e135c5bc3a511d96f58e8d`. Independent inspection
  verified the checksum manifest, required wrapper and generated Protobuf classes, six native
  libraries across `arm64-v8a`, `armeabi-v7a`, and `x86_64`, expected FFI/JNI symbols, Android 26
  ELF targets, and basename-only `liblinguamesh_ffi.so` dynamic dependencies without build-host
  paths.
- Linux artifact `8397909478` has archive SHA-256
  `2b87c9a2e56955b6faf011895140e8f9d11ca7e441017a0b73a8226b47f76878`. Its outer and nine packaged
  checksums, exact source/ABI/protocol metadata, headers, libraries, exported symbols, and
  `x86_64-unknown-linux-gnu` target passed independent inspection.
- Apple artifact `8397926235` has XCFramework ZIP and SwiftPM checksum
  `cb5571ec602510300a80aaffbd2c68c158c5ed48f478a7509a2564ea5a799a9a`. Its checksum manifest,
  exact source/ABI/protocol metadata, Swift wrapper tests, and universal macOS `arm64` plus `x86_64`
  archive passed independent inspection.

These expiring workflow artifacts are integration evidence, not published release artifacts.

Not verified or not implemented:

- Android platform 36 and Java 21 are available locally, and an isolated non-native project copy
  enumerated AGP 9.3 tasks and passed `testDebugUnitTest`. The required NDK is unavailable locally,
  so local native cross-compilation, lint, and AAR assembly were not claimed; the canonical Android
  job above supplies that evidence.
- Windows/MSVC is unavailable locally. The remote Windows job validates the x64 DLL/import library
  and C/C++ consumer, but ARM64, NuGet packaging, generated C++/WinRT projection, application
  integration, and symbol packaging remain incomplete.
- Swift and Xcode are unavailable locally. The remote Apple job validates the wrapper,
  XCFramework, and universal archive, but client application linkage, signing, symbols,
  notarization, and distribution remain separate gates.
- C ABI projection of typed host secret brokerage and semantic/catalog/feature negotiation, file
  leases, generated Swift and C++ Protobuf types, sanitizer/fuzz coverage, Android/Apple symbol
  bundles, SBOMs, immutable release checksums, and cross-platform conformance remain incomplete.
- Engine-handle forgery, stale-handle use, repeated destruction, and destruction racing unjoined
  raw callers remain outside the ABI-major-1 contract and lack sanitizer-backed misuse tests.
- The Windows job emitted a non-blocking GitHub Actions annotation because the pinned
  `ilammy/msvc-dev-cmd` action still declares Node.js 20; GitHub ran it under Node.js 24. The action
  should be updated when an independently reviewed compatible revision is available.
- `shellcheck`, `actionlint`, and `pwsh` are unavailable locally; their platform-specific checks were
  not run. Bash parsing and YAML parsing passed, but only GitHub Actions runs can validate runner
  behavior.
- No package was released. The central compatibility and release manifests remain unreleased and
  require the compatible client checkpoints before they can record this source train.

## 2026-07-18 — Protected-span integrity slice

Assumption: automatic protection covers common URLs, email addresses, Markdown inline/fenced code,
and placeholder forms (`{name}`, `${name}`, `{{name}}`, and printf-style markers). User-managed
glossaries, custom product names, and provider families beyond the current OpenAI-compatible adapter
remain separate work.

Implemented:

- The shared domain scans untrusted source text and replaces recognized structured spans with
  collision-checked opaque markers. A bounded incremental restorer holds split marker fragments,
  restores each original span exactly once, rejects duplicates and unknown markers, and fails closed
  when a provider omits a required marker.
- The OpenAI-compatible adapter sends the protected source with an explicit marker-preservation
  instruction and restores markers before yielding streamed deltas. Safe structural failures map to
  typed `MalformedResponse` errors without including source content.
- Core compatibility advertises `protected_spans_v1`; Linux clients must negotiate this feature
  before using the updated adapter.

Validated locally with Rust 1.93.0:

- `cargo fmt --all` passed.
- `cargo check --workspace --all-targets --all-features --locked` passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passed.
- `cargo test --workspace --all-targets --all-features --locked` passed: 60 tests, 0 failed.
- Domain tests cover common span scanning, split-marker restoration, missing, duplicate, and
  unknown-marker rejection. The provider test captures the outbound JSON and verifies split-marker
  restoration through a real SSE response.

This slice does not claim complete glossary enforcement, user-selected protected terms, long-text
chunking, translation memory, document translation, or stable-release readiness.

## 2026-07-17 — Linux secure-provider Core prerequisite

Change identifier: `LM-CHANGE-2026-07-LINUX-SECURE-PROVIDER-1`

Assumption: this checkpoint supplies shared behavior required by the Linux-first client work. It
does not resume Android, Windows, or macOS client implementation and does not claim complete
Milestone 2 host-service support through the C ABI.

Assumption: declaring `url` directly in `linguamesh-domain` is not a new third-party package
introduction because the same locked version was already present through `reqwest`; it makes the
shared endpoint parser's existing supply-chain edge explicit.

Assumption: enabling rusqlite's existing `SQLITE_OPEN_NOFOLLOW` flag is a Linux persistence
security prerequisite within the reviewed bundled SQLite dependency. Its enforcement is
VFS-dependent, is tested only on Unix in this checkpoint, and changes neither schema, ABI,
protocol, nor the dependency graph.

Implemented:

- The Rust workspace advances to `0.1.0-alpha.2` for this source-breaking prerelease API change;
  alpha-1 consumer migration is documented. ABI major 1 and protocol version 1 remain unchanged.
- A `CoreCompatibility` snapshot covering the four version/catalog dimensions (Core semantic
  version, ABI major, protocol version, and provider-catalog version) plus stable enabled-feature
  identifiers. Prerelease clients require exact equality for the four dimensions and require their
  declared feature subset.
- Canonical non-secret `ProviderProfileId`, closed-namespace and random-UUID `SecretRef`, validated
  `EndpointConfiguration`, and `ProviderProfile` domain types. Embedded user information, queries,
  fragments, credential-shaped paths/fields, remote HTTP, and raw credential-shaped references are
  rejected. Profile and OpenAI adapter `Debug` output redact endpoints, display names, and
  credential values.
- SQLite schema version 2 with a transactional migration from schema 1, provider CRUD, active
  provider selection, per-profile last-model state, enabled-state enforcement, and foreign-key
  cascades. Untrusted alpha-1 references are cleared; `session:` references are rejected by
  persistence. On-disk connections use WAL, `synchronous=NORMAL`, secure deletion, and a truncating
  post-migration checkpoint. The actual migrated schema contains only a secret reference and no
  credential value, and passes `PRAGMA foreign_key_check`. A busy truncating checkpoint fails the
  open operation closed, and every later supported on-disk open retries cleanup even after the
  schema-2 transaction committed. Every on-disk open requests SQLite's no-follow flag. On Linux's
  default Unix VFS, any symbolic-link path component is rejected before migration or journal
  mutation; other platform VFS behavior remains unclaimed.
- A nontrivial `linguamesh-application` orchestration crate. Its bounded host-secret channel emits
  correlated leases, accepts one zeroizing in-memory secret or a closed typed failure, rejects late
  responses after cancellation, drains cancelled queue entries, validates endpoint policy before
  asking for a secret, and performs cancellable model discovery. `ProviderManager` owns at most one
  active credential-bearing engine, preserves it when a candidate fails, and on switch or
  disconnect cancels retained and in-flight operations before clearing the shared credential slot.
- An authenticated loopback fake-provider mode that proves the resolved Bearer canary reaches the
  intended provider without appearing in host-request metadata, tested `Debug` output, the SQLite
  database, or live WAL/SHM sidecar artifacts. A separate schema-1 fixture seeds a legacy credential
  value and proves migration clearing, secure deletion, and checkpointing remove it from every
  database artifact.

Locally verified with Rust 1.93.0:

- `cargo fmt --all --check` passed.
- `cargo check --workspace --all-targets --all-features --locked` passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passed.
- `cargo test --workspace --all-targets --all-features --locked` passed: 57 tests, 0 failed,
  0 ignored.
- `cargo build --workspace --locked` passed.
- `cargo deny check advisories bans licenses sources` passed all four checks with only the existing
  allowed duplicate-version and unmatched-license warnings.
- `bash tools/test-native-sdk.sh` and `bash tools/test-native-sdk-fake-provider.sh` passed the C,
  C++, and standalone loopback-provider smoke tests.
- `bash tools/verify-linux-sdk-package.sh` rebuilt the Linux SDK archive twice, verified its outer
  and per-file manifests plus packaged C consumer, and reproduced SHA-256
  `a22d5e4849b2c3cb0be36c86bd15e487749eba8939fdfae0d01ceef08471a6bf` from clean functional
  revision `fbf3e9b5927049dccaa19f8c36013495ffebba12`. Packaged metadata records that exact revision,
  workspace version `0.1.0-alpha.2`, ABI major 1, protocol version 1, and prerelease status.
- The tracked-file CI credential-signature scan and a matching intended-worktree scan passed. The
  credential canaries are assembled at compile time so the repository does not contain a literal
  credential signature.

Remote validation for functional revision `fbf3e9b5927049dccaa19f8c36013495ffebba12`:

- GitHub CI run `29572377637` passed Rust job `87858924329`, credential-signature job
  `87858924323`, and dependency-policy job `87858924320`.
- Native SDK run `29572377631` passed Linux job `87858924315` and produced Linux artifact
  `8403635653`. The same automatic run also rebuilt
  the frozen Apple, Windows, and Android wrappers successfully; no non-Linux client feature work
  was introduced by this checkpoint.

Remaining for the Linux secure-provider checkpoint:

- Keep the Linux client pinned to the reviewed no-follow Core revision, implement Secret Service
  for persistent credentials, and prove the Linux save/restart/translation path in native CI.
- The C ABI still rejects host-response messages and does not project semantic/catalog/feature
  negotiation. File leases and other complete Milestone 2 host services remain unimplemented.

## 2026-07-18 — Linux history controls contract

Assumption: Linux history inspection uses the existing bounded schema-3 table and returns at most
`MAX_TRANSLATION_HISTORY_ENTRIES`; clients remain responsible for presenting and exporting entries.

Implemented:

- `Storage::translation_history` returns newest-first entries with stable operation IDs, timestamps,
  locales, model IDs, source text, and translated text.
- `Storage::delete_translation_history_entry` deletes exactly one operation ID and reports whether a
  row existed; SQL parameters remain bound and no credential columns are introduced.
- Storage regression coverage verifies newest ordering, timestamp presence, exact deletion, and a
  missing-entry no-op alongside the existing Incognito, clear, size, and count tests.

Validated locally with Rust 1.93.0:

- `cargo fmt --all` passed.
- `cargo test -p linguamesh-storage --all-targets --all-features --offline` passed: 14 tests,
  0 failed.
- `git diff --check` passed.

## 2026-07-18 — Linux history policy contract

Assumption: disabling history changes only future standard-request persistence; existing entries
remain available for inspection, export, and deletion, while Incognito remains an unconditional
per-request opt-out.

Implemented:

- Schema 4 adds a singleton `translation_history_policy` table with an enabled-by-default flag.
- `Storage::translation_history_enabled` and `Storage::set_translation_history_enabled` expose
  the persisted policy without storing source text, output, or credentials in the setting.
- `Storage::record_translation_history` checks the policy before applying the existing bounded
  write path; disabled history does not delete existing entries.
- Storage tests cover default enablement, disabled-write behavior, reopen persistence, and re-enable.

Validated locally with Rust 1.93.0:

- `cargo fmt --all` passed.
- `cargo test -p linguamesh-storage --all-targets --all-features --offline` passed: 15 tests,
  0 failed.
- `git diff --check` passed.

## 2026-07-18 — Linux translation memory storage contract

Assumption: translation memory is an optional local cache for standard requests; Incognito never
reads or writes it, disabling the policy preserves existing entries, and cache identity changes
when any relevant request input or versioned translation policy changes.

Implemented:

- Schema 5 adds a singleton translation-memory policy and a bounded, inspectable entry table.
- Identity includes normalized source text, locales, model/provider identity, chunking options,
  serialized glossary, protected-span policy, prompt-template version, and quality mode.
- Storage exposes lookup, bounded write, newest-first inspection, exact deletion, clear-all, and
  persisted enable/disable APIs without storing credentials.
- Regression tests cover identity mismatches, policy persistence, Incognito isolation, exact delete,
  clear-all, size limits, and schema migration.

Validated locally with Rust 1.93.0:

- `cargo fmt --all -- --check` passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` passed.
- `cargo build --workspace --locked` passed.
- `cargo test --workspace --all-targets --all-features --offline` passed: 59 tests, 0 failed.
- `git diff --check` passed.
