# Releases

Releases are started by pushing an annotated tag named
`mpaka-mcp-vMAJOR.MINOR.PATCH` matching `Cargo.toml`. The workflow validates
the package, builds Linux x86_64, macOS arm64, and Windows x86_64 binaries,
packages each binary, and publishes checksums and a GitHub Release.

Before tagging, run the development checks in the README. The optional GitHub
artifact attestations are enabled by setting the repository variable
`MPAKA_MCP_RELEASE_ATTESTATIONS` to `true`.
