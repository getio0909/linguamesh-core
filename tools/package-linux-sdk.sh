#!/usr/bin/env bash
set -euo pipefail

# 从仓库根目录解析所有输入和输出。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

for required_tool in cargo git gzip sha256sum tar; do
    if ! command -v "$required_tool" >/dev/null; then
        printf 'Required tool is unavailable: %s\n' "$required_tool" >&2
        exit 1
    fi
done

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
abi_major="$(sed -n 's/^#define LM_ABI_VERSION_MAJOR UINT32_C(\([0-9][0-9]*\))$/\1/p' contracts/abi/linguamesh.h)"
protocol_version="$(sed -n 's/^#define LM_PROTOCOL_VERSION UINT32_C(\([0-9][0-9]*\))$/\1/p' contracts/abi/linguamesh.h)"
if [[ -z "$version" || -z "$abi_major" || -z "$protocol_version" ]]; then
    printf '%s\n' 'Linux build metadata could not be resolved.' >&2
    exit 1
fi
target="$(rustc -vV | sed -n 's/^host: //p')"
source_revision="$(git rev-parse HEAD)"
if [[ -n "$(git status --short)" ]]; then
    source_revision="${source_revision}-dirty"
fi
package_name="linguamesh-core-sdk-${version}-${target}"
output_root="$repo_root/dist/native-sdk"
archive_path="$output_root/$package_name.tar.gz"

cargo build --locked --release -p linguamesh-ffi

# 仅在系统临时目录中构建归档树。
staging_root="$(mktemp -d)"
cleanup() {
    rm -rf -- "$staging_root"
}
trap cleanup EXIT
package_root="$staging_root/$package_name"
mkdir -p \
    "$package_root/include/linguamesh" \
    "$package_root/lib/pkgconfig" \
    "$package_root/share/linguamesh/contracts"

install -m 0644 contracts/abi/linguamesh.h "$package_root/include/linguamesh.h"
install -m 0644 \
    bindings/cpp/include/linguamesh/linguamesh.hpp \
    "$package_root/include/linguamesh/linguamesh.hpp"
install -m 0644 target/release/liblinguamesh_ffi.so "$package_root/lib/liblinguamesh_ffi.so"
install -m 0644 target/release/liblinguamesh_ffi.a "$package_root/lib/liblinguamesh_ffi.a"
install -m 0644 contracts/proto/linguamesh.proto "$package_root/share/linguamesh/contracts/linguamesh.proto"
install -m 0644 LICENSE THIRD_PARTY_NOTICES.md "$package_root/share/linguamesh/"

sed \
    -e "s|@PREFIX@|/usr/local|g" \
    -e "s|@VERSION@|$version|g" \
    bindings/linux/linguamesh-core.pc.in \
    > "$package_root/lib/pkgconfig/linguamesh-core.pc"

cat > "$package_root/share/linguamesh/build-metadata.json" <<EOF
{
  "schema_version": 1,
  "package_version": "$version",
  "abi_major": $abi_major,
  "protocol_version": $protocol_version,
  "target": "$target",
  "source_revision": "$source_revision",
  "artifact_status": "prerelease"
}
EOF

(
    cd "$package_root"
    find . -type f ! -name SHA256SUMS -print0 \
        | LC_ALL=C sort -z \
        | xargs -0 sha256sum \
        > SHA256SUMS
)

mkdir -p "$output_root"
rm -f -- "$archive_path" "$archive_path.sha256"
tar \
    --sort=name \
    --mtime='@0' \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    --format=posix \
    --pax-option=delete=atime,delete=ctime \
    -C "$staging_root" \
    -cf - \
    "$package_name" \
    | gzip -n > "$archive_path"
(
    cd "$output_root"
    sha256sum "$package_name.tar.gz" > "$package_name.tar.gz.sha256"
)
printf 'Linux SDK package created: %s\n' "$archive_path"
