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

Assumption: this partial checkpoint establishes tested wrapper source, package layouts, and
deterministic build definitions. It does not complete Milestone 2 without successful Android,
Windows, and Apple artifact jobs, complete host services, cross-platform conformance evidence, and
release-manifest records.

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

Not verified or not implemented:

- Android platform 36 is installed locally and a Java 21 runtime is available, but `ANDROID_HOME`
  is unset and the required NDK, Gradle executable, and `javac` are unavailable. Android unit tests,
  lint, native cross-compilation, and AAR assembly therefore were not run locally.
- Windows DLL/import-library, x64 and ARM64 coverage, NuGet packaging, C++/WinRT application
  integration, a generated C++/WinRT projection, and debug symbols were not built because
  Windows/MSVC is unavailable locally.
- The XCFramework and Swift Package were not built because Swift and Xcode are unavailable locally;
  Apple architecture slices and publishable symbols therefore remain unverified.
- Typed host secret brokerage, file leases, semantic-version/catalog/feature negotiation, generated
  Swift and C++ Protobuf types, sanitizer/fuzz coverage, Android/Apple symbol bundles, SBOMs,
  immutable release checksums, and cross-platform conformance remain incomplete.
- Engine-handle forgery, stale-handle use, repeated destruction, and destruction racing unjoined
  raw callers remain outside the ABI-major-1 contract and lack sanitizer-backed misuse tests.
- `shellcheck`, `actionlint`, and `pwsh` are unavailable locally; their platform-specific checks were
  not run. Bash parsing and YAML parsing passed, but only GitHub Actions runs can validate runner
  behavior.
- No package was published and no central release or compatibility manifest is claimed by this
  uncommitted checkpoint.
