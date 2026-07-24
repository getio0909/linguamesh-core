# Implementation Status

## 2026-07-24 — ABI status reconciliation

Assumption: Core `f5b818c3598d78e7cac30604577fa8057d380737` is the current Linux-consumed runtime
revision; this correction aligns the historical release audit with the implemented ABI surface and
does not change source or dependency behavior.

- The ABI 1 file-lease surface is implemented and tested: engine-scoped numeric tokens cover
  desktop/temporary/output paths, POSIX descriptors, Android parcel descriptors, Windows handles,
  bounded document consumption, one-shot consumption, expiration, revocation, and cleanup.
- Bounded FFI fuzz coverage is implemented for malformed inputs, safe lifecycle/control sequences,
  valid loopback commands, and opaque handle lifetime; sanitizer-backed misuse of unjoined raw
  callers remains outside the ABI contract.
- Generated Swift/C++ projections, remaining platform symbol/package/distribution work, signed
  artifacts, cross-platform conformance, and stable-release evidence remain open.

## 2026-07-24 — Linux alternate SQLite VFS regression

Assumption: SQLite's bundled `unix-excl` VFS is a representative Linux alternate VFS for the
tested storage contract; custom or third-party VFS implementations and physical power-loss
behavior require separate evidence.

- Added the Linux-only `unix_exclusive_vfs_preserves_migrations_and_committed_profiles` regression.
  It opens the database through `unix-excl` with `SQLITE_OPEN_NOFOLLOW`, applies the full schema
  migration and `WAL`/`synchronous=FULL` configuration, persists a provider profile, reopens it,
  and rejects both a symbolic-link database alias and a symbolic-link parent path before migration.
- Added `unix_exclusive_vfs_wal_replay_survives_process_termination_after_commit`, which aborts a
  child process after a committed WAL transaction and verifies profile recovery through the same
  bundled VFS.
- Added `unix_dotfile_vfs_fails_closed_without_required_wal`, which probes the bundled
  `unix-dotfile` VFS and verifies that Core rejects it before migrations when it cannot provide the
  required WAL journal mode; no schema tables are created. This is an explicit unsupported-VFS
  boundary, not a claim of `unix-dotfile` compatibility.
- Added `unix_none_vfs_fails_closed_without_required_wal`, applying the same pre-migration
  fail-closed check to SQLite's non-locking `unix-none` VFS; the rejected database remains free of
  schema tables.
- Focused and full storage validation passed with the host-pinned Rust 1.93.0 command:
  `cargo +1.93.0 test -p linguamesh-storage --locked --offline` (`59 passed; 0 failed`).
- This closes only the tested bundled `unix-excl` VFS path. Physical power-loss recovery,
  custom/third-party VFS behavior, cross-client conformance, signing, rollback, and stable release
  remain open.

## 2026-07-24 — ABI 1 opaque engine-handle lifetime hardening

Assumption: the C ABI remains source-compatible when its opaque `LmEngine *` value becomes a
registry token; clients never dereference the handle, and worker shutdown coordination remains a
caller responsibility.

- Replaced raw `Box<LmEngine>` pointer dereferences with a process-local `Arc` registry keyed by
  monotonic opaque handle tokens. Calls that already acquired a registry entry keep the state alive
  during concurrent destroy; stale, forged, and repeated-destroy handles fail closed without
  dereferencing freed memory.
- Added regression coverage for stale handles and concurrent destroy/control calls. The FFI suite
  now passes 22 tests, including the existing buffer, lease, protocol, secret, and cancellation
  checks. The new `ffi_handles` AddressSanitizer target destroys an engine at arbitrary points and
  completed 1,068 time-bounded iterations locally without a crash or leak report (coverage peaked at
  2,120 features); CI runs 2,000 iterations or 30 seconds.
- Updated the ABI migration, native SDK contract, fuzz workflow, and testing guidance. Local
  `cargo fmt --all -- --check`, fuzz-workspace bin check, strict FFI Clippy, full locked offline
  workspace tests, and `bash tools/test-native-sdk.sh` passed. Release remains `unreleased`; native
  client close/worker coordination, cross-client conformance, signed artifacts, rollback, and
  stable-release evidence remain separate gates.

## 2026-07-24 — Valid FFI command fuzz and local fake-provider smoke

Assumption: the loopback `FakeProviderServer` is a deterministic, network-free provider fixture;
the fuzz target bounds source text to 4 KiB and event polling to 16 events, so it does not claim
coverage for commercial providers, live credentials, or cross-client projections.

- Added the `ffi_commands` libFuzzer target. Each generated UTF-8-safe source string builds a valid
  ABI 1 `TranslateText` envelope, submits it through `lm_engine_submit`, frees every returned
  buffer, requires exactly one terminal event, and destroys the engine after completion.
- Local pinned nightly smoke `cargo +nightly-2026-07-20 fuzz run ffi_commands -- -runs=200
  -max_total_time=20` completed without a crash after 136 time-bounded iterations, reaching
  10,653 coverage features and a 49-file minimized corpus. Stable offline workspace checks also
  passed after the fuzz workspace added its testkit and Tokio fixture dependencies.
- Existing `ffi_inputs` and `ffi_sequences` smokes still pass 200 iterations each without a crash;
  valid-command fuzz remains local-fixture evidence only until the remote sanitizer and Native SDK
  jobs complete. Raw engine-pointer use-after-free, cross-client conformance, signed artifacts,
  rollback, and stable release remain separate gates.

## 2026-07-24 — FFI input fuzz and AddressSanitizer smoke

Assumption: malformed and unsupported C ABI envelopes are the safe fuzz boundary; valid
`TranslateText` envelopes are skipped so the harness cannot create arbitrary network traffic or
consume provider credentials.

- Added the `ffi_inputs` libFuzzer target to the isolated `fuzz/` workspace. It reuses one real
  `lm_engine_create` handle, sends bounded arbitrary bytes through `lm_engine_submit`, asserts the
  controlled rejection result set, and leaves valid translation commands to the existing provider
  fixtures.
- Local `cargo +nightly-2026-07-20 check --manifest-path fuzz/Cargo.toml --offline` passed.
- Local AddressSanitizer smoke `cargo +nightly-2026-07-20 fuzz run ffi_inputs -- -runs=200
  -max_total_time=10` passed 200 runs with no crash, reaching 299 coverage features and a 29-file
  minimized corpus. The CI gate runs 2,000 iterations or 30 seconds.
- Core CI `30060612966` passed the stable Rust 1.93.0 formatting, strict Clippy, workspace tests,
  and build gates. Fuzz/AddressSanitizer run `30060612978` passed protocol, document, and FFI
  targets in job `89381326908`; Native SDK run `30060612972` passed Windows `89381326909`,
  Android `89381326913`, Apple `89381326948`, and Linux `89381326956` jobs.
- This strengthens malformed-input FFI coverage; valid-command behavior now has a separate bounded
  loopback fuzz target, while raw-handle misuse, cross-client conformance, signed artifacts, and
  stable release remain separate gates.

The follow-up `ffi_sequences` target fuzzes safe lifecycle/control-call sequences without destroying
the active engine mid-sequence. It covers idempotent shutdown, cancellation, polling, compatibility
buffers, forged descriptors, file-lease token operations, and malformed host responses; arbitrary
raw engine-pointer use-after-free remains outside the C ABI's safe caller contract. Local pinned
nightly ASAN smoke passed 200 runs with 2,326 coverage features and a 47-file minimized corpus;
the CI gate runs 2,000 iterations or 30 seconds.

## 2026-07-23 — typed provider rate-limit category

Assumption: HTTP 429 is the stable cross-provider signal for temporary throttling; quota exhaustion,
provider billing, and stable-release policy remain separate contracts.

- Added `ErrorKind::RateLimited` and a shared HTTP-status normalizer used by OpenAI-compatible,
  Anthropic, Gemini, and Ollama adapters. Existing bounded `Retry-After` parsing is preserved.
- Persisted provider-health categories and ABI error serialization now round-trip `rate_limited`
  without exposing provider response bodies or credentials. Release remains `unreleased`.
- `cargo fmt --all -- --check`, `cargo check --workspace --locked --all-targets`, strict workspace
  Clippy, and `cargo test --workspace --locked --all-targets` pass locally; the provider-api
  mapping test covers 401, 404, 429, and generic network statuses.

## 2026-07-23 — bounded TBX glossary import

Assumption: Linux is the active client priority, so Core exposes a restricted, dependency-light
TBX reader while other clients remain unchanged; the first `langSet` is the source language and
each later language set contributes target rules.

- `linguamesh-domain` now imports UTF-8 TBX under the existing 4 MiB/256-entry bounds, preserves
  `xml:lang` and the first `descrip` note, decodes XML character references, and rejects DTDs,
  unknown entities, malformed entries, missing source/target terms, conflicts, and credential-shaped
  values without storing file contents.
- Domain tests cover multilingual terms, notes and escaped text plus malformed, unsafe, missing,
  oversized, and bounded-entry inputs. `cargo test --workspace --locked --all-targets` passed
  (228 tests, 0 failed, 0 ignored), and strict all-feature Clippy passed. Release remains `unreleased`.

## 2026-07-23 — normalized usage-record persistence

Assumption: usage metadata is safe to retain only as bounded, non-secret accounting metadata; the
history policy and Incognito mode are the persistence controls, and no provider pricing is inferred.

- Core schema 32 adds `usage_records` with the operation ID, sanitized provider ID, model ID,
  source (`provider_reported`, `locally_estimated`, or `unknown`), and bounded token counts. It
  never stores source text, translated text, endpoints, or credential values.
- History completion writes the history row and usage row in one transaction; history trimming,
  individual deletion, and **Clear history** remove orphaned usage records as well. Incognito
  requests write neither record, and malformed provider identities are dropped rather than stored.
- Storage tests cover schema migration, provider-identity redaction, source/count round trips,
  incognito skipping, cleanup, and deletion. `cargo test --workspace --locked --all-targets`
  passed (222 tests) and strict workspace Clippy passed; release remains `unreleased`.

## 2026-07-23 — Anthropic Messages testkit transport fixture

Assumption: the Linux-first protocol boundary needs a deterministic `/v1/messages` service so the
production GTK path can be verified without commercial credentials; live Anthropic account,
quota, and model availability remain outside this fixture.

- `linguamesh-testkit` now exposes `start_anthropic()`, authenticates only with `x-api-key`, and
  emits fragmented `content_block_delta` events plus `message_start`, usage, and `message_stop`
  events. The regression checks unauthorized and authorized requests and confirms the expected
  stream markers without logging the canary key.
- The adapter's existing Messages unit coverage remains unchanged; this fixture is shared by the
  Linux production GTK protocol-preset test. Release remains `unreleased` pending remote gates
  and the broader cross-client acceptance matrix.

## 2026-07-23 — Linux client-certificate TLS identity checkpoint

Assumption: enterprise provider endpoints may require mutual TLS; the smallest safe Linux-first
boundary is one combined PEM certificate/private-key identity resolved through a persistent or
session `SecretRef`, never stored as a profile value.

- Core schema 31 migration `0031_provider_profile_client_certificate_identity.sql` persists only
  `ProviderProfile.client_certificate_identity_ref`; storage rejects session references. The
  bounded domain parser requires certificate and private-key PEM sections and redacts diagnostics.
- OpenAI Chat/Responses/Azure, Anthropic, Gemini, and Ollama apply reqwest rustls identities while
  retaining system roots, hostname verification, redirect blocking, and TLS verification.
- Local `cargo fmt --all`, full workspace tests (including schema 31 migration and identity
  rejection/round-trip tests), strict all-feature Clippy, and the repository secret-pattern scan
  passed. Core CI/Fuzz/Native SDK runs `29978060455`/`29978060459`/`29978060500` passed for
  `2a3534faa9a2531cbbc6cc06d325ad7c82c69394`; Linux and l10n consumers are pinned to their
  verified revisions, and release remains `unreleased`.

## 2026-07-23 — Proxy authentication SecretRef checkpoint

Assumption: Linux needs optional proxy Basic authentication while proxy URLs remain credential-free;
the smallest complete boundary is one bounded `username:password` host secret referenced by
`ProviderProfile.proxy_auth_ref`, with session-only input unless the user explicitly remembers it.

- Core schema 30 migration `0030_provider_profile_proxy_auth.sql` persists only a persistent
  `SecretRef`. Domain parsing bounds and redacts the username/password pair, and storage rejects
  session references or credential values at the persistence boundary.
- OpenAI Chat/Responses/Azure, Anthropic, Gemini, and Ollama resolve the host secret once and apply
  it to the configured HTTP proxy; embedded proxy URL userinfo remains rejected.
- Local `cargo fmt --all`, `cargo check --workspace`, full workspace tests (all targets/features),
  and strict Clippy passed. Core CI/Fuzz/Native SDK runs `29975202072`/`29975202087`/`29975202129`
  passed for this exact commit; Linux client, l10n resources, and central coordination evidence
  are recorded in their sibling repositories. Release stays `unreleased`.

## 2026-07-23 — Provider streaming idle timeout checkpoint

Assumption: a bounded streaming idle timeout of 1–300 seconds (default 60) is the smallest
complete follow-up to connection timeout; the budget resets after each received response chunk,
while TLS policy remains separate.

- Core schema 28 migration `0028_provider_profile_streaming_idle_timeout.sql` persists the
  validated idle timeout beside request and connection timeouts. Domain/storage tests cover the
  default, range rejection, migration, and profile round-trip without storing credentials.
- OpenAI Chat/Responses/Azure, Anthropic, Gemini, and Ollama streams apply a per-chunk
  `tokio::time::timeout` and return the typed English `Timeout` diagnostic when the body stalls;
  request-total and connection timeouts remain independent. A stalled-body OpenAI regression
  proves the typed timeout path.
- Local `cargo fmt --all`, `cargo check --workspace`, and full `cargo test --workspace` passed.
  Remote CI/Fuzz/Native SDK evidence for `b247155ad429639fdb65d3b063c3efc580ce46a4` is recorded
  after the GitHub runs complete; release remains `unreleased`.

## 2026-07-23 — Provider connection timeout checkpoint

Assumption: a bounded connection-establishment timeout of 1–120 seconds (default 10) is the
smallest next ProviderProfile slice; streaming-idle timeout and TLS policy remain separate.

- Core schema 27 migration `0027_provider_profile_connection_timeout.sql` persists the validated
  connection timeout beside the existing bounded total request timeout. Domain and storage tests
  cover defaults, range rejection, migration, and profile round-trip without storing credentials.
- OpenAI Chat/Responses/Azure, Anthropic, Gemini, and Ollama configuration builders now carry the
  value into `reqwest::ClientBuilder::connect_timeout`; request-total timeout remains independent.
- Local `cargo fmt --all`, `cargo check --workspace --locked --offline`, full workspace tests
  (domain 50, storage 44, application 15, provider and document suites), and strict all-feature
  Clippy passed. Core CI/Fuzz/Native SDK runs `29969609373`/`29969609372`/`29969609379` passed
  for the exact revision; Linux demo-provider tests (`159 passed; 3 ignored`), strict Clippy, and
  l10n validation also passed.

## 2026-07-22 — ABI 1 provider metadata projection

Assumption: ABI 1 can add optional Protobuf fields without changing the envelope or protocol
version; older clients omit the fields and Core treats them as absent.

- `TranslateTextCommand` now carries optional non-secret `organization`, `project`, and
  `custom_headers_json` fields. The C ABI validates organization/project values against the shared
  credential-shape and size rules, validates custom headers before requesting a host secret, and
  forwards the accepted metadata into the OpenAI-compatible adapter.
- The Android wrapper exposes source-compatible optional parameters, while raw envelope clients
  can populate the same fields directly. Credentials remain only in `secret_ref`/one-shot host
  responses and are never serialized into these metadata fields.
- Domain/protocol/FFI tests pass, including metadata forwarding against a header-enforced fake
  provider and fail-closed credential-shaped metadata before any secret request.

This is a prerelease ABI capability projection; other native clients still require integration
and platform-specific validation before cross-client acceptance or stable release.

## 2026-07-22 — Provider profile custom-header checkpoint

Assumption: Linux needs a bounded non-secret JSON map for provider-specific routing headers; header
names and values that resemble credentials or override built-in authentication metadata must fail
closed. Secret custom headers and proxy settings remain separate work.

- Core schema 23 migration `0023_provider_profile_custom_headers.sql` persists canonicalized
  custom headers without any credential-value column. Domain validation bounds the map to 16 HTTP
  token headers, rejects control/credential/authentication values, and redacts presence in Debug.
- OpenAI Chat Completions and Responses apply the validated headers while preserving built-in
  organization, project, and authentication handling. Domain, storage, provider, and application
  regressions passed locally; release remains `unreleased`.

## 2026-07-22 — Provider project application wiring correction

Assumption: the persisted non-secret `project` identifier must reach both OpenAI-compatible Chat
Completions and Responses requests before the Linux provider-project checkpoint can be treated as
functionally complete.

- `ProviderManager::connect` now forwards `ProviderProfile.project` into both adapter configuration
  builders; organization forwarding and adapter-neutral metadata behavior remain unchanged.
- The fake provider can require `OpenAI-Project`; application regressions cover authenticated Chat
  streaming and Responses typed-SSE discovery/translation without storing or logging credentials.
- `cargo fmt --all -- --check`, the offline workspace test suite, strict all-feature Clippy, and the
  targeted application tests (14 passed) passed locally. Remote CI and Linux repinning remain
  pending for this correction; release remains `unreleased`.

## 2026-07-22 — Provider profile region/account checkpoint

Assumption: `region` and `account_identifier` are optional, bounded, non-secret provider metadata;
provider adapters do not interpret either value until a provider-specific contract is defined.

- Added Core schema 22 migration `0022_provider_profile_region_account.sql`, domain validation,
  storage round-trip coverage, and redacted debug presence reporting.
- Session and persistent profile reconstruction retain both fields without touching credential
  values; Linux and localization bindings are tracked in their own checkpoints.
- Release remains `unreleased` pending cross-client compatibility, provider-specific semantics, and
  mandatory acceptance evidence.

## 2026-07-22 — Provider profile project checkpoint

Assumption: `project` is an optional, bounded non-secret OpenAI-compatible project identifier.
Core forwards it only as `OpenAI-Project` for Chat Completions and Responses; Azure and other
adapters ignore it until a provider-specific contract is defined.

- Added Core schema 21 migration `0021_provider_profile_project.sql`, domain validation, storage
  round-trip coverage, and redacted debug presence reporting.
- Added OpenAI adapter configuration builders and request-header coverage; no credential or
  endpoint value is logged or persisted through this field.
- Linux and l10n bindings are tracked in their own implementation-status checkpoints. Release
  remains unreleased pending cross-client compatibility and mandatory acceptance evidence.

## 2026-07-22 — Provider profile organization checkpoint

Assumption: `organization` is an optional, bounded non-secret OpenAI-compatible routing/account
identifier. Core forwards it only as `OpenAI-Organization` for Chat Completions and Responses; other
adapters ignore it until their own contract is specified.

- Added Core schema 20 migration `0020_provider_profile_organization.sql`, domain validation, storage
  round-trip coverage, and redacted debug presence reporting.
- Added OpenAI adapter configuration builders and request-header coverage; no credential or endpoint
  value is logged or persisted through this field.
- Linux and l10n bindings are tracked in their own implementation-status checkpoints. Release remains
  unreleased pending cross-client compatibility and mandatory acceptance evidence.

## 2026-07-22 — Provider profile notes checkpoint

Assumption: a single optional, bounded non-secret note was the smallest Linux-first slice of the
ProviderProfile metadata contract at the time of this checkpoint; later checkpoints add fields
independently.

- Added Core schema 19 migration `0019_provider_profile_notes.sql` and persisted optional
  `ProviderProfile.user_notes` without storing credentials or exposing it in debug output.
- Added domain validation for the 2 KiB profile-note bound, control/credential-shaped rejection,
  and whitespace normalization to an absent note.
- Added storage and domain regression tests for migration version 19 and note round-trip behavior.
- Pending: Linux form binding, localization, CI evidence, and cross-repository compatibility update.

## 2026-07-21 — SQLite WAL process-crash recovery regression

Assumption: abrupt Unix process termination after a committed WAL transaction is a useful
automatable crash-recovery boundary, but it does not emulate physical power loss or every SQLite
VFS failure mode.

- Added `wal_replay_survives_process_termination_after_commit`. A child test process keeps a reader
  snapshot open, commits a provider profile with `synchronous=FULL`, terminates abruptly, and the
  parent reopens the database to verify the model and persistent `SecretRef` are recovered.
- Updated [`Core testing`](docs/testing.md) to distinguish process-crash evidence from physical
  power-loss qualification. No credentials or source/translated text are introduced by the test.
- Targeted storage regression, formatting, workspace check, strict Clippy, and full offline
  workspace tests passed locally.

This is unreleased Unix crash-recovery evidence. Physical power-loss simulation, alternate SQLite
VFS coverage, cross-client conformance, signing, rollback, and stable release remain open.

## 2026-07-21 — provider-reported usage normalization

Assumption: provider usage fields are advisory wire metadata; malformed, absent, or partial values
must never block a translation or become a pricing claim.

- Provider streams now carry typed text or usage events through `TranslationStreamEvent`; the
  engine merges partial records and preserves `UsageSource::ProviderReported` when wire counts
  are available, otherwise retaining the existing bounded local estimate.
- OpenAI Chat Completions requests opt into `stream_options.include_usage`; OpenAI Responses,
  Anthropic Messages, Gemini Generate Content, and native Ollama terminal metadata are parsed into
  the same `UsageRecord` shape. Azure uses the OpenAI-compatible path without changing its API-key
  contract.
- Added decoder regressions for fragmented SSE/NDJSON usage, partial Anthropic counts, final
  Gemini text plus usage, and Responses completion metadata. The stable C ABI/protobuf projection
  remains unchanged and no pricing table or source text is persisted.

- Targeted provider/engine tests passed after formatting; full workspace validation follows before
  the revision is pinned by Linux.

This is prerelease wire-parsing evidence. Provider accounting semantics, pricing estimates, native
non-Rust ABI projection, and stable-release qualification remain open.

## 2026-07-21 — normalized translation usage records

Assumption: usage metadata is non-sensitive and must distinguish provider-reported counts from a
bounded local estimate; no provider pricing or source/translated content is persisted.

- Added `UsageSource` and bounded `UsageRecord` domain types and an optional `usage` field on
  completed Rust translation events. Missing usage remains backward-compatible with older JSON.
- Added the `usage_records_v1` compatibility feature. The engine currently emits a conservative
  local estimate; provider adapters can populate the provider-reported constructor in a later
  checkpoint. The stable C ABI/protobuf projection intentionally remains unchanged.
- `cargo fmt --all`, `cargo check --workspace --all-targets --all-features --offline`, strict
  workspace Clippy, and `cargo test --workspace --all-targets --all-features --offline` passed.

This is a prerelease Rust-host contract. Provider-reported parsing, pricing estimates, native
non-Rust ABI projection, and stable-release qualification remain open.

## 2026-07-21 — SQLite WAL replay after writer disconnect

Assumption: process interruption can leave a committed transaction in the SQLite WAL; the next
Core open must replay that sidecar without losing the provider model or SecretRef.

- Added `wal_replay_preserves_committed_profile_after_writer_disconnect`. A private reader holds a
  snapshot while the writer commits, the writer closes before checkpointing, the `-wal` sidecar is
  required to exist, and the next `Storage::open` restores the committed profile.
- Local `cargo fmt --all -- --check`, the targeted locked offline storage test, the full locked
  offline workspace tests, and strict Clippy passed. The reproducible Linux SDK package smoke also
  passed with SHA-256 `9857c972ce16ae3d0243fecfe76755f301abe94ca3a3c10f880f62a2836914f`.
- Remote Core CI/Fuzz and sanitizers/Native SDK runs `29812421226`/`29812421184`/`29812421320`
  passed for this exact commit.

This is bounded WAL-replay evidence, not a claim for physical power-loss or every SQLite VFS
failure.

## 2026-07-21 — Reproducible Linux SDK package verification

Assumption: the Linux SDK archive is prerelease evidence only; reproducibility and consumer
linkage must be verified before any artifact can be considered for a signed release.

- `bash tools/verify-linux-sdk-package.sh` rebuilt the `0.1.0-alpha.2` archive twice from the
  current Core revision `4badabe735499a50265a1260a838df3254622c15` and reproduced SHA-256
  `9857c972ce16ae3d0243fecfe76755f301abe94ca3a3c10f880f62a2836914f`.
- The verifier accepted the outer archive and every packaged file, validated `linguamesh-core.pc`,
  and compiled and ran the packaged static-library C consumer smoke test.
- The generated archive remains local, unsigned prerelease evidence. No release-manifest artifact
  or stable-release claim is made.

## 2026-07-21 — Document decoder fuzz and AddressSanitizer smoke

Assumption: Milestone 8's parser-boundary requirement includes every supported document
extension and must enforce the existing 4 MiB document limit before parser work begins; this
checkpoint records fuzz-smoke evidence only and does not claim parser-completeness or a stable
release.

- Added a `document_decoders` libFuzzer target covering text, subtitle, CSV, HTML, JSON, DOCX,
  PPTX, XLSX, EPUB, and PDF dispatch through `DocumentJob::from_utf8`. Inputs are clamped to
  `MAX_DOCUMENT_BYTES`, and the target performs no provider, credential, or filesystem work.
- Added a locked fuzz-workspace dependency graph and a 106-file minimized corpus generated from
  the bounded local smoke. The fuzz workflow runs protocol and document targets under the pinned
  nightly `2026-07-20` AddressSanitizer toolchain for 2,000 iterations or 30 seconds each.
- Local document smoke passed with 2,000 runs (`cov: 869`, `ft: 1310`) and no crash. Core CI
  `29791113656`, Fuzz and sanitizers `29791113663`, and Native SDK `29791113659` passed for
  commit `e7ca21df183b15e10e157f175526a1b7ac0b3ad0`.

This closes the executable document-decoder fuzz-smoke sub-boundary. Broader FFI misuse
sanitizers, cross-client conformance, signed artifacts, and stable release remain open.

## 2026-07-21 — Protocol decoder fuzz and AddressSanitizer smoke

Assumption: Milestone 8 requires an executable coverage-guided decoder harness and sanitizer-backed
CI evidence, while production and release artifacts remain on stable Rust 1.93.0.

- Added a separate `fuzz/` Cargo workspace with a libFuzzer `protocol_decoders` target. It decodes
  the versioned Envelope and every command/event payload family, rejects inputs above the 1 MiB
  protocol bound, and performs no provider or credential work.
- Added `.github/workflows/fuzz.yml`, pinning fuzz-only nightly `2026-07-20`; cargo-fuzz's default
  AddressSanitizer instrumentation runs 2,000 iterations or 30 seconds on every Core push and pull
  request. The stable production workspace is not changed to nightly.
- Local smoke passed with 2,000 runs, increasing coverage features, a minimized corpus, and no
  crash. Remote Fuzz and sanitizers run `29789910142` passed on the fixed nightly toolchain;
  Core CI `29789910147` and Native SDK `29789910099` also passed all jobs for this revision.

This closes the executable protocol-decoder fuzz-smoke sub-boundary only. Broader FFI misuse
sanitizers, document-parser fuzzing, cross-client conformance, signed artifacts, and stable release
remain open.

## 2026-07-20 — C ABI malformed-input stress corpus

Assumption: the ABI submit boundary must remain panic-safe and bounded for arbitrary untrusted
bytes before native clients can rely on decoder hardening; this deterministic corpus complements,
but does not replace, sanitizer and coverage-guided fuzzing.

- Added a 4,096-case deterministic pseudo-random corpus with payload lengths capped at the existing
  1 MiB protocol limit. Every input crosses the real `lm_engine_submit` C boundary and must return
  a documented rejection or busy result; the engine is destroyed only after the complete corpus.
- The regression exercises malformed, incompatible, unsupported, and empty messages without a
  provider request, credentials, or unbounded allocation. It records the remaining sanitizer and
  coverage-guided fuzzing requirement explicitly instead of overstating this stress test.

The targeted FFI test passed locally. Full Core validation and remote CI must be recorded after the
next compatibility pin; no stable release or sanitizer/fuzz completion is claimed.

## 2026-07-20 — C ABI FileLease lifecycle projection

Assumption: native hosts need a bounded, engine-scoped lease control surface before document
commands can consume platform resources; resource values must remain private to Core.

- Added `file_lease_v1` C ABI create calls for validated paths, POSIX descriptors, Android parcel
  descriptors, and Windows handles. Core returns only an engine-scoped numeric token and stores
  the validated resource in a bounded registry of 64 leases per engine.
- Added active-state query, monotonic expire/revoke controls, and explicit destroy. Calls are
  panic-safe, reject wrong-engine or unknown tokens, remain cleanable after shutdown, and never
  return a path, descriptor, or handle across the ABI.
- Added C header declarations, a C++20 move-only RAII wrapper with engine-lifetime invalidation,
  and native C/C++ smoke coverage for ownership, expiry, revocation, engine isolation, and the
  registry bound. Document-command resource consumption and OS-handle transfer remain open.

Targeted FFI tests and native SDK smoke passed. Full workspace validation and remote CI evidence
must be recorded before this revision is considered compatible; no stable release or artifact
promotion is claimed.

## 2026-07-20 — Bounded FileLease lifecycle contract

Assumption: document hosts must be able to hand Core a bounded file resource without exposing
platform handle details or allowing a borrowed resource to survive expiry or explicit revocation.

- Added the public `FileLease` domain abstraction for desktop and temporary/output paths, POSIX and
  Android parcel descriptors, and duplicated Windows handles. Lease identifiers are opaque; paths,
  descriptors, and handles are validated before work, and `acquire` rechecks the active state on
  every resource access.
- Expiry and revocation are monotonic and race-safe. A held guard fails closed after either state
  transition, while the resource value remains private to the guard. This is a lifecycle contract,
  not an OS-handle duplication or close implementation; native platform ownership remains a client
  boundary until the ABI projection is specified.
- Domain tests cover every resource shape, invalid locations/descriptors/handles, and expired or
  revoked guard access. The `file_lease_v1` feature is now advertised by Core for client negotiation.

Local targeted domain/engine tests and formatting passed. Full workspace, FFI, and dependent Linux
validation will be recorded after this revision is pinned; no stable release or artifact promotion
is claimed.

## 2026-07-20 — C ABI compatibility snapshot projection

Assumption: native clients must negotiate the complete shared Core contract through one synchronous
ABI query before creating provider work; file-lease resource transport remains a later boundary.

- Added `CompatibilitySnapshot` to the versioned protocol and `lm_engine_get_compatibility` to the
  C ABI. The query returns an engine-owned `compatibility` Envelope containing Core semantic version,
  ABI major, protocol version, provider-catalog version, and enabled feature identifiers.
- The FFI regression decodes every field, validates the buffer ownership/release protocol, and
  confirms the `compatibility_negotiation_v1` feature is advertised. Protocol tests cover a complete
  snapshot round trip.
- Local Core formatting, locked workspace check, strict Clippy, all-target/all-feature tests, and
  native C/C++ SDK smoke are the required validation for this checkpoint. This remains a prerelease
  Core boundary; ABI file-lease projection, generated client projections, and release evidence
  remain open.

## 2026-07-20 — C ABI host-secret response projection

Assumption: native clients need the same one-time host-secret contract through ABI 1 that the Rust
application layer already enforces, while secret values remain runtime-only and absent from events.

- The versioned protocol now carries an optional non-secret `secret_ref` on `TranslateTextCommand`,
  `SecretRequiredEvent`, and `HostSecretResponse`. The C ABI emits `secret_required` before a
  credential-bearing operation, validates operation/correlation/request identity and bounded
  resolutions, delivers one `SecretValue` through the Core broker, and rejects replay, late,
  oversized, malformed, or mismatched responses.
- FFI event sequencing reserves sequence zero for the host request and starts the translated
  operation at sequence one. A cancellation during secret resolution produces exactly one typed
  cancelled terminal event; provider sessions clear the pending request map on completion.
- Local protocol/engine/FFI tests passed (`5`, `5`, and `12` tests). The FFI authenticated loopback
  regression proves `secret_required`, one-shot response handling, replay rejection, streamed
  `你好，LinguaMesh！`, and one completed terminal event without persisting the secret canary.
  Formatting and the FFI all-target check passed. This is a prerelease Core boundary; semantic
  compatibility/catalog negotiation, file leases, sanitizer/fuzz coverage, and client projections
  remain open.

## 2026-07-20 — Shared bounded retry policy contract

Assumption: all native clients should derive retry and circuit-breaker limits from one validated
Core policy so backoff, provider hints, jitter, and cooldowns cannot drift between platforms.

- Added the public `RetryPolicy` domain type with bounded construction, standard defaults, optional
  `Retry-After` handling, and typed validation errors. The policy caps backoff at 60 seconds,
  cooldown at five minutes, jitter at 100%, and consecutive failures at 32.
- Linux now consumes the Core policy for routing backoff and circuit-breaker thresholds while
  preserving cancellation and approved fallback behavior.

Local validation: Core formatting, workspace check, strict Clippy, and domain tests passed (34
tests). Linux formatting, GUI check, strict Clippy, no-default tests (80 passed, 1 ignored), and
demo-provider tests (144 passed, 3 ignored) passed. CI evidence will be recorded after the pinned
Core and Linux revisions complete their workflows; no stable release or artifact promotion is
claimed.

## 2026-07-20 — Short-text chunking regression fix

Assumption: the default approximate byte budget should only split text when the complete protected
source exceeds that budget; semantic whitespace boundaries remain available for oversized text.

- Core `ac1161cf7d90e5d44ec06cba9a4d667d44b0f9ac` keeps any protected source at or below the
  configured chunk limit as one chunk, preventing duplicate provider requests for short
  whitespace-containing input. A domain regression covers `Hello again`.
- Local `cargo fmt --all -- --check`, strict workspace Clippy, all-target/all-feature tests, and
  locked workspace build passed. Core Native SDK run `29764592256` passed Linux, Windows, Android,
  and Apple jobs. No stable release or artifact promotion is claimed.

## 2026-07-20 — Persisted document translation presets

Assumption: document jobs persist the same bounded, non-secret preset contract as text requests;
rows created before schema 18 continue with the `General` preset.

- Added transactional schema 18 migration `0018_document_translation_preset.sql`. Document-job
  options now validate and round-trip the complete `TranslationPreset` JSON, including across
  close/reopen and resume, while legacy NULL values default to `General`.
- Added a bounded 8 KiB serialized-preset limit and rejection tests for credential-shaped custom
  instructions. The storage table still contains no endpoint, credential, or source-content values.

Local validation: targeted storage tests and formatting passed. Full workspace evidence will be
recorded after the dependent Linux worker propagates the saved preset through restart and retry.

## 2026-07-20 — Request-level translation presets

Assumption: the built-in Linux presets (`General`, `Technical`, and `Marketing`) are the first
UI surface; document jobs now persist their selected preset through schema 18.

- Added the validated `TranslationPreset` contract with bounded domain, tone, formality, audience,
  regional-locale, script, custom-context, and custom-instruction fields. Built-ins have stable IDs,
  legacy requests deserialize to `General`, and credential-shaped or control text is rejected before
  provider work.
- Added `translation_presets_v1` to the Core compatibility feature set. The versioned provider
  prompt renders preset fields as escaped data, and translation-memory identity now includes the
  selected preset so cached output cannot cross preference boundaries.
- Updated OpenAI-compatible, Responses, Azure, native Ollama, Anthropic, and Gemini request paths
  to carry the same preset without hidden provider calls.

Local validation: `cargo fmt --all`, workspace offline check, strict Clippy, workspace all-target/all-
feature tests, and locked build passed. Linux and localization integration are recorded in their
dependent repositories after the next pinned revisions.

## 2026-07-20 — Persisted document quality modes

Assumption: document jobs use the same `Fast`, `Balanced`, and `Best` request policy as plain text;
legacy rows without a value are upgraded to `Balanced`.

- Added transactional schema 17 migration `0017_document_quality_mode.sql` and validated parsing
  of the stable `fast`/`balanced`/`best` names. Document-job options now round-trip the mode while
  retaining the existing non-secret storage boundary.
- Added schema-15 migration coverage and restart-safe storage tests; endpoint, credential, and source
  content fields remain absent from the options table.

Local validation: `cargo fmt --all`, workspace offline check, strict Clippy, workspace all-target/all-feature
tests (139 passed), locked build, and `git diff --check` passed. Linux integration remains on the next
pinned Core revision.

## 2026-07-20 — Translation quality modes and prompt contract

Assumption: `Fast`, `Balanced`, and `Best` remain single provider requests in this prerelease;
`Best` asks the model for an internal critique and revision, while Core performs deterministic
output validation and never adds hidden paid calls.

- Added `TranslationQualityMode` to `TranslationRequest`, defaulting to `Balanced`, with stable
  `fast`/`balanced`/`best` persistence names. Translation-memory identity now includes the actual
  mode instead of a placeholder value.
- Added the versioned `translation-prompt-v2` helper shared by OpenAI-compatible, Responses,
  Azure, native Ollama, Anthropic Messages, and Gemini adapters. Prompts delimit source content as
  untrusted data and include mode-specific trade-offs without exposing credentials.
- The engine validates non-empty UTF-8 output before emitting `Completed`; empty output and Unicode
  replacement characters become typed malformed-response failures after any received deltas.
- Added Core feature `translation_quality_modes_v1` and domain/provider/engine regression tests.

Local validation for this checkpoint: `cargo fmt --all`, workspace offline check, strict Clippy, and
workspace all-target/all-feature tests passed. Linux and localization integration remain on the
next pinned revisions; live provider account behavior and human prompt/copy review remain open.

## 2026-07-20 — OpenAI Responses streaming adapter

Assumption: Responses API model discovery remains compatible with `/v1/models`, while translation
requests use the typed SSE event contract documented by OpenAI; live quota, account, and model
availability remain external gates.

- Added the `openai_responses` adapter and `openai-responses` catalog preset. The adapter sends
  developer/user `input` items to `/v1/responses`, authenticates with Bearer credentials, and
  decodes `response.output_text.delta` plus `response.completed` events while ignoring metadata
  events and mapping provider failures to typed errors.
- Added a deterministic Responses fixture, fragmented-event decoder tests, application secret
  broker coverage, and the `openai_responses_v1` compatibility feature. Targeted Core tests pass;
  Linux integration and remote CI evidence are recorded in the dependent Linux checkpoint.

## 2026-07-20 — Azure OpenAI Chat Completions adapter

Assumption: Azure deployments are user-entered model identifiers and Azure model discovery is
manual for this prerelease; the deterministic fixture verifies the wire contract without live
credentials, quota, or account behavior.

- Added the `azure_openai_chat` adapter and `azure-openai` catalog preset. The shared OpenAI
  adapter now validates a resource root and deployment segment, sends `api-key` rather than
  Bearer authentication, appends `api-version=2024-10-21`, returns a manual deployment descriptor,
  preserves cancellation/protected-span behavior, and never puts credentials in URLs.
- Added deterministic testkit and application coverage for unauthorized/authorized requests,
  deployment paths, streamed `你好，Azure！` output, manual model selection, and secret-broker
  correlation. Core targeted tests and strict checks pass; Linux integration is pending its pinned
  Core/l10n revisions.

## 2026-07-20 — C ABI concurrent control-call checkpoint

Assumption: native clients may issue cancellation, shutdown, and bounded event-poll calls from
different host threads while keeping the opaque engine alive; the FFI boundary must serialize its
internal state and fail closed after shutdown without allowing a panic across C.

- Added `concurrent_control_calls_are_serialized_and_fail_closed_after_shutdown` to the FFI suite.
  Twelve host threads concurrently exercise cancel, shutdown, poll, and engine-scoped buffer
  release; the test then verifies that post-shutdown polling returns `LM_RESULT_SHUTDOWN` before
  one final destroy.
- `cargo fmt --all -- --check`, workspace strict Clippy, workspace all-target/all-feature tests,
  workspace locked build, and `cargo deny check advisories bans licenses sources` passed. The FFI
  suite now reports 10 passing tests, including the concurrent control-call regression.
- This is a test-only Core hardening descendant; the reviewed functional Linux pin remains
  `a87aaf2bef7cca287c4a6faa8addd340e0245b0e` and no stable release is claimed.

## 2026-07-20 — Anthropic Messages adapter checkpoint

Assumption: Anthropic's Messages API is configured with a user-supplied model identifier because
Anthropic does not expose a general-purpose model-list endpoint for this integration. The selected
model is validated before any host secret request, and remote HTTPS keeps the host's configured
proxy policy while loopback HTTP fixtures bypass ambient proxies.

Implemented `linguamesh-provider-anthropic` with `/v1/messages` streaming, the required
`anthropic-version` and `x-api-key` headers, bounded SSE decoding, cancellation, protected-span
restoration, typed HTTP errors, redacted diagnostics, and session credential clearing. The provider
catalog and `ProviderManager` now expose the `anthropic_messages` adapter and manual model listing.

Validated locally:

- `cargo fmt --all` — passed.
- `cargo check --workspace --all-targets --all-features --locked --offline` — passed.
- `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings` — passed.
- `cargo test --workspace --all-targets --all-features --locked --offline` — passed: 11 application,
  4 Anthropic-provider, and all existing workspace tests; 0 failed.
- `cargo deny check advisories bans licenses sources` — passed all four checks with existing
  non-blocking duplicate-version and unmatched-license warnings.

The deterministic fixture proves the Messages wire contract only; production credential approval,
human translated-copy review, native-client UI binding, and a stable release remain open.

## 2026-07-20 — Trusted Linux descriptor-backed storage open

Assumption: a Linux host that pins a private database inode with `openat2` must be able to hand
that already-open descriptor to Core without reopening a replaceable pathname; ordinary Core opens
must continue rejecting symbolic-link paths.

- `Storage::open_from_trusted_descriptor` accepts only an exact `/proc/self/fd/<fd>` path and
  opens it through a narrowly scoped trusted-host path. `Storage::open` retains
  `SQLITE_OPEN_NOFOLLOW` for every ordinary path.
- The Linux client now opens the private parent with `RESOLVE_NO_SYMLINKS`, opens the final file
  with `O_NOFOLLOW | O_CLOEXEC`, and keeps that descriptor alive through Core migration/open.
- Local format, strict Clippy, workspace tests, and the Linux pinned-parent replacement regression
  passed. The trusted descriptor API also has a Linux storage regression and rejects non-descriptor
  paths. This is a host-integration hardening checkpoint, not a stable-release claim.

## 2026-07-19 — Document routing-profile restart migration checkpoint

Assumption: a routed document job must retain only the non-secret routing-profile identifier so a
restart can re-run deterministic selection without persisting endpoints, credentials, or source
content; legacy schema-15 jobs continue to use their saved provider/model options.

Core schema 16 adds the transactional `0016_document_routing_profile.sql` migration and nullable
`document_job_options.routing_profile_id`. Storage validation bounds the identifier, schema-15
databases migrate in place, and the storage regression round-trips the new field without any
credential-shaped columns or values.

Validated locally:

- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` — passed.
- `cargo test --workspace --all-targets --all-features --offline` — passed: all workspace tests,
  including 31 storage tests; 0 failed.

Linux integration and remote SDK evidence are recorded in the client and central repositories.

## 2026-07-19 — Explainable routing planner contract checkpoint

Assumption: provider routing policy must be shared by all native clients and must not carry
endpoints, credentials, or source content. Platform UI integration remains a separate client slice.

Implemented `routing_planner_v1` in `linguamesh-domain`, and Core schema 15 now persists bounded
validated routing-profile JSON without endpoints, credentials, or source text. `RoutingProfile`
supports Manual, Ordered,
and Automatic selection with bounded, validated provider/model candidates; local, privacy,
capability, locale, request-size, quality, latency, and cost constraints; stable rejection reasons;
deterministic ranking; and explicit fallback ordering only when enabled by the profile. Core
compatibility now advertises the feature for clients that bind this contract.

Validated locally:

- `cargo fmt --all -- --check` — passed.
- `cargo test -p linguamesh-domain --offline routing` — passed: 5 routing tests, 0 failed,
  including malformed allow/deny lists, locale tags, and zero request limits.
- `cargo test -p linguamesh-storage --offline` — passed: 30 tests, 0 failed, including routing
  profile migration and round-trip coverage.

Full workspace validation and Linux compatibility evidence are pending for the published revision.

## 2026-07-19 — OOXML archive compression-ratio checkpoint

Assumption: bounded OOXML imports must reject suspicious compression ratios before any archive entry
is decompressed into a document manifest; the ratio guard complements the existing 4 MiB package,
512-entry, path, encryption, duplicate-name, and total-uncompressed-size limits.

Implemented a shared Core guard for DOCX, PPTX, and XLSX archive inspection and reconstruction. A
non-empty entry at or above 1 KiB is rejected when its uncompressed size exceeds 200 times its
compressed size, including zero-byte compressed descriptors. The rejection is typed as
`DocumentError::TooLarge` and occurs before XML inspection or output reconstruction.

Validated locally:

- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` — passed.
- `cargo test --workspace --all-targets --all-features --offline` — passed: all workspace tests,
  including 26 document tests and the suspicious-compression fixture; 0 failed.
- `cargo build --workspace --locked` — passed.

The fixture is deterministic and in-memory; full Linux integration and stable-release evidence
remain pending.

## 2026-07-19 — Native Ollama `/api` adapter checkpoint

Assumption: Linux-first local-model support needs both Ollama's native `/api` contract and the
OpenAI-compatible `/v1/` contract, while interoperability with an independently running daemon
remains outside the deterministic fixture boundary.

Implemented `linguamesh-provider-ollama` with loopback/HTTPS endpoint validation, optional one-shot
credential handling, `/api/tags` model discovery, `/api/chat` NDJSON streaming, fragmented UTF-8
decoding, cancellation, bounded responses, protected-span restoration, and exactly-one completion
validation. The provider catalog now includes the loopback-only `ollama` preset. Core application
tests and the Linux worker exercise explicit `ollama_chat` profiles without a secret and stream
`你好，Ollama！` from the native fixture.

Validated locally:

- `cargo fmt --all --check` — passed.
- `cargo check --workspace --all-targets --all-features --locked` — passed.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` — passed.
- `cargo test --workspace --all-targets --all-features --offline` — passed: all workspace tests,
  including 10 application, 4 Ollama-provider, and 6 testkit tests; 0 failed.
- Linux `cargo test --features demo-provider --lib --offline` — passed: 105 tests, 2 ignored,
  including the native Ollama worker flow.

The fixture proves the native wire contract and does not claim a running third-party Ollama daemon,
GPU acceleration, or a stable release.

## 2026-07-19 — Ollama-compatible loopback contract checkpoint

Assumption: the required local-model acceptance path may use Ollama's OpenAI-compatible `/v1/`
surface. Native Ollama `/api` protocol support and interoperability with a running third-party
daemon remain separate work.

Implemented a deterministic testkit fixture that returns an Ollama-style `llama3.2:latest` model
from `/v1/models` and streams the same OpenAI Chat Completions shape from `/v1/chat/completions`
without a credential. The fixture is consumed by the Linux worker's real connect, deliberate model
selection, and streaming translation test; it is explicitly not a claim about the native `/api`
surface.

Validated locally:

- `cargo fmt --all --check` — passed.
- `cargo test -p linguamesh-testkit --locked` — passed: 5 tests, 0 failed.
- Linux `cargo test --features demo-provider --locked worker::tests::loopback_ollama_compatible_provider_translates_without_secret -- --exact` — passed.

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

## 2026-07-18 — Linux document job persistence checkpoint

Assumption: the Linux-first document queue persists only an opaque job ID, source basename, format,
ordered bounded segments, and lifecycle state. It never stores the source filesystem path, provider
credentials, or session secrets. Pending and running jobs are the only states restored automatically;
GUI queue presentation and archive codecs remain later work.

Implemented Core schema 6 with `document_jobs` and `document_segments` tables, transactional snapshot
replacement, bounded job/segment/text limits, exact segment updates, resumable-job listing, state
transitions, deletion, and cascade cleanup. `DocumentJobState` covers pending, running, completed,
cancelled, and failed snapshots. Linux `CoreWorker` now exposes create/list/update/resume/cancel
commands, emits startup restoration snapshots, and persists segment progress across worker restart.
Core Storage tests (19 passed) and Linux worker tests (88 passed, 1 intentional environment skip)
cover migration, restart recovery, output reconstruction, queue bounds, and credential/path absence.

## 2026-07-18 — Linux document pause checkpoint

Assumption: pause is a segment-boundary operation. The active provider operation is cancelled,
only completed segments are persisted, and the job becomes `paused`; resume continues pending
segments with explicit provider options, while retry also accepts cancelled or failed snapshots.

Implemented Core schema 7 and the `paused` state, including a transactional table rebuild and
restart recovery. Linux `CoreWorker` now exposes pause, resume, and retry commands, and the GTK
surface shows per-job progress plus pause/resume/retry controls. Android, Windows, and macOS
remain intentionally out of scope for this Linux-first slice; archive codecs, automatic provider
parameter persistence, and a multi-job queue remain open.

## 2026-07-18 — Linux document restart options checkpoint

Assumption: only non-secret translation parameters are reusable after restart. The active provider
profile and selected model must match the saved identifiers; endpoints, credentials, session secrets,
and privacy-mode state remain runtime-only.

Implemented Core schema 8 with bounded, validated `document_job_options` persistence for source and
target locales, model/provider identifiers, and optional glossary JSON. Linux Translate saves these
options before entering the running state; Resume and Retry load them from storage, require the
matching active provider/model, and resume with standard privacy. A worker restart test pauses a
slow job, reconnects to a fresh worker/provider, and completes the saved job without UI parameters.
Android, Windows, and macOS remain intentionally out of scope; archive codecs, automatic provider
discovery, and multi-job queue selection remain open.

## 2026-07-18 — Linux subtitle document checkpoint

Assumption: SRT and WebVTT translation keeps cue IDs, headers, timestamps, ordering, and original
line endings verbatim. Only cue text becomes translatable; no automatic timing or line-length rewrite
is performed.

Implemented `linguamesh-document` support for bounded UTF-8 `.srt` and `.vtt` jobs. The codec validates
SubRip/WebVTT headers, cue ordering, timestamp syntax, and required cue text before creating segments;
reconstruction validates the subtitle structure again. Linux's native file chooser accepts both suffixes
and maps malformed subtitle structure to a safe import error. TXT/Markdown behavior remains unchanged;
HTML/JSON/CSV and archive formats remain future slices.

Validated locally:

- Core document tests: 7 passed, including cue-ID/timestamp preservation and malformed-structure rejection.
- Linux `cargo check --all-targets --all-features --offline`, strict Clippy, and 95 library tests passed;
  one existing environment-dependent test remains intentionally ignored.

## 2026-07-18 — TXT/Markdown document contract

Assumption: the first Linux-first document slice treats TXT and Markdown as bounded UTF-8 line
documents; Markdown fenced code and blank structure remain verbatim, while prose lines are
translated independently and reconstructed only after every prose segment is complete.

Implemented `linguamesh-document` with extension validation, 4 MiB input/output bounds, BOM removal,
line-ending preservation, Markdown fence classification, serializable segment/job state, pending
segment counting, exact segment updates, and fail-closed reconstruction. Five focused unit tests
cover format detection, BOM/line endings, verbatim fences, incomplete reconstruction, and size/UTF-8
rejection. Full workspace fmt, strict Clippy, locked build, offline workspace tests, and diff checks
passed locally. Native document queues, persistent interrupted-job recovery, archive codecs, and
stable release evidence remain open.

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
- Generated Swift and C++ Protobuf types, Android/Apple symbol bundles, SBOMs, immutable release
  checksums, and cross-platform conformance remain incomplete; the bounded ABI file-lease and
  sanitizer/fuzz surfaces are covered by the current status entries above.
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
  persistence. On-disk connections use WAL, `synchronous=FULL`, secure deletion, and a truncating
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
- The C ABI now projects the one-time host-secret response flow and the five-dimension compatibility
  snapshot. File leases and other complete Milestone 2 host services remain unimplemented.

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
