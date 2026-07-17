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

For native client conformance, keep the same deterministic provider running on a chosen loopback
port:

```sh
cargo run --locked -p linguamesh-cli -- fake-provider --port 40123
```

The command prints the exact endpoint and stays active until Ctrl+C. Desktop clients can use that
endpoint directly. For an Android emulator, run `adb reverse tcp:40123 tcp:40123` and configure
the embedded core with `http://127.0.0.1:40123/v1/`; this preserves the loopback-only HTTP policy.

## Native SDK foundations

The prerelease ABI now executes `translate_text` Protobuf commands through the real engine and
returns ordered event envelopes. See [the native SDK contract](docs/native-sdk.md) for ownership,
threading, compatibility, wrapper locations, build commands, and explicit host-service gaps.

On Linux, validate the C header and C++20 RAII wrapper with:

```sh
bash tools/test-native-sdk.sh
```
