# Core Alpha 1 to Alpha 2

Change identifier: `LM-CHANGE-2026-07-LINUX-SECURE-PROVIDER-1`

The Rust workspace version advances to `0.1.0-alpha.2` because this Linux-first prerequisite adds
source-breaking prerelease APIs. Linux consumers must update every exact Core dependency pin
together and validate `core_compatibility()` before provider work. Android, Windows, and Apple
client feature/API work remains frozen in this change and must not claim the typed Rust capability.
Existing CI jobs may rebuild those unchanged wrappers. Their package coordinates remain
platform-owned prerelease versions, while the generated metadata, where it exists, records the
embedded Core Rust workspace version separately.

## Source migration

- Replace cloned `ApiCredential` values with the non-cloneable
  `linguamesh_domain::SecretValue`. The deprecated `ApiCredential` type alias keeps constructor
  spelling available temporarily, but it no longer provides `Clone`.
- Construct credential-bearing adapter configuration with
  `OpenAiConfig::with_credential(endpoint, secret)`. Use `without_credential` for loopback and
  credential-free providers. `OpenAiConfig` is intentionally non-cloneable.
- Handle the new `ErrorKind` variants: `InvalidConfiguration`, `UnsupportedCapability`,
  `SecretUnavailable`, and `SecureStorageUnavailable`.
- Use `ProviderManager` and `host_secret_channel` from `linguamesh-application`. One manager owns
  at most one active credential-bearing engine and preserves the old engine when a candidate
  connection fails or is cancelled.
- Persist only validated `ProviderProfile` values. Endpoints reject embedded user information,
  queries, fragments, percent-encoded or credential-shaped paths, remote HTTP, and unsupported
  schemes. `SecretRef` uses a closed namespace and a canonical random UUID; `session:<uuid>`
  references must never be written to SQLite.

SQLite schema migration is documented in [`Storage Schema 1 to 2`](storage-1-to-2.md). ABI major 1
and protocol version 1 do not change. The new secret broker and compatibility snapshot are typed
Rust capabilities and remain unavailable through the C ABI.
