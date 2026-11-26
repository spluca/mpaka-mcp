# Security Policy

## Reporting a vulnerability

Please report security vulnerabilities privately to the maintainers of
`spluca/mpaka-mcp`. Do not include API keys, private source code, or customer
data in public issues. Include the affected version, impact, and steps needed
to reproduce the issue using non-sensitive test data.

The MCP server stores no API key itself. Configure tenant-scoped API keys in
your MCP client's secret or environment configuration and rotate any key that
may have been exposed.
