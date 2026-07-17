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
buffer-release functions; tests cover invalid inputs, protocol versions, ownership, and repeated
shutdown. `contracts/abi/linguamesh.h` records the current native boundary.

This checkpoint does not claim complete Milestone 1 provider configuration, complete C ABI
behavior, native SDK artifacts, document support, or a stable release.
