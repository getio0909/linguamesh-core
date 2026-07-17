# Architecture

The workspace separates stable domain types, protocol envelopes, provider contracts, the OpenAI-compatible adapter, persistence, orchestration, test infrastructure, C ABI, and CLI. Dependencies point inward toward domain and provider abstractions. Native clients consume the engine through generated wrappers over the stable C ABI, except Linux where direct Rust integration is preferred.
