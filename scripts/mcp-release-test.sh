#!/usr/bin/env bash
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workflow="$root/.github/workflows/release.yml"
test_script="$root/scripts/mcp-release.py"

grep -q '^  push:$' "$workflow"
grep -q 'mpaka-mcp-v\*' "$workflow"
grep -q 'actions/attest@v4' "$workflow"
grep -q 'actions/setup-python@v7' "$workflow"
grep -q "vars.MPAKA_MCP_RELEASE_ATTESTATIONS == 'true'" "$workflow"
grep -q 'gh release create' "$workflow"
grep -q 'cargo build --locked --release -p mpaka-mcp' "$workflow"
grep -q 'MPAKA_MCP_RELEASE_VERSION' "$workflow"
grep -q 'tools/list' "$root/scripts/mcp-binary-smoke.py"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/dist"
printf 'linux binary fixture\n' > "$tmp/bin/linux"
printf 'macos binary fixture\n' > "$tmp/bin/macos"
printf 'windows binary fixture\n' > "$tmp/bin/windows"

python3 "$test_script" package --version 0.1.0 --target x86_64-unknown-linux-gnu --binary "$tmp/bin/linux" --output-dir "$tmp/dist"
python3 "$test_script" package --version 0.1.0 --target aarch64-apple-darwin --binary "$tmp/bin/macos" --output-dir "$tmp/dist"
python3 "$test_script" package --version 0.1.0 --target x86_64-pc-windows-msvc --binary "$tmp/bin/windows" --output-dir "$tmp/dist"
python3 "$test_script" manifest --version 0.1.0 --release-dir "$tmp/dist"
python3 "$test_script" verify --version 0.1.0 --release-dir "$tmp/dist"
python3 "$root/scripts/mcp-binary-smoke-test.py"
grep -q 'mpaka-mcp-v0.1.0-x86_64-unknown-linux-gnu.tar.gz' "$tmp/dist/SHA256SUMS"
grep -q 'mpaka-mcp-v0.1.0-aarch64-apple-darwin.tar.gz' "$tmp/dist/SHA256SUMS"
grep -q 'mpaka-mcp-v0.1.0-x86_64-pc-windows-msvc.zip' "$tmp/dist/SHA256SUMS"

echo 'MCP release tests passed'
