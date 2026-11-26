# Contributing

Contributions should be focused and include tests for behavior changes.

Before opening a pull request, run:

```sh
cargo fmt --all -- --check
cargo nextest run
cargo clippy --all-targets -- -D warnings
bash scripts/mcp-release-test.sh
```

Keep API keys, `.env` files, customer data, and private platform deployment
details out of commits. Changes to REST paths or DTOs must match the public
mPaka API contract.
