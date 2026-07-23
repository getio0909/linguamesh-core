# Privacy

LinguaMesh Core has no telemetry, account, or default network destination. The engine sends
translation content only to the provider endpoint selected by the host application. The local
fake provider and loopback fixtures keep content on the machine.

Provider credentials and client-certificate identities enter Core only through bounded, one-shot
host-secret responses or reference-bearing profiles. Core stores SecretRefs and non-secret profile
metadata, never secret values, authorization headers, private keys, source text, or translated
output in diagnostics or logs. Persistent history and translation memory are explicit policies;
Incognito requests bypass both.

Provider terms govern content deliberately sent to a remote endpoint. Hosts must redact logs,
protect storage paths, and provide the platform's secure secret broker. Report privacy defects
through `SECURITY.md` without including credentials or private documents.
