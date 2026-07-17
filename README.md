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
