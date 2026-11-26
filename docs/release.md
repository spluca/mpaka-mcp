# Release and Cloudflare distribution

The primary download host is `https://mcp.mpaka.dev`. A release is started by
pushing an annotated tag named `mpaka-mcp-vMAJOR.MINOR.PATCH` matching
`Cargo.toml`. The workflow builds Linux x86_64 and ARM64, macOS Intel and
Apple Silicon, and Windows x86_64 binaries. It signs the versioned manifest,
publishes immutable release objects to Cloudflare R2, verifies them through
the public Worker, and advances `latest` only after verification. GitHub
Releases remain a secondary mirror.

## Cloudflare configuration

Create the private R2 bucket `mpaka-mcp-releases`. Deploy the Worker in
`mcp-distribution/` with the `MCP_RELEASES` binding and custom domain
`mcp.mpaka.dev`. Keep it separate from the public landing Worker. Downloads
must be publicly readable without a Cloudflare Access login; this hostname
must not proxy to `app.mpaka.dev` or the REST API. Confirm the Worker returns
404 for unlisted objects and 405 for methods other than GET and HEAD.

Install the Worker's JavaScript tooling with `pnpm install --frozen-lockfile`
from `mcp-distribution/`; run its tests with `pnpm test` and validate the
configuration with `pnpm deploy:dry-run` before deploying it.

## GitHub configuration

Create a protected GitHub environment named `release` and restrict it to tags
matching `mpaka-mcp-v*`. Configure:

- Secret `MPAKA_MCP_RELEASE_SIGNING_KEY`: the independent RSA-3072 private key
  whose public key is embedded in `install`, `install.ps1`, and
  `mcp-distribution/release-public.pem` (SPKI SHA-256
  `ca569c222fcc2403e2e8d44bc138ac1fe2d7a7be08c91e0c1b9545bbe1f54b56`). Keep
  the private key out of Git and do not reuse the CLI signing key.
- Secret `MPAKA_MCP_CLOUDFLARE_API_TOKEN`: scoped to deploy this Worker and
  write objects to the `mpaka-mcp-releases` bucket.
- Variable `MPAKA_MCP_CLOUDFLARE_ACCOUNT_ID`: the Cloudflare account ID.

The release workflow fails before publication if the private signing key does
not match the embedded public key. Pull request checks never receive the
private key or Cloudflare token.

## Release, rollback, and key rotation

The tag workflow builds and smoke-tests all five targets, creates and verifies
the signed `manifest.json`, and uploads `releases/<version>/` as an immutable
prefix. It then downloads every public object back and compares the bytes
before it writes `latest`. The Bash and PowerShell installers are published
last, after `latest` points to a release they can verify. Re-running a tag
cannot overwrite an existing release with different contents or move `latest`
backward.

To roll back, choose a version that is already published and move only the R2
`latest` object to that `vMAJOR.MINOR.PATCH` value. Do not overwrite or delete
versioned objects. Verify the new pointer and all public manifest/signature
objects through `https://mcp.mpaka.dev`.

For key rotation, create and embed a new public key, publish a transition
release signed by the current key, then update the installers and workflow to
trust the next key before moving to releases signed only by that key. Freeze
the old pointer on a compatible transition release; never silently replace a
trusted public key in an already published installer.

Before tagging, run `make ci`. GitHub artifact attestations are optional and
are enabled with repository variable `MPAKA_MCP_RELEASE_ATTESTATIONS=true`.
