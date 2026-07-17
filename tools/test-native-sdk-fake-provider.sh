#!/usr/bin/env bash
set -euo pipefail

# 从仓库根目录运行可执行文件并将临时状态限制在独立目录。
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
test_dir="$(mktemp -d)"
provider_pid=""

# 在所有退出路径上停止服务并删除测试临时目录。
cleanup() {
    if [[ -n "$provider_pid" ]] && kill -0 "$provider_pid" 2>/dev/null; then
        kill -INT "$provider_pid" 2>/dev/null || true
        wait "$provider_pid" 2>/dev/null || true
    fi
    rm -rf -- "$test_dir"
}
trap cleanup EXIT INT TERM

cd "$repo_root"
cargo build --locked -p linguamesh-cli
"$repo_root/target/debug/linguamesh-cli" fake-provider --port 0 \
    >"$test_dir/provider.log" 2>&1 &
provider_pid="$!"

# 等待服务打印系统选择的端点，且拒绝静默的提前退出。
endpoint=""
for _ in $(seq 1 100); do
    endpoint="$(sed -n 's/^Fake provider endpoint: //p' "$test_dir/provider.log" | head -n 1)"
    if [[ -n "$endpoint" ]]; then
        break
    fi
    if ! kill -0 "$provider_pid" 2>/dev/null; then
        sed -n '1,120p' "$test_dir/provider.log" >&2
        printf '%s\n' 'Fake provider exited before reporting its endpoint.' >&2
        exit 1
    fi
    sleep 0.05
done
if [[ -z "$endpoint" ]]; then
    printf '%s\n' 'Fake provider did not report its endpoint.' >&2
    exit 1
fi

models="$(curl --fail --silent --show-error "${endpoint}models")"
if [[ "$models" != *'fake-translator'* ]] || [[ "$models" != *'fake-slow-translator'* ]]; then
    printf '%s\n' 'Fake provider returned an unexpected model catalog.' >&2
    exit 1
fi

kill -INT "$provider_pid"
wait "$provider_pid"
provider_pid=""
if ! grep -Fqx 'Fake provider stopped.' "$test_dir/provider.log"; then
    printf '%s\n' 'Fake provider did not shut down cleanly.' >&2
    exit 1
fi
printf '%s\n' "Fake provider CLI smoke test passed at $endpoint"
