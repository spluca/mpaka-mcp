import test from 'node:test';
import assert from 'node:assert/strict';
import { keyForPath, fetch } from '../src/index.js';

test('allows only published MCP paths and supported archives', () => {
  assert.equal(keyForPath('/install'), 'install');
  assert.equal(keyForPath('/install.ps1'), 'install.ps1');
  assert.equal(keyForPath('/latest'), 'latest');
  assert.equal(keyForPath('/releases/v0.1.0/manifest.json'), 'releases/v0.1.0/manifest.json');
  assert.equal(keyForPath('/releases/v0.1.0/manifest.json.sig'), 'releases/v0.1.0/manifest.json.sig');
  assert.equal(keyForPath('/releases/v0.1.0/mpaka-mcp-v0.1.0-aarch64-unknown-linux-gnu.tar.gz'), 'releases/v0.1.0/mpaka-mcp-v0.1.0-aarch64-unknown-linux-gnu.tar.gz');
  assert.equal(keyForPath('/releases/v0.1.0/unlisted-file'), null);
  assert.equal(keyForPath('/releases/latest/manifest.json'), null);
});

test('rejects traversal, encoded separators, and invalid versions', () => {
  for (const path of ['/../secret', '/releases/v0.1.0/../../secret', '/%2e%2e/secret', '/releases/v1.2/manifest.json']) {
    assert.equal(keyForPath(path), null, path);
  }
});

test('serves immutable release objects and rejects other methods', async () => {
  const worker = { async get(key) { return { body: `object:${key}`, httpEtag: '"tag"' }; } };
  const response = await fetch(new Request('https://mcp.mpaka.dev/latest'), { MCP_RELEASES: worker });
  assert.equal(response.status, 200);
  assert.match(response.headers.get('cache-control'), /no-cache/);
  const asset = await fetch(new Request('https://mcp.mpaka.dev/releases/v0.1.0/manifest.json'), { MCP_RELEASES: worker });
  assert.match(asset.headers.get('cache-control'), /immutable/);
  const post = await fetch(new Request('https://mcp.mpaka.dev/latest', { method: 'POST' }), { MCP_RELEASES: worker });
  assert.equal(post.status, 405);
});
