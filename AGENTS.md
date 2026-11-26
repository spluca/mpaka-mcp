# Repository Guidelines

## Project Structure

This repository is a Rust 2024 MCP server. The binary entry point is `src/main.rs`; server configuration, API transport, and most contract tests are in `src/lib.rs`; MCP tool definitions and their tests are in `src/tools.rs`. User-facing tool documentation lives in `docs/tools.md`. Release and protocol smoke helpers are in `scripts/`, and CI workflows are in `.github/workflows/`.

## Build, Test, and Development

Use a stable Rust toolchain and keep dependency resolution locked:

- `cargo build --locked --release` builds the distributable binary.
- `cargo fmt --all -- --check` checks formatting.
- `cargo nextest run --locked` runs the Rust test suite (install `cargo-nextest` first).
- `cargo clippy --locked --all-targets -- -D warnings` checks all targets with warnings treated as errors.
- `make smoke-test` builds the release binary and exercises all MCP tools; its Python smoke-test dependency is listed in `scripts/requirements-mcp-smoke.txt`.
- `make ci` runs the complete local CI sequence, including release packaging checks.

## Coding Style and Naming

Follow `rustfmt` defaults (four-space indentation) and idiomatic Rust naming: `snake_case` for functions, modules, and variables; `UpperCamelCase` for types and traits; and `SCREAMING_SNAKE_CASE` for constants. Keep changes focused, make API DTOs and REST paths match the public mPaka API contract, and document tool behavior in `docs/tools.md` when it changes.

## Testing

Tests are colocated in Rust `#[cfg(test)]` modules and use `#[test]` or `#[tokio::test]`; HTTP contract tests use a local mocked API. Add or update tests for behavior changes. Run the narrow test or `cargo nextest run --locked`, then run `make ci` before submitting a pull request. The protocol smoke test requires the built binary and the listed Python validator dependency.

## Commits and Pull Requests

Recent commits use short, imperative, categorized subjects such as `feat: ...` and `build: ...`; follow that pattern. Keep pull requests focused, describe user-visible and API-contract changes, link relevant issues, and report validation performed. Include screenshots only when a change has a visual component.

## Security and Configuration

Never commit API keys, `.env` files, customer data, or private deployment details. Configure `MPAKA_API_KEY` through the MCP client's environment or secret store. Keep stdout reserved for MCP protocol messages; diagnostics belong on stderr.
