# Architecture

The workspace separates stable domain types, protocol envelopes, provider contracts, the
OpenAI-compatible adapter, persistence, orchestration, test infrastructure, C ABI, and CLI.
Dependencies point inward toward domain and provider abstractions. Native clients consume the
engine through platform wrappers over the stable C ABI, except Linux where direct Rust integration
is preferred. The current prerelease wrappers are maintained source; reproducible wrapper
generation remains future work.

The ABI owns a Tokio runtime, a bounded event channel, and at most one active operation per opaque
engine handle. A submitted `translate_text` envelope is decoded into the same `TranslationEngine`
used by Rust callers. A forwarding task encodes ordered domain events back into Protobuf envelopes;
native polling never invokes arbitrary callbacks. See [native-sdk.md](native-sdk.md) for the wire
contract and current host-service limitations.
