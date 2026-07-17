#!/usr/bin/env bash
set -euo pipefail

# 始终从仓库根目录解析头文件和构建输出。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

command -v cc >/dev/null || {
    printf '%s\n' 'A C compiler is required.' >&2
    exit 1
}
command -v g++ >/dev/null || {
    printf '%s\n' 'A C++ compiler is required.' >&2
    exit 1
}

cargo build --locked -p linguamesh-ffi

# 仅在系统临时目录中放置可执行测试产物。
native_test_dir="$(mktemp -d)"
cleanup() {
    rm -rf -- "$native_test_dir"
}
trap cleanup EXIT

cc \
    -std=c11 \
    -Wall \
    -Wextra \
    -Werror \
    -pedantic \
    -Icontracts/abi \
    tests/native/c_header_smoke.c \
    -Ltarget/debug \
    -Wl,-rpath,"$repo_root/target/debug" \
    -llinguamesh_ffi \
    -o "$native_test_dir/c_header_smoke"

g++ \
    -std=c++20 \
    -Wall \
    -Wextra \
    -Werror \
    -pedantic \
    -Icontracts/abi \
    -Ibindings/cpp/include \
    tests/native/cpp_wrapper_smoke.cpp \
    -Ltarget/debug \
    -Wl,-rpath,"$repo_root/target/debug" \
    -llinguamesh_ffi \
    -o "$native_test_dir/cpp_wrapper_smoke"

"$native_test_dir/c_header_smoke"
"$native_test_dir/cpp_wrapper_smoke"
printf '%s\n' 'Native SDK smoke tests passed.'
