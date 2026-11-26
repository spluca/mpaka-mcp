#!/usr/bin/env bash
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
workflow="$root/.github/workflows/release.yml"
test_script="$root/scripts/mcp-release.py"

grep -q '^  push:$' "$workflow"
grep -q 'mpaka-mcp-v\*' "$workflow"
grep -q 'actions/attest@v4' "$workflow"
grep -q 'actions/setup-python@v7' "$workflow"
grep -q 'mcp.mpaka.dev' "$workflow"
grep -q 'MPAKA_MCP_CLOUDFLARE_API_TOKEN' "$workflow"
grep -q 'MPAKA_MCP_RELEASE_SIGNING_KEY' "$workflow"
grep -q 'gh release create' "$workflow"
grep -q 'cargo build --locked --release -p mpaka-mcp' "$workflow"
grep -q 'MPAKA_MCP_RELEASE_VERSION' "$workflow"
grep -q 'tools/list' "$root/scripts/mcp-binary-smoke.py"
grep -q 'manifest.json.sig' "$root/install"
grep -q 'manifest.json.sig' "$root/install.ps1"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/dist"
openssl genrsa -out "$tmp/signing-key.pem" 2048 2>/dev/null
openssl pkey -in "$tmp/signing-key.pem" -pubout -out "$tmp/public.pem" 2>/dev/null
for target in \
  x86_64-unknown-linux-gnu \
  aarch64-unknown-linux-gnu \
  x86_64-apple-darwin \
  aarch64-apple-darwin \
  x86_64-pc-windows-msvc; do
  printf 'binary fixture for %s\n' "$target" > "$tmp/bin/$target"
  python3 "$test_script" package --version 0.1.0 --target "$target" --binary "$tmp/bin/$target" --output-dir "$tmp/dist"
done
python3 "$test_script" manifest --version 0.1.0 --release-dir "$tmp/dist" --signing-key "$tmp/signing-key.pem"
python3 "$test_script" verify --version 0.1.0 --release-dir "$tmp/dist" --public-key "$tmp/public.pem"
printf 'tampered' > "$tmp/dist/manifest.json.sig"
if python3 "$test_script" verify --version 0.1.0 --release-dir "$tmp/dist" --public-key "$tmp/public.pem" >/dev/null 2>&1; then
  echo 'release verifier accepted a tampered manifest signature' >&2
  exit 1
fi
if [[ "${MPAKA_MCP_SKIP_SOCKET_SMOKE:-0}" != 1 ]]; then
  python3 "$root/scripts/mcp-binary-smoke-test.py"
fi
grep -q 'mpaka-mcp-v0.1.0-x86_64-unknown-linux-gnu.tar.gz' "$tmp/dist/SHA256SUMS"
grep -q 'mpaka-mcp-v0.1.0-aarch64-unknown-linux-gnu.tar.gz' "$tmp/dist/SHA256SUMS"
grep -q 'mpaka-mcp-v0.1.0-x86_64-apple-darwin.tar.gz' "$tmp/dist/SHA256SUMS"
grep -q 'mpaka-mcp-v0.1.0-aarch64-apple-darwin.tar.gz' "$tmp/dist/SHA256SUMS"
grep -q 'mpaka-mcp-v0.1.0-x86_64-pc-windows-msvc.zip' "$tmp/dist/SHA256SUMS"
cd "$root/mcp-distribution"
node --test test/worker.test.mjs

echo 'MCP release tests passed'
