# Contributing

Contributions should be focused and include tests for behavior changes.

Before opening a pull request, run:

```sh
make ci
```

Keep API keys, `.env` files, customer data, and private platform deployment
details out of commits. Changes to REST paths or DTOs must match the public
mPaka API contract.
