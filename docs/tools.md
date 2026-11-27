# MCP tools

The server exposes tools that call the tenant-scoped mPaka REST API:

| Tool | Purpose |
| --- | --- |
| `execute_python` | Submit Python code with an idempotency key and optional resource profile or environment. |
| `get_execution` | Retrieve an execution by ID. |
| `cancel_execution` | Request cancellation of an execution. |
| `list_executions` | List executions with supported filters and pagination. |
| `wait_execution` | Poll an execution for a bounded period. |
| `create_environment` | Request a Python environment from pinned package requirements. |
| `get_environment` | Retrieve an environment by ID. |
| `list_environments` | List environments with pagination. |
| `wait_environment` | Poll an environment for a bounded period. |
| `get_resource_profiles` | Read API execution limits and resource profiles. |
| `get_usage` | Read tenant usage records and aggregates. |
| `get_usage_summary` | Read aggregate usage totals and optional daily breakdowns for a period. |
| `get_quota` | Read tenant quota information. |

`get_usage_summary` defaults to the last 30 days. Pass RFC 3339 `since` and
`until` values for a specific period, and `group_by: "day"` to request daily
rows. The result includes attempt counts, metering coverage counts, and
provisioned CPU and memory time; it does not represent a monetary charge.

The API validates and authorizes every operation. Use stable idempotency keys
for create/submit calls and reuse the same key and payload when retrying an
ambiguous response. Code, requirements, stdout, and stderr are untrusted data.

Environment lifecycle states follow the API contract: `queued` and `building`
are pending; `ready` and `failed` are terminal. `wait_environment` returns the
latest state when its bounded wait expires. Environment and execution tools
send the caller's `environment_id` unchanged to the API. Structured API errors
preserve the API's `error.code` in the MCP tool error, including
`invalid_requirements`, `environment_not_found`, `environment_not_ready`, and
`environments_unavailable`; locally generated validation and transport errors
use MCP-specific codes.

For controlled deployment acceptance, CLI runs first to create and verify one
`requests==2.32.3` environment. MCP then calls `wait_environment` and
`execute_python` with that same `environment_id`, waits for completion, and
checks stdout `2.32.3`, exit code `0`, Firecracker mode, and the artifact SHA-256
reported by the environment. Use distinct idempotency keys for each client
operation. The public API does not return the durable Builder job ID or the
persisted execution environment reference; correlate these through Builder
records/logs and the execution database row. The shared smoke runner leaves
these checks pending for operator joins and the OpenAPI fixture comparison are described in the RustBox
[`python-environment-campaign.md`](https://github.com/antpard/rustbox/blob/main/docs/operations/python-environment-campaign.md).
The fixture in `contracts/openapi.json` is synchronized from RustBox's
published contract.
