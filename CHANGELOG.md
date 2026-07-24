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
- Added a bounded FFI lifecycle-sequence fuzz target covering synchronized control calls, forged
  buffer descriptors, and file-lease token handling without destroying an active engine mid-sequence.
- Added a bounded valid-command FFI fuzz target using the loopback fake provider to exercise
  submit, streamed events, buffer ownership, terminal completion, and engine destruction without
  commercial credentials.

No stable Core release has been published.
