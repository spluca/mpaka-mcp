# mpaka-mcp

`mpaka-mcp` is a local stdio [Model Context Protocol](https://modelcontextprotocol.io/)
server that forwards tool requests to the mPaka REST API. It does not execute
submitted code on the MCP host; execution behavior is provided by the API.

## Build and install

Install a stable Rust toolchain, then build the release binary:

```sh
cargo build --locked --release
```

The binary is `target/release/mpaka-mcp` (or `mpaka-mcp.exe` on Windows).
Prebuilt archives are published on the GitHub Releases page when a release is
created.

## Configure an MCP client

For clients using the `mcpServers` configuration format:

```json
{
  "mcpServers": {
    "mpaka": {
      "command": "mpaka-mcp",
      "env": {
        "MPAKA_API_KEY": "your-tenant-scoped-api-key",
        "MPAKA_API_BASE_URL": "https://your-mpaka-api.example"
      }
    }
  }
}
```

`MPAKA_API_KEY` is required. `MPAKA_API_BASE_URL` defaults to
`http://127.0.0.1:5300` for local development. Plain HTTP is accepted only for
loopback addresses; remote API URLs must use HTTPS. `MPAKA_API_TIMEOUT_SECONDS`
sets the request timeout (default 15 seconds).

The server reserves stdout for MCP protocol messages and writes diagnostics to
stderr. Keep API keys in the MCP client's environment or secret store; do not
put them in tool arguments or commit them to configuration files.

## Available tools

- `execute_python`, `get_execution`, `cancel_execution`, `list_executions`,
  `wait_execution`
- `create_environment`, `get_environment`, `list_environments`,
  `wait_environment`
- `get_resource_profiles`, `get_usage`, `get_quota`

See [docs/tools.md](docs/tools.md) for arguments and behavior.

## Development

```sh
cargo fmt --all -- --check
cargo nextest run
cargo clippy --all-targets -- -D warnings
bash scripts/mcp-release-test.sh
```

The HTTP contract tests use a local mocked API. They do not prove the safety or
isolation properties of the mPaka execution service.
