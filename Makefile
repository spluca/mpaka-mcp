.PHONY: help fmt test clippy build smoke-test release-test ci

help:
	@printf '%s\n' \
		'mPaka MCP development commands:' \
		'  make fmt          Check Rust formatting' \
		'  make test         Run tests with nextest' \
		'  make clippy       Run Clippy with warnings denied' \
		'  make build        Build the release binary' \
		'  make smoke-test   Exercise all MCP tools using the built binary' \
		'  make release-test Check release packaging and smoke validators' \
		'  make ci            Run all local CI checks'

fmt:
	cargo fmt --all -- --check

test:
	cargo nextest run --locked

clippy:
	cargo clippy --locked --all-targets -- -D warnings

build:
	cargo build --locked --release

smoke-test: build
	python3 scripts/mcp-binary-smoke.py target/release/mpaka-mcp

release-test:
	bash scripts/mcp-release-test.sh

ci: fmt test clippy release-test smoke-test
