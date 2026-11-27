# Client integration and recoverable results

## Codex setup

Install `mpaka-mcp` and configure it in `~/.codex/config.toml`. Keep the API key
in the environment used to launch Codex:

```toml
[mcp_servers.mpaka]
command = "mpaka-mcp"
env_vars = ["MPAKA_API_KEY"]

[mcp_servers.mpaka.env]
MPAKA_API_URL = "https://api.mpaka.dev"
```

Restart Codex after changing the configuration. Check that its MCP tool list
contains `create_environment`, `wait_environment`, `execute_python`, and
`wait_execution` before submitting work.

## Live stdio integration check

The protocol smoke test uses a simulated API. The following check starts the
actual binary over stdio and sends requests to the API configured for that
binary with a small JSON-RPC client; it does not launch Codex. Full mode creates
an environment and execution and uses API quota. This validates the server's
stdio protocol, but Codex still needs a separate acceptance run using the
configuration above.

```sh
cargo build --locked --release
MPAKA_API_KEY=... python3 scripts/mcp-live-integration.py
```

If Python environments are unavailable on the selected API deployment, use
`--execution-only` to check direct execution without environment creation:

```sh
MPAKA_API_KEY=... python3 scripts/mcp-live-integration.py --execution-only
```

The optional `--cli-environment-id` verifies an environment created by another
client that can create environments. The mPaka CLI version checked on
2026-11-27 only provides `run`; it has no environment create or select
command, so it cannot supply this ID.

```sh
MPAKA_API_KEY=... python3 scripts/mcp-live-integration.py \
  --cli-environment-id 00000000-0000-0000-0000-000000000000
```

To check controlled timeout and failure fixtures without creating a new
environment or execution, pass their IDs with `--fixtures-only`:

```sh
MPAKA_API_KEY=... python3 scripts/mcp-live-integration.py \
  --fixtures-only \
  --timeout-environment-id 00000000-0000-0000-0000-000000000001 \
  --failed-environment-id 00000000-0000-0000-0000-000000000002
```

The first ID must remain queued or building longer than 100 ms; the second
must already be in the terminal `failed` state. These options assert
`timed_out: true` for the first and `status: failed` with `timed_out: false`
for the second.

When a client-created environment ID is available, the check retrieves that exact ID through MCP,
confirms its state is `ready`, and runs Python in it. It also repeats each
submission with the same idempotency key and payload, then checks that the API
returns the same execution ID. Run it with a test tenant key and the same
`MPAKA_API_URL` used by the CLI. The command prints IDs, states, and stdout for
comparison.

## Results the client should handle

`wait_environment` and `wait_execution` return the latest known resource and a
`timed_out` flag. A wait timeout only means the polling deadline expired; it
does not cancel the environment build or execution. The client can keep the ID
and call the wait tool again, or use `get_environment` / `get_execution` later.

Example for an environment that is still building when the wait deadline ends:

```json
{
  "environment": {"id": "<environment-id>", "status": "building"},
  "timed_out": true
}
```

Live check on 2026-11-27: the configured API returned `Python environments are
not configured` for `create_environment`. Direct execution was accepted but
remained `queued` beyond two 30-second waits; each wait returned
`timed_out: true`. The test execution was then cancelled and the tenant's
active count returned to zero. A terminal failed-environment fixture was not
available, so that state still needs validation on an environment-enabled
test deployment.

When an environment reaches `failed`, `wait_environment` returns that terminal
state with `timed_out: false`. Keep the ID for diagnosis and do not try to
execute in the failed environment. The API response includes the environment
metadata available to that tenant; the MCP server does not invent a failure
reason.

An execution that fails is also a terminal result, distinct from a wait
timeout. Inspect `execution.status`, `execution.exit_code`, `execution.stderr`,
and `execution.stdout`; do not retry a completed or failed execution under a
new idempotency key unless a new run is intended.

If a submit response is lost or ambiguous, resend the exact same arguments
with the same `idempotency_key`. This includes the code, environment ID,
timeout, and resource profile. Reusing the key with changed arguments is not a
valid retry. The live integration check exercises an identical retry and
requires the same execution ID.

After the timeout assertion, call `wait_environment` again and confirm the
resource continues to progress. The exact inputs that cause a build failure
are API/deployment specific, so use the test tenant's documented failure
fixture; do not send deliberately broken package requirements to a production
tenant.
