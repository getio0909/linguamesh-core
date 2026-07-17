# Apple XCFramework and Swift Package

`Package.swift` exposes the public `LinguaMeshCore` Swift lifecycle wrapper over a local binary
target named `CLinguaMeshCore`. The wrapper owns the opaque engine and Rust buffers, validates ABI
and protocol versions, prevents `close()` from racing active calls, and keeps raw C calls in one
module.

On a macOS host with Xcode command-line tools and Rust 1.93.0:

```sh
bash tools/build-apple-sdk.sh
swift test --package-path bindings/apple
```

The script builds Apple Silicon and Intel macOS static libraries, creates
`bindings/apple/Artifacts/LinguaMeshCore.xcframework`, produces a normalized ZIP, and writes its
SwiftPM checksum, source-revision metadata, and `SHA256SUMS` covering both the ZIP and metadata. The
native SDK workflow uploads the complete set on pull requests, manual runs, and pushes to `main`.
Generated artifacts are intentionally ignored until a verified prerelease is authorized. A remote
binary-target release must replace the local `path` with an immutable URL and the recorded checksum.

This Linux checkpoint does not claim that the XCFramework or Swift package builds. Typed secret
and file-lease host flows also remain unavailable even though raw host-response envelopes can pass
through the wrapper.
