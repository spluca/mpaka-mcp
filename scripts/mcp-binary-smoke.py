#!/usr/bin/env python3
"""Start the MCP binary and complete a stdio initialize handshake."""

import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import subprocess
import sys
from threading import Thread
import uuid

from jsonschema import validators


EXPECTED_TOOLS = {
    "execute_python",
    "get_execution",
    "cancel_execution",
    "list_executions",
    "wait_execution",
    "create_environment",
    "get_environment",
    "list_environments",
    "wait_environment",
    "get_resource_profiles",
    "get_usage",
    "get_usage_summary",
    "get_quota",
}
SMOKE_WAIT_TIMEOUT_MS = 5_000


class SmokeApiHandler(BaseHTTPRequestHandler):
    def _send(self, payload: dict) -> None:
        body = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _execution(self) -> dict:
        return {
            "id": "00000000-0000-0000-0000-000000000001", "status": "completed",
            "language": "python", "timeout_ms": 3000, "memory_mb": 128,
            "vcpu_count": 1, "created_at": "2026-11-24T00:00:00Z",
            "finished_at": "2026-11-24T00:00:01Z", "stdout": "ok\n",
            "stderr": "", "exit_code": 0, "mode": "mock_no_code_execution",
            "output_truncated": False,
        }

    def _environment(self) -> dict:
        return {
            "id": "00000000-0000-0000-0000-000000000002", "status": "ready",
            "requirements": ["requests==2.32.3"], "artifact_sha256": None,
            "artifact_size_bytes": None, "resolved_digest": None,
            "created_at": "2026-11-24T00:00:00Z", "updated_at": "2026-11-24T00:00:01Z",
        }

    def do_GET(self) -> None:
        if self.headers.get("Authorization") != "Bearer release-smoke-placeholder":
            self.send_error(401, "Missing smoke authorization")
            return
        path = self.path.split("?", 1)[0]
        if path == "/v1/limits":
            payload = {
                "language": "python",
                "max_code_bytes": 16384,
                "min_timeout_ms": 100,
                "max_timeout_ms": 10000,
                "default_timeout_ms": 3000,
                "min_memory_mb": 128,
                "max_memory_mb": 1024,
                "default_memory_mb": 256,
                "guest_memory_profiles": ["small", "medium"],
                "guest_cpu_profiles": ["default"],
                "min_vcpu_count": 1,
                "max_vcpu_count": 2,
                "default_vcpu_count": 1,
            }
        elif path == "/v1/quota":
            payload = {"used_units": 1, "limit_units": 1000, "active_count": 0, "active_limit": 8}
        elif path == "/v1/usage":
            payload = {
                "since": "2026-11-01T00:00:00Z", "until": "2026-11-24T00:00:00Z",
                "attempt_count": 1, "measured_attempt_count": 1,
                "provisioned_vcpu_ms": 1000, "provisioned_mib_ms": 128000,
                "has_more": False, "next_cursor": None, "items": [],
            }
        elif path == "/v1/usage/summary":
            payload = {
                "since": "2026-11-01T00:00:00Z", "until": "2026-11-24T00:00:00Z",
                "attempt_count": 1, "measured_attempt_count": 1,
                "pending_attempt_count": 0, "unmeasured_terminal_attempt_count": 0,
                "provisioned_vcpu_ms": 1000, "provisioned_mib_ms": 128000,
                "daily": [{
                    "date": "2026-11-23", "attempt_count": 1,
                    "measured_attempt_count": 1, "pending_attempt_count": 0,
                    "unmeasured_terminal_attempt_count": 0,
                    "provisioned_vcpu_ms": 1000, "provisioned_mib_ms": 128000,
                }],
            }
        elif path == "/v1/executions":
            execution = self._execution()
            payload = {"items": [{key: execution[key] for key in ("id", "status", "language", "created_at", "finished_at")}], "page": 1, "page_size": 25, "total": 1, "has_next": False}
        elif path == "/v1/environments":
            payload = {"items": [self._environment()], "page": 1, "page_size": 25, "total": 1, "has_next": False}
        elif path.startswith("/v1/executions/") and _is_uuid_path(path, "/v1/executions/"):
            payload = self._execution()
        elif path.startswith("/v1/environments/") and _is_uuid_path(path, "/v1/environments/"):
            payload = self._environment()
        else:
            self.send_error(404, "No smoke fixture for this route")
            return
        self._send(payload)

    def do_POST(self) -> None:
        if self.headers.get("Authorization") != "Bearer release-smoke-placeholder":
            self.send_error(401, "Missing smoke authorization")
            return
        if self.path in ("/v1/environments", "/v1/executions"):
            if not self.headers.get("Idempotency-Key"):
                self.send_error(400, "Missing idempotency key")
                return
            self._send(self._environment() if self.path == "/v1/environments" else self._execution())
        elif self.path.startswith("/v1/executions/") and self.path.endswith("/cancel"):
            execution_id = self.path[len("/v1/executions/"):-len("/cancel")].strip("/")
            if not _is_uuid(execution_id):
                self.send_error(404, "No smoke fixture for this route")
                return
            self._send(self._execution())
        else:
            self.send_error(404, "No smoke fixture for this route")


def _is_uuid(value: str) -> bool:
    try:
        uuid.UUID(value)
        return True
    except ValueError:
        return False


def _is_uuid_path(path: str, prefix: str) -> bool:
    return _is_uuid(path.removeprefix(prefix))

    def log_message(self, _format: str, *_args: object) -> None:
        pass


def require_tool_catalog(response: dict) -> None:
    result = response.get("result")
    tools = result.get("tools") if isinstance(result, dict) else None
    if not isinstance(tools, list):
        raise SystemExit("MCP process did not return a tools list")

    by_name = {tool.get("name"): tool for tool in tools if isinstance(tool, dict)}
    missing = EXPECTED_TOOLS - by_name.keys()
    if missing:
        raise SystemExit(f"MCP tool discovery is missing: {', '.join(sorted(missing))}")

    for name, tool in by_name.items():
        for schema_name in ("inputSchema", "outputSchema"):
            label = "input schema" if schema_name == "inputSchema" else "output schema"
            schema = tool.get(schema_name)
            if not isinstance(schema, dict) or schema.get("type") != "object":
                raise SystemExit(f"MCP tool {name} has no valid {label} object")
            if not isinstance(schema.get("properties"), dict):
                raise SystemExit(f"MCP tool {name} {label} has no properties object")


def require_schema_value(value: object, schema: dict, path: str = "result") -> None:
    validator_class = validators.validator_for(schema)
    validator_class.check_schema(schema)
    errors = sorted(validator_class(schema).iter_errors(value), key=lambda error: list(error.path))
    if errors:
        error = errors[0]
        location = ".".join(str(part) for part in error.absolute_path)
        suffix = f" at {path}.{location}" if location else f" at {path}"
        raise SystemExit(f"MCP tool result does not match its JSON Schema{suffix}: {error.message}")


def require_tool_call_result(response: dict, catalog_response: dict, tool_name: str) -> bool:
    result = response.get("result")
    if not isinstance(result, dict):
        raise SystemExit("MCP process did not return a tools/call result")
    if result.get("isError") is True:
        return False
    structured = result.get("structuredContent")
    if not isinstance(structured, dict):
        raise SystemExit("Successful MCP tools/call response has no structured content")
    tools = catalog_response["result"]["tools"]
    tool = next((item for item in tools if item.get("name") == tool_name), None)
    if tool is None:
        raise SystemExit(f"MCP tool catalog is missing {tool_name}")
    output_schema = tool["outputSchema"]
    require_schema_value(structured, output_schema)
    return True


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: mcp-binary-smoke.py PATH_TO_BINARY")
    initialize = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "mpaka-mcp-release-smoke", "version": "1"},
        },
    }
    initialized = {"jsonrpc": "2.0", "method": "notifications/initialized"}
    list_tools = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {},
    }
    smoke_calls = [
        ("execute_python", {"code": "print('ok')", "idempotency_key": "smoke-execution-1"}),
        ("get_execution", {"execution_id": "00000000-0000-0000-0000-000000000000"}),
        ("cancel_execution", {"execution_id": "00000000-0000-0000-0000-000000000000"}),
        ("list_executions", {}),
        ("wait_execution", {"execution_id": "00000000-0000-0000-0000-000000000000", "timeout_ms": SMOKE_WAIT_TIMEOUT_MS, "poll_interval_ms": 100}),
        ("create_environment", {"requirements": ["requests==2.32.3"], "idempotency_key": "smoke-environment-1"}),
        ("get_environment", {"environment_id": "00000000-0000-0000-0000-000000000000"}),
        ("list_environments", {}),
        ("wait_environment", {"environment_id": "00000000-0000-0000-0000-000000000000", "timeout_ms": SMOKE_WAIT_TIMEOUT_MS, "poll_interval_ms": 100}),
        ("get_usage", {"limit": 1}),
        ("get_usage_summary", {"group_by": "day"}),
        ("get_resource_profiles", {}),
        ("get_quota", {}),
    ]
    call_requests = [
        {"jsonrpc": "2.0", "id": index + 3, "method": "tools/call", "params": {"name": name, "arguments": arguments}}
        for index, (name, arguments) in enumerate(smoke_calls)
    ]
    env = os.environ.copy()
    api = ThreadingHTTPServer(("127.0.0.1", 0), SmokeApiHandler)
    api_thread = Thread(target=api.serve_forever, daemon=True)
    api_thread.start()
    env.update({
        "MPAKA_API_KEY": "release-smoke-placeholder",
        "MPAKA_API_BASE_URL": f"http://127.0.0.1:{api.server_port}",
        "MPAKA_API_TIMEOUT_SECONDS": "5",
    })
    try:
        result = subprocess.run(
            [sys.argv[1]],
            input=(
                json.dumps(initialize)
                + "\n"
                + json.dumps(initialized)
                + "\n"
                + json.dumps(list_tools)
                + "\n"
                + "".join(json.dumps(request) + "\n" for request in call_requests)
            ).encode(),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            timeout=30,
            check=False,
        )
    finally:
        api.shutdown()
        api.server_close()
        api_thread.join()
    if result.returncode != 0:
        message = result.stderr.decode(errors="replace")
        raise SystemExit(f"MCP process exited with status {result.returncode}: {message}")
    messages = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    response = next((message for message in messages if message.get("id") == 1), None)
    if response is None or "result" not in response:
        raise SystemExit("MCP process did not return an initialize result")
    if response["result"].get("protocolVersion") != "2025-06-18":
        raise SystemExit("MCP process negotiated an unexpected protocol version")
    tools_response = next((message for message in messages if message.get("id") == 2), None)
    if tools_response is None:
        raise SystemExit("MCP process did not return a tools/list result")
    require_tool_catalog(tools_response)
    successful_calls = 0
    api_errors = 0
    for index, (name, _) in enumerate(smoke_calls, start=3):
        call_response = next((message for message in messages if message.get("id") == index), None)
        if call_response is None:
            raise SystemExit(f"MCP process did not return a tools/call response for {name}")
        if require_tool_call_result(call_response, tools_response, name):
            successful_calls += 1
        else:
            api_errors += 1
            print(f"Smoke tool call failed: {name}: {json.dumps(call_response, sort_keys=True)}", file=sys.stderr)
    if successful_calls != len(smoke_calls):
        failed = len(smoke_calls) - successful_calls
        raise SystemExit(f"MCP smoke requires successful results for all {len(smoke_calls)} tool calls; {failed} failed")
    print(f"MCP stdio initialize, discovery, and tools/call schema smoke passed ({successful_calls} successful calls, {api_errors} API errors)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
