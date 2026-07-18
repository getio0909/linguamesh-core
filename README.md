# LinguaMesh Core

Shared Rust engine and stable native boundary for LinguaMesh.

## Prerequisites

- Rust 1.93.0 (installed automatically through `rust-toolchain.toml`)
- A C compiler for bundled SQLite

## Validate

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --workspace --locked
```

## Local streaming demo

```sh
cargo run -p linguamesh-cli -- demo --text "Hello, LinguaMesh" --target zh-CN
```

The demo starts a loopback fake provider and requires no API key.

The shared OpenAI-compatible path protects common URLs, email addresses, Markdown code spans,
fenced code, and placeholders while translating. Marker restoration is incremental and fails closed
if a provider omits, duplicates, or changes a protected span. User glossaries and custom protected
terms are not yet part of this prerelease slice.

For native client conformance, keep the same deterministic provider running on a chosen loopback
port:

```sh
cargo run --locked -p linguamesh-cli -- fake-provider --port 40123
```

The command prints the exact endpoint and stays active until Ctrl+C. Desktop clients can use that
endpoint directly. For an Android emulator, run `adb reverse tcp:40123 tcp:40123` and configure
the embedded core with `http://127.0.0.1:40123/v1/`; this preserves the loopback-only HTTP policy.

## Secure provider foundation

The `linguamesh-application` crate exposes a bounded, cancellable host-secret request channel and
connects canonical non-secret provider profiles to the shared engine. Native hosts persist the
credential itself in platform secure storage and return a one-time `SecretValue`; Core SQLite stores
only its `SecretRef`. `linguamesh-engine::core_compatibility` reports the semantic, ABI, protocol,
provider-catalog, and enabled-feature snapshot. A native client must validate every version
dimension and its required feature subset before starting provider work.

On Linux's default Unix VFS, on-disk SQLite opens use SQLite's no-follow flag, so any symbolic-link
component in the database path is rejected before migrations or journal configuration. Other VFS
implementations require platform-specific verification. Native hosts remain responsible for
private parent directories, regular-file checks, and platform-specific file permissions.

The Linux client consumes this typed Rust path directly. The C ABI projection of the secret broker
remains future work and must not be inferred from the Rust API. Rust consumers moving from alpha 1
must follow the
[alpha-2 source migration](docs/migrations/core-alpha-1-to-alpha-2.md).

## Native SDK foundations

The prerelease ABI now executes `translate_text` Protobuf commands through the real engine and
returns ordered event envelopes. See [the native SDK contract](docs/native-sdk.md) for ownership,
threading, compatibility, wrapper locations, build commands, and explicit host-service gaps.

On Linux, validate the C header and C++20 RAII wrapper with:

```sh
bash tools/test-native-sdk.sh
```
