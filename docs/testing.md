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
