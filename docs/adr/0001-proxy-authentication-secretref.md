# ADR 0001: Proxy authentication uses host-secret references

Status: Accepted for the prerelease Core contract.

Assumption: provider proxy authentication is needed for Linux-first deployments, but proxy URLs
must remain safe to display, persist, export, and diagnose.

## Decision

`ProviderProfile.proxy_auth_ref` stores only a `SecretRef`. The Linux host keeps the entered
`username:password` in memory for session-only connections or stores it in Secret Service when the
user chooses Remember. Core resolves the reference through the existing one-time host secret
broker, parses a bounded pair, and passes it to each HTTP adapter as proxy Basic authentication.

## Security boundary

Proxy URL userinfo is rejected. The parsed username and password are never included in profile
debug output, SQLite columns, manifests, URLs, logs, or provider request bodies. Storage accepts
only persistent references, while the host may provide a session reference without persisting it.
Malformed, oversized, or control-containing values fail closed before network setup.

## Consequences

All built-in adapters share one semantic contract, while native clients remain responsible for
their secure-storage UX. Cross-client bindings and proxy-specific provider behavior remain open
until each client implements the same reference-only boundary.
