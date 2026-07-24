# Changelog

## Unreleased

- Added bounded provider profile transport settings, SecretRef-backed proxy authentication, and
  client-certificate TLS identity handling across the built-in adapters.
- Added ABI 1 host-secret response validation, bounded document/file-lease contracts, provider
  routing, document codecs, migration coverage, and fake-provider integration tests.
- Added Linux SDK packaging, fuzz/sanitizer smoke checks, checksums, SBOM evidence, and strict
  compatibility validation.
- Added a bounded FFI input fuzz target with AddressSanitizer coverage for malformed and unsupported
  protocol envelopes; valid provider commands remain outside this network-free smoke.

No stable Core release has been published.
