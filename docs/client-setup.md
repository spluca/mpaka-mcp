# MCP client setup

The server uses stdio. Install `mpaka-mcp` on the same machine or execution
environment as the MCP client, then provide `MPAKA_API_KEY`. The default API
endpoint is `https://api.mpaka.dev`; set `MPAKA_API_URL` only for a different
endpoint.

Keep the API key in a user-local configuration or secret store. Do not commit
it to a project configuration file.

## Claude Desktop

In Claude Desktop's `claude_desktop_config.json`, add this entry under
`mcpServers`:

```json
{
  "mcpServers": {
    "mpaka": {
      "command": "mpaka-mcp",
      "env": {
        "MPAKA_API_KEY": "YOUR_MPAKA_API_KEY",
        "MPAKA_API_URL": "https://api.mpaka.dev"
      }
    }
  }
}
```

Restart Claude Desktop after saving the configuration. See Anthropic's
[MCP documentation](https://modelcontextprotocol.io/docs/develop/connect-local-servers)
for local stdio servers and configuration.

## Claude Code

Register the local process from a terminal:

```sh
claude mcp add --transport stdio --env MPAKA_API_KEY=YOUR_MPAKA_API_KEY mpaka -- mpaka-mcp
```

For a personal server, add `--scope user`. The CLI writes its configuration;
keep that user configuration private. See the official
[Claude Code MCP guide](https://code.claude.com/docs/en/mcp).

## Codex

Export `MPAKA_API_KEY` in the environment that launches Codex, then add this to
`~/.codex/config.toml`:

```toml
[mcp_servers.mpaka]
command = "mpaka-mcp"
env_vars = ["MPAKA_API_KEY"]

[mcp_servers.mpaka.env]
MPAKA_API_URL = "https://api.mpaka.dev"
```

Codex also supports project-scoped configuration in `.codex/config.toml`.
Avoid placing the API key in a shared project file. See the official
[Codex MCP configuration guide](https://chatgpt.com/docs/extend/mcp).

## Cursor

Add this entry to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "mpaka": {
      "command": "mpaka-mcp",
      "env": {
        "MPAKA_API_KEY": "${env:MPAKA_API_KEY}",
        "MPAKA_API_URL": "https://api.mpaka.dev"
      }
    }
  }
}
```

Set `MPAKA_API_KEY` in the environment used to launch Cursor. See
[Cursor's MCP guide](https://docs.cursor.com/context/mcp).

## Visual Studio Code with GitHub Copilot

For the current Copilot harness, add a workspace `.mcp.json` (or the user
`mcp-config.json`) using the portable `mcpServers` format:

```json
{
  "mcpServers": {
    "mpaka": {
      "type": "stdio",
      "command": "mpaka-mcp",
      "env": {
        "MPAKA_API_KEY": "${MPAKA_API_KEY}",
        "MPAKA_API_URL": "https://api.mpaka.dev"
      }
    }
  }
}
```

Provide `MPAKA_API_KEY` to the host running the Copilot harness. Do not commit a
literal key in a workspace file. See Microsoft's
[MCP configuration reference](https://code.visualstudio.com/docs/agents/reference/mcp-configuration).
