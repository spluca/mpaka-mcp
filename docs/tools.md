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
