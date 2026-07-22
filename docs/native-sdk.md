# Native SDK Contract

The prerelease native boundary is defined by `contracts/abi/linguamesh.h` and
`contracts/proto/linguamesh.proto`. ABI major `1` and protocol version `1` are development
contracts: clients must query both before creating an engine and must reject unknown values.
ABI major `1` replaces the published ABI `0` prerelease source skeleton. ABI 0 had no binary SDK or
compatible client release; see [`ABI 0 to ABI 1 Migration`](migrations/abi-0-to-1.md).

Before creating an engine, call `lm_engine_get_compatibility` with a zeroed `LmBuffer`. Decode the
returned `compatibility` Envelope as `CompatibilitySnapshot` and validate `core_version`,
`abi_major`, `protocol_version`, `provider_catalog_version`, and every required
`enabled_features` entry. Release the returned buffer with `lm_engine_buffer_free` before using the
engine. Unknown versions, catalog values, or feature requirements must fail closed.

## Command and event flow

An engine accepts one active operation. Submit an `Envelope` with non-empty operation and
correlation IDs, sequence `0`, message type `translate_text`, and an encoded
`TranslateTextCommand` payload. The command contains an endpoint, model ID, untrusted source text,
target locale, and optional bounded non-secret `organization`, `project`, and `custom_headers_json`
metadata. Credential-shaped organization/project values and unsafe custom headers are rejected
before any host-secret request; credentials themselves are deliberately absent.

Poll from a dedicated non-UI execution context. A successful poll returns an owned buffer; an empty
buffer means the bounded timeout elapsed. Every non-empty buffer contains an `Envelope` with the
same IDs and a monotonic sequence. Supported event types are:

| Message type | Payload | Meaning |
| --- | --- | --- |
| `started` | empty | Operation accepted by the engine |
| `text_delta` | `TextDeltaEvent` | New incremental output |
| `completed` | empty | Successful terminal event |
| `cancelled` | empty | Cancelled terminal event |
| `failed` | `FailureEvent` | Typed safe failure terminal event |

Initialize every `LmBuffer` output descriptor to zero before polling. The caller must release every
returned non-empty allocation with `lm_engine_buffer_free` and the same engine that produced it;
releasing the same cleared descriptor again is safe. Release remains available after engine
shutdown, but every buffer must be released before engine destruction. Each engine owns its active
allocation registry and nonzero ownership-token sequence. The ABI verifies the token, pointer,
length, capacity, and engine ownership before freeing. Wrong-engine, forged, copied-after-free,
address-reused, or client-owned descriptors are rejected without dereferencing their data pointers.
No process-global buffer state is used, and buffer bytes are neither persisted nor logged. Kotlin,
C++, and Swift wrappers copy event bytes and release the native allocation while the engine call is
still active. Kotlin and Swift wrappers coordinate close with in-flight calls; a C++ host must stop
and join its polling worker before destroying the owning RAII object. Call `cancel` from another
worker context; never block a platform UI thread in `poll_event`.

Each engine permits at most `LM_MAX_OUTSTANDING_BUFFERS` (currently 64) returned buffers. Polling
reserves a slot before reading the event queue. If every slot is held, polling returns
`LM_RESULT_RESOURCE_EXHAUSTED`, leaves the output descriptor zeroed, and does not consume an event.
Release at least one buffer through its owning engine before polling again.

## Current boundaries

The command configures a generic OpenAI-compatible endpoint and may carry a non-secret `secret_ref`.
When a reference is present, the engine emits one `secret_required` event, pauses the operation,
and accepts exactly one matching `host_secret_response` envelope. The response resolution is
`provided`, `unavailable`, or `secure_storage_unavailable`; a provided value is held only in the
bounded in-memory provider session and is never persisted, logged, or placed in an event. Secret
responses are checked against the active operation and correlation IDs, reject oversized values,
and reject replay or late responses. Only loopback HTTP or remote HTTPS should be used. A second
submit while an operation is active returns `LM_RESULT_BUSY`.

The direct Rust application layer now owns the bounded `FileLease` lifecycle for path, descriptor,
temporary, and output resources. A lease has an opaque ID and can be acquired only while active;
expiry or explicit revocation makes subsequent guard access fail closed. The C ABI advertises
`file_lease_v1` in its compatibility snapshot and exposes engine-scoped creation, active-state,
expiry, revocation, and destroy calls. Creation accepts bounded UTF-8 paths or validated numeric
platform descriptors/handles and returns only a numeric lease token; resource values never cross
the ABI. Each engine permits at most 64 leases, tokens are not portable between engines, and
cleanup remains valid after shutdown. `lm_engine_file_lease_consume_document` accepts a bounded
UTF-8 source name and document snapshot, parses it with the shared document contract, and consumes
the lease exactly once on success; malformed input or an expired lease is rejected without
consumption. On Unix, `lm_engine_file_lease_consume_posix_document` duplicates the registered
POSIX descriptor, reads at most `MAX_DOCUMENT_BYTES + 1`, and applies the same one-shot parse and
lease cleanup rules; the caller must position the descriptor at the beginning of the document.
The original descriptor remains owned by the host. Engine handles still depend on the
documented single-destroy and close-after-workers-stop contract; stale, forged, or concurrently
destroyed handles are not protected by a handle registry in ABI major `1`.

## Loopback conformance provider

Run `cargo run --locked -p linguamesh-cli -- fake-provider --port 40123` to keep the deterministic
OpenAI-compatible provider active until Ctrl+C. It binds only `127.0.0.1`, prints its exact `/v1/`
endpoint in English, and needs no credential. Desktop clients use the printed endpoint directly.
For an Android emulator, first run `adb reverse tcp:40123 tcp:40123`, then use
`http://127.0.0.1:40123/v1/` inside the app. Port zero requests an available system-selected port.

## Platform sources

- Android AAR/JNI source: `bindings/android/`
- Generic C++20 RAII wrapper usable by a C++/WinRT host: `bindings/cpp/`
- macOS XCFramework/Swift Package source: `bindings/apple/`
- Linux direct Rust guidance: `bindings/linux/`

Run `bash tools/test-native-sdk.sh` for Linux C/C++ smoke tests and
`bash tools/verify-linux-sdk-package.sh` for deterministic archive verification. Android, Windows,
and Apple jobs live in `.github/workflows/native-sdk.yml`. Every third-party action is pinned to an
immutable commit. Pull requests, manual runs, and pushes to `main` upload the Linux SDK archive,
Android AAR, Apple XCFramework ZIP, their source-revision metadata, and SHA-256 manifests. A
workflow definition is not successful artifact evidence until that platform job actually passes.
