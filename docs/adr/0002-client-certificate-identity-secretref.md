# ADR 0002: Client-certificate identity through SecretRef

Status: accepted for the Linux-first prerelease slice.

Assumption: some enterprise provider endpoints require mutual TLS, but client certificates and
private keys are credentials and must not be persisted in `ProviderProfile`.

`ProviderProfile.client_certificate_identity_ref` stores only a persistent or session `SecretRef`.
The Linux host accepts one combined PEM certificate/private-key value, clears the input widget
immediately, and resolves it once through the host secret broker. Core bounds the value, requires
both certificate and private-key PEM sections, and keeps the parsed identity in memory only.

All built-in reqwest adapters use the rustls `Identity` builder while preserving system roots,
hostname verification, redirect blocking, and normal TLS verification. Invalid PEM is rejected as
typed configuration failure before provider discovery. SQLite schema 31 and storage tests reject
session references at the persistence boundary.

Cross-client bindings, hardware-backed keys, PKCS#12 import, live enterprise endpoint evidence,
and stable-release qualification remain open.
