# Android AAR Source Package

This directory builds `org.linguamesh:linguamesh-core-android:0.1.0-alpha.1`. The `core` module
contains the public `org.linguamesh.core.LinguaMeshEngine` lifecycle wrapper, configures Protobuf
generation under `org.linguamesh.core.protocol`, isolates JNI in one bridge, and stages selected
Rust libraries in `src/main/jniLibs` during a build.

Required tools are JDK 17, Android SDK 36 with Build Tools 36.0.0, NDK `28.2.13676358`, CMake
3.22.1, Rust 1.93.0, and Gradle 9.5.0. The build uses AGP 9.3's built-in Kotlin support and the
AGP-9-compatible Protobuf Gradle plugin 0.10.0. From the core repository root:

```sh
ANDROID_NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358" \
  bash tools/build-android-sdk.sh
```

The script cross-compiles `linguamesh-ffi` for `arm64-v8a`, `armeabi-v7a`, and `x86_64`, runs the
Kotlin unit tests and release lint, then builds the release AAR. Applications can call
`translateText(...)`, optionally supplying non-secret organization/project identifiers and
validated custom-header JSON, and consume `pollDecodedEvent(...)` as typed `CoreEvent` values
without importing generated Protobuf classes.
Raw envelope methods remain available for protocol-level integration and diagnostics. The package
does not contain credentials. Apps keep platform secrets in the Android Keystore-backed broker and
send only versioned host responses to the wrapper.

The release AAR is written to `core/build/outputs/aar/core-release.aar`. `build-metadata.json`
records the source revision, ABI, protocol, selected ABIs, embedded Core Rust workspace version in
the `package_version` field, and prerelease status; `SHA256SUMS` covers both files. The native SDK
workflow uploads the complete set on pull requests, manual runs, and pushes to `main`.

Typed secret request decoding and one-shot host-response encoding are now exposed through
`CoreEvent.SecretRequired` and `sendHostResponse(...)`; the wrapper keeps the response envelope
and Protobuf details private while the app remains responsible for resolving platform secrets.
Typed file-lease request/response messages and background document support remain outside this
prerelease source package.
