#!/usr/bin/env bash
set -euo pipefail

# 从仓库根目录解析所有输入和输出。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    printf '%s\n' 'ANDROID_NDK_HOME is required.' >&2
    exit 1
fi
if [[ ! -d "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" ]]; then
    printf '%s\n' 'Android NDK LLVM toolchain was not found.' >&2
    exit 1
fi
ndk_revision="$(sed -n 's/^Pkg.Revision[[:space:]]*=[[:space:]]*//p' "$ANDROID_NDK_HOME/source.properties" | head -n 1)"
if [[ "$ndk_revision" != "28.2.13676358" ]]; then
    printf 'Android NDK 28.2.13676358 is required; found %s.\n' "${ndk_revision:-unknown}" >&2
    exit 1
fi

# 宿主标签来自 NDK 已安装的唯一预编译工具链目录。
toolchain_root="$(find "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt" -mindepth 1 -maxdepth 1 -type d -print -quit)"
if [[ -z "$toolchain_root" ]]; then
    printf '%s\n' 'Android NDK host toolchain was not found.' >&2
    exit 1
fi

# 仅清理由本脚本管理且已忽略的 JNI 库目录，避免打包陈旧架构。
jni_library_root="$repo_root/bindings/android/core/src/main/jniLibs"
rm -rf -- "$jni_library_root"
mkdir -p "$jni_library_root"

build_android_target() {
    local rust_target="$1"
    local android_abi="$2"
    local linker_name="$3"
    local linker_variable="$4"
    local destination="$jni_library_root/$android_abi"
    local target_key="${rust_target//-/_}"
    rustup target add "$rust_target"
    env \
        "PATH=$toolchain_root/bin:$PATH" \
        "$linker_variable=$toolchain_root/bin/$linker_name" \
        "CC_$target_key=$toolchain_root/bin/$linker_name" \
        "AR_$target_key=$toolchain_root/bin/llvm-ar" \
        cargo build --locked --release -p linguamesh-ffi --target "$rust_target"
    mkdir -p "$destination"
    install -m 0644 \
        "target/$rust_target/release/liblinguamesh_ffi.so" \
        "$destination/liblinguamesh_ffi.so"
}

build_android_target \
    aarch64-linux-android \
    arm64-v8a \
    aarch64-linux-android26-clang \
    CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER
build_android_target \
    armv7-linux-androideabi \
    armeabi-v7a \
    armv7a-linux-androideabi26-clang \
    CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER
build_android_target \
    x86_64-linux-android \
    x86_64 \
    x86_64-linux-android26-clang \
    CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER

if ! command -v gradle >/dev/null; then
    printf '%s\n' 'Gradle 9.5.0 is required to assemble the AAR.' >&2
    exit 1
fi
if ! command -v sha256sum >/dev/null; then
    printf '%s\n' 'sha256sum is required to record the AAR checksum.' >&2
    exit 1
fi
gradle_version="$(gradle --version | sed -n 's/^Gradle //p' | head -n 1)"
if [[ "$gradle_version" != "9.5.0" ]]; then
    printf 'Gradle 9.5.0 is required; found %s.\n' "${gradle_version:-unknown}" >&2
    exit 1
fi

gradle --no-daemon --project-dir bindings/android \
    :core:clean \
    :core:testReleaseUnitTest \
    :core:lintRelease \
    :core:assembleRelease
aar_directory="$repo_root/bindings/android/core/build/outputs/aar"
aar_path="$aar_directory/core-release.aar"
if [[ ! -f "$aar_path" ]]; then
    printf '%s\n' 'Android AAR was not produced at the expected path.' >&2
    exit 1
fi
source_revision="$(git rev-parse HEAD)"
if [[ -n "$(git status --short)" ]]; then
    source_revision="${source_revision}-dirty"
fi
package_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
abi_major="$(sed -n 's/^#define LM_ABI_VERSION_MAJOR UINT32_C(\([0-9][0-9]*\))$/\1/p' contracts/abi/linguamesh.h)"
protocol_version="$(sed -n 's/^#define LM_PROTOCOL_VERSION UINT32_C(\([0-9][0-9]*\))$/\1/p' contracts/abi/linguamesh.h)"
if [[ -z "$package_version" || -z "$abi_major" || -z "$protocol_version" ]]; then
    printf '%s\n' 'Android build metadata could not be resolved.' >&2
    exit 1
fi
cat > "$aar_directory/build-metadata.json" <<EOF
{
  "schema_version": 1,
  "package_version": "$package_version",
  "abi_major": $abi_major,
  "protocol_version": $protocol_version,
  "android_abis": [
    "arm64-v8a",
    "armeabi-v7a",
    "x86_64"
  ],
  "source_revision": "$source_revision",
  "artifact_status": "prerelease"
}
EOF
(
    cd "$aar_directory"
    sha256sum core-release.aar build-metadata.json > SHA256SUMS
    sha256sum --check SHA256SUMS
)
printf '%s\n' 'Android AAR build completed.'
