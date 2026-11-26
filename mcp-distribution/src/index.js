const VERSION = /^v[0-9]+\.[0-9]+\.[0-9]+$/;
const ARTIFACTS = new Set([
  'x86_64-unknown-linux-gnu.tar.gz',
  'aarch64-unknown-linux-gnu.tar.gz',
  'x86_64-apple-darwin.tar.gz',
  'aarch64-apple-darwin.tar.gz',
  'x86_64-pc-windows-msvc.zip',
]);

export function keyForPath(pathname) {
  let decoded;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (!decoded.startsWith('/') || decoded.includes('..') || decoded.includes('\\') || decoded.includes('//')) return null;
  const relative = decoded.slice(1);
  if (['install', 'install.ps1', 'latest'].includes(relative)) return relative;
  const match = relative.match(/^releases\/(v[0-9]+\.[0-9]+\.[0-9]+)\/(manifest\.json|manifest\.json\.sig|[^/]+)$/);
  if (!match || !VERSION.test(match[1])) return null;
  if (match[2] === 'manifest.json' || match[2] === 'manifest.json.sig' || match[2] === 'SHA256SUMS' || [...ARTIFACTS].some((suffix) => match[2] === `mpaka-mcp-${match[1]}-${suffix}`)) {
    return `releases/${match[1]}/${match[2]}`;
  }
  return null;
}

function contentType(key, metadata) {
  if (key.endsWith('.json')) return 'application/json; charset=utf-8';
  if (key.endsWith('.sig')) return 'application/octet-stream';
  if (key.endsWith('.tar.gz')) return 'application/gzip';
  if (key.endsWith('.zip')) return 'application/zip';
  return metadata?.contentType || 'text/plain; charset=utf-8';
}

export async function fetch(request, env) {
  if (request.method !== 'GET' && request.method !== 'HEAD') return new Response('method not allowed\n', { status: 405 });
  const key = keyForPath(new URL(request.url).pathname);
  if (!key) return new Response('not found\n', { status: 404 });
  const object = await env.MCP_RELEASES.get(key);
  if (!object) return new Response('not found\n', { status: 404 });
  const headers = new Headers({
    'cache-control': key.startsWith('releases/') ? 'public, max-age=31536000, immutable' : 'no-cache, max-age=60, must-revalidate',
    'content-type': contentType(key, object.httpMetadata),
    'x-content-type-options': 'nosniff',
  });
  if (object.httpEtag) headers.set('etag', object.httpEtag);
  return new Response(request.method === 'HEAD' ? null : object.body, { status: 200, headers });
}

export default { fetch };
