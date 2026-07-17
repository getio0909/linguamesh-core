#!/usr/bin/env bash
set -euo pipefail

# 从仓库根目录解析所有输入和输出。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

for required_tool in cargo rustup lipo xcodebuild swift zip shasum; do
    if ! command -v "$required_tool" >/dev/null; then
        printf 'Required tool is unavailable: %s\n' "$required_tool" >&2
        exit 1
    fi
done

artifact_root="$repo_root/bindings/apple/Artifacts"
xcframework_path="$artifact_root/LinguaMeshCore.xcframework"
header_root="$artifact_root/headers"

# 仅清理由本脚本在已忽略产物目录中创建的内容。
rm -rf -- "$artifact_root"
mkdir -p "$header_root"
install -m 0644 contracts/abi/linguamesh.h "$header_root/linguamesh.h"
install -m 0644 bindings/apple/xcframework/module.modulemap "$header_root/module.modulemap"
source_revision="$(git rev-parse HEAD)"
if [[ -n "$(git status --short)" ]]; then
    source_revision="${source_revision}-dirty"
fi
package_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
abi_major="$(sed -n 's/^#define LM_ABI_VERSION_MAJOR UINT32_C(\([0-9][0-9]*\))$/\1/p' contracts/abi/linguamesh.h)"
protocol_version="$(sed -n 's/^#define LM_PROTOCOL_VERSION UINT32_C(\([0-9][0-9]*\))$/\1/p' contracts/abi/linguamesh.h)"
if [[ -z "$package_version" || -z "$abi_major" || -z "$protocol_version" ]]; then
    printf '%s\n' 'Apple build metadata could not be resolved.' >&2
    exit 1
fi
sed \
    -e "s|@PACKAGE_VERSION@|$package_version|g" \
    -e "s|\"abi_major\": [0-9][0-9]*|\"abi_major\": $abi_major|g" \
    -e "s|\"protocol_version\": [0-9][0-9]*|\"protocol_version\": $protocol_version|g" \
    -e "s|@SOURCE_REVISION@|$source_revision|g" \
    bindings/apple/xcframework/build-metadata.json \
    > "$artifact_root/build-metadata.json"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --locked --release -p linguamesh-ffi --target aarch64-apple-darwin
cargo build --locked --release -p linguamesh-ffi --target x86_64-apple-darwin

# 同一平台的架构必须先合并为一个通用库再创建 XCFramework。
universal_library_root="$artifact_root/universal-macos"
mkdir -p "$universal_library_root"
universal_library="$universal_library_root/liblinguamesh_ffi.a"
lipo -create \
    target/aarch64-apple-darwin/release/liblinguamesh_ffi.a \
    target/x86_64-apple-darwin/release/liblinguamesh_ffi.a \
    -output "$universal_library"
lipo -verify_arch arm64 x86_64 "$universal_library"

xcodebuild -create-xcframework \
    -library "$universal_library" \
    -headers "$header_root" \
    -output "$xcframework_path"

# 归一化时间戳和归档顺序，使相同输入生成稳定 ZIP。
find "$xcframework_path" -exec touch -t 198001010000 {} +
archive_path="$artifact_root/LinguaMeshCore.xcframework.zip"
(
    cd "$artifact_root"
    find LinguaMeshCore.xcframework -type f -print \
        | LC_ALL=C sort \
        | zip -X -q "$archive_path" -@
)
swift package compute-checksum "$archive_path" > "$archive_path.swiftpm-checksum"
(
    cd "$artifact_root"
    shasum -a 256 LinguaMeshCore.xcframework.zip build-metadata.json > SHA256SUMS
    shasum -a 256 --check SHA256SUMS
)
printf '%s\n' 'Apple XCFramework build completed.'
