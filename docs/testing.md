# Testing

Default tests use only local deterministic fixtures and the loopback fake provider. They must not require commercial credentials or the public internet. Provider contract tests cover fragmented SSE, split UTF-8, malformed messages, disconnects, errors, response limits, and cancellation. Persistence tests use isolated temporary or in-memory SQLite databases.

Native ABI tests submit a real Protobuf translation command to the loopback fake provider, assert
ordered deltas and exactly one terminal event, and verify cancellation. Run Linux C and C++ consumer
smoke tests with `bash tools/test-native-sdk.sh`. The FFI suite also verifies bounded concurrent
polling, isolates allocation ownership between engines, rejects forged or duplicate buffer
descriptors without freeing client memory, permits release after engine shutdown, and proves that
the 65th outstanding-buffer reservation fails without growing the registry beyond 64. Run
deterministic Linux packaging twice, then verify its outer and per-file manifests, with
`bash tools/verify-linux-sdk-package.sh`. That verification also validates the pkg-config metadata
and links the packaged static library into the C consumer smoke test.

Run `bash tools/test-native-sdk-fake-provider.sh` to verify that the standalone loopback provider
reports a usable endpoint, serves the deterministic model catalog, and shuts down cleanly.

Android AAR, Windows DLL/import-library, and macOS XCFramework builds require their platform jobs.
Do not treat YAML parsing, source review, or a Linux-only build as evidence that those artifacts
compile.

The Linux secure-provider prerequisite is covered by normal workspace tests. They verify every
compatibility dimension, canonical profile validation and redacted `Debug`, schema-1-to-2 migration,
on-disk reopen, active-profile and per-profile model persistence, cascade/disable behavior, and the
absence of a synthetic credential-shaped canary from every database-directory artifact after
authenticated use. A schema-1 fixture also starts with a credential value and proves the
secure-delete migration
plus truncating checkpoint removes it from all database artifacts, including after a reader makes
the first checkpoint busy and the next on-disk open retries it. The application tests run an
authenticated loopback provider and prove correlated one-time
secret delivery, strict queue capacity, typed host failure, cancellation of pending secret/model
discovery work, rejection of late secrets, in-flight cancellation and credential clearing on
provider disconnect, and adapter rejection before any secret request. Domain tests reject unsafe
endpoints before a profile can reach the application layer.

These tests exercise an in-process fake host. Linux Secret Service behavior, session-only fallback,
and native restart restoration remain client-repository gates. The C ABI host-response projection
also remains unimplemented and is not covered by the typed Rust broker tests.
