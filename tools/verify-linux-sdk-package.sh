#!/usr/bin/env bash
set -euo pipefail

# 连续构建两次并比较完整归档的 SHA-256。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
target="$(rustc -vV | sed -n 's/^host: //p')"
if [[ -z "$version" || -z "$target" ]]; then
    printf '%s\n' 'Linux SDK package identity could not be resolved.' >&2
    exit 1
fi
package_name="linguamesh-core-sdk-${version}-${target}"
archive_path="$repo_root/dist/native-sdk/$package_name.tar.gz"
checksum_file="$archive_path.sha256"

bash tools/package-linux-sdk.sh
if [[ ! -f "$checksum_file" ]]; then
    printf '%s\n' 'Linux SDK checksum file was not produced.' >&2
    exit 1
fi
first_checksum="$(cut -d ' ' -f 1 "$checksum_file")"
bash tools/package-linux-sdk.sh
second_checksum="$(cut -d ' ' -f 1 "$checksum_file")"
if [[ "$first_checksum" != "$second_checksum" ]]; then
    printf '%s\n' 'Linux SDK package is not reproducible.' >&2
    exit 1
fi

# 验证外部清单和归档内每个已登记文件。
(
    cd "$(dirname "$archive_path")"
    sha256sum --check "$(basename "$checksum_file")"
)
verification_root="$(mktemp -d)"
cleanup() {
    rm -rf -- "$verification_root"
}
trap cleanup EXIT
tar -xzf "$archive_path" -C "$verification_root"
package_root="$verification_root/$package_name"
if [[ ! -f "$package_root/SHA256SUMS" ]]; then
    printf '%s\n' 'Packaged per-file checksum manifest was not found.' >&2
    exit 1
fi
(
    cd "$package_root"
    sha256sum --check SHA256SUMS
)
if ! command -v cc >/dev/null || ! command -v pkg-config >/dev/null; then
    printf '%s\n' 'A C compiler and pkg-config are required for package validation.' >&2
    exit 1
fi
pkg_config_path="$package_root/lib/pkgconfig"
PKG_CONFIG_PATH="$pkg_config_path" \
    pkg-config --define-variable="prefix=$package_root" --validate linguamesh-core
static_flags="$(
    PKG_CONFIG_PATH="$pkg_config_path" \
        pkg-config --define-variable="prefix=$package_root" --libs --static linguamesh-core
)"
for required_flag in -ldl -lpthread -lm; do
    if [[ " $static_flags " != *" $required_flag "* ]]; then
        printf 'Static pkg-config flag is missing: %s\n' "$required_flag" >&2
        exit 1
    fi
done
cc \
    -std=c11 \
    -Wall \
    -Wextra \
    -Werror \
    -pedantic \
    -I"$package_root/include" \
    tests/native/c_header_smoke.c \
    "$package_root/lib/liblinguamesh_ffi.a" \
    -ldl \
    -lpthread \
    -lm \
    -o "$verification_root/c_static_smoke"
"$verification_root/c_static_smoke"
printf 'Linux SDK package is reproducible: %s\n' "$second_checksum"
