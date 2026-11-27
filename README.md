# mpaka-mcp

[![CI](https://github.com/spluca/mpaka-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/spluca/mpaka-mcp/actions/workflows/ci.yml)

`mpaka-mcp` is a local stdio [Model Context Protocol](https://modelcontextprotocol.io/)
server that forwards tool requests to the mPaka REST API. It does not execute
submitted code on the MCP host; execution behavior is provided by the API.

## Build and install

Install a stable Rust toolchain, then build the release binary:

```sh
cargo build --locked --release
```

The binary is `target/release/mpaka-mcp` (or `mpaka-mcp.exe` on Windows).
The release workflow is configured to publish signed prebuilt archives at
`https://mcp.mpaka.dev`, with GitHub Releases as a secondary mirror. The first
public release requires the Cloudflare bucket, Worker, and release secrets in
[docs/release.md](docs/release.md) to be configured.

After the Cloudflare release channel has been configured and its first release
published, install the latest signed release on Linux or macOS:

```sh
curl -fsSL https://mcp.mpaka.dev/install | bash
```

Install a pinned Unix version with `--version vMAJOR.MINOR.PATCH`. On Windows
PowerShell 7+, use `irm https://mcp.mpaka.dev/install.ps1 | iex`; set
`$env:MPAKA_MCP_VERSION` first to pin a version. Both installers verify the
signed manifest, archive size, and SHA-256 before installing.

## Configure an MCP client

For clients using the `mcpServers` configuration format:

```json
{
  "mcpServers": {
    "mpaka": {
      "command": "mpaka-mcp",
      "env": {
        "MPAKA_API_KEY": "your-tenant-scoped-api-key",
        "MPAKA_API_URL": "https://api.mpaka.dev"
      }
    }
  }
}
```

`MPAKA_API_KEY` is required. `MPAKA_API_URL` defaults to
`https://api.mpaka.dev`. `MPAKA_API_BASE_URL` remains supported as a legacy
alias when `MPAKA_API_URL` is unset. Plain HTTP is accepted only for loopback
addresses; remote API URLs must use HTTPS. `MPAKA_API_TIMEOUT_SECONDS` sets
the request timeout (default 15 seconds).

The server reserves stdout for MCP protocol messages and writes diagnostics to
stderr. Keep API keys in the MCP client's environment or secret store; do not
put them in tool arguments or commit them to configuration files.

## Available tools

- `execute_python`, `get_execution`, `cancel_execution`, `list_executions`,
  `wait_execution`
- `create_environment`, `get_environment`, `list_environments`,
  `wait_environment`
- `get_resource_profiles`, `get_usage`, `get_usage_summary`, `get_quota`

See [docs/tools.md](docs/tools.md) for arguments and behavior.
See [docs/client-setup.md](docs/client-setup.md) for client-specific setup.
See [docs/integration.md](docs/integration.md) for live checks and
client-side error handling.

## Development

Run the full local CI checks with `make ci`. Individual targets are listed by
`make help` and include formatting, nextest, Clippy, a release build, the
13-tool protocol smoke test, and release packaging checks.

The HTTP contract tests use a local mocked API. They do not prove the safety or
isolation properties of the mPaka execution service.
