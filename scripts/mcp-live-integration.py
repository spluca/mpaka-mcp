#!/usr/bin/env python3
"""Exercise the stdio MCP server against the configured live mPaka API.

Requires MPAKA_API_KEY and a built mpaka-mcp binary. This submits real work.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid


class McpProcess:
    def __init__(self, binary: Path) -> None:
        self.process = subprocess.Popen(
            [str(binary)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=None,
            text=True,
            bufsize=1,
        )
        self.request_id = 0
        self.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "mpaka-live-integration", "version": "1.0.0"},
        })
        self.notify("notifications/initialized")

    def notify(self, method: str) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.process.stdin.flush()

    def request(self, method: str, params: dict | None = None) -> dict:
        assert self.process.stdin is not None and self.process.stdout is not None
        self.request_id += 1
        request_id = self.request_id
        payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self.process.stdin.write(json.dumps(payload) + "\n")
        self.process.stdin.flush()
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError(f"MCP process exited with code {self.process.poll()}")
            response = json.loads(line)
            if response.get("id") == request_id:
                if "error" in response:
                    raise RuntimeError(f"MCP protocol error: {response['error']}")
                return response.get("result", {})

    def call(self, name: str, arguments: dict) -> dict:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            content = result.get("content", [])
            details = "\n".join(item.get("text", "") for item in content)
            raise RuntimeError(f"{name} failed: {details}")
        structured = result.get("structuredContent")
        if isinstance(structured, dict):
            return structured
        for item in result.get("content", []):
            if item.get("type") == "text":
                try:
                    parsed = json.loads(item["text"])
                    if isinstance(parsed, dict):
                        return parsed
                except json.JSONDecodeError:
                    continue
        raise RuntimeError(f"{name} returned no structured object: {result}")

    def close(self) -> None:
        if self.process.stdin is not None:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=5)


def require(condition: bool, message: str, value: dict) -> None:
    if not condition:
        raise RuntimeError(f"{message}: {json.dumps(value, sort_keys=True)}")


def wait_environment(client: McpProcess, environment_id: str) -> dict:
    result = client.call("wait_environment", {
        "environment_id": environment_id,
        "timeout_ms": 30_000,
        "poll_interval_ms": 2_000,
    })
    environment = result.get("environment", {})
    require(not result.get("timed_out"), "Environment wait timed out", result)
    require(environment.get("status") == "ready", "Environment did not become ready", result)
    return environment


def execute_and_wait(client: McpProcess, environment_id: str | None, suffix: str) -> dict:
    marker = f"mpaka-mcp-live-{suffix}"
    arguments = {
        "code": f"print({marker!r})",
        "idempotency_key": f"mpaka-mcp-live-{suffix}",
    }
    if environment_id is not None:
        arguments["environment_id"] = environment_id
    submitted = client.call("execute_python", arguments)
    execution_id = submitted.get("id")
    require(isinstance(execution_id, str) and bool(execution_id), "Missing execution ID", submitted)
    retried = client.call("execute_python", arguments)
    require(retried.get("id") == execution_id, "Idempotent retry returned a different execution ID", retried)
    result = client.call("wait_execution", {
        "execution_id": execution_id,
        "timeout_ms": 30_000,
        "poll_interval_ms": 1_000,
    })
    execution = result.get("execution", {})
    require(not result.get("timed_out"), "Execution wait timed out", result)
    require(execution.get("id") == execution_id, "Execution ID changed during wait", result)
    require(execution.get("status") == "completed", "Execution did not complete", result)
    require(execution.get("exit_code") == 0, "Execution returned a nonzero exit code", result)
    require(marker in execution.get("stdout", ""), "Execution output did not match", result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/mpaka-mcp"))
    parser.add_argument(
        "--execution-only",
        action="store_true",
        help="skip environment creation and verify direct Python execution only",
    )
    parser.add_argument(
        "--fixtures-only",
        action="store_true",
        help="check supplied timeout/failure environment fixtures without creating work",
    )
    parser.add_argument(
        "--cli-environment-id",
        help="optional environment ID created by another client for cross-client verification",
    )
    parser.add_argument(
        "--timeout-environment-id",
        help="optional queued/building test environment used to verify wait timeout handling",
    )
    parser.add_argument(
        "--failed-environment-id",
        help="optional failed test environment used to verify terminal failure handling",
    )
    args = parser.parse_args()
    if not os.environ.get("MPAKA_API_KEY"):
        parser.error("MPAKA_API_KEY must be set in the environment")
    if not args.binary.is_file():
        parser.error(f"MCP binary not found: {args.binary}")
    if args.fixtures_only and not (args.timeout_environment_id or args.failed_environment_id):
        parser.error("--fixtures-only requires a timeout or failed environment ID")
    if args.fixtures_only and (args.execution_only or args.cli_environment_id):
        parser.error("--fixtures-only cannot be combined with execution-only or a CLI environment ID")

    client = McpProcess(args.binary.resolve())
    try:
        suffix = uuid.uuid4().hex
        environment_id = None
        ready = None
        execution = None
        cli_environment = None
        cli_execution = None
        if not args.fixtures_only:
            if not args.execution_only:
                created = client.call("create_environment", {
                    "requirements": ["requests==2.32.3"],
                    "idempotency_key": f"mpaka-mcp-live-environment-{suffix}",
                })
                environment_id = created.get("id")
                require(isinstance(environment_id, str) and bool(environment_id), "Missing environment ID", created)
                ready = wait_environment(client, environment_id)
            execution = execute_and_wait(client, environment_id, suffix)

            if args.cli_environment_id:
                cli_environment = client.call("get_environment", {
                    "environment_id": args.cli_environment_id,
                })
                require(cli_environment.get("id") == args.cli_environment_id, "Client environment ID did not round-trip", cli_environment)
                require(cli_environment.get("status") == "ready", "Client-created environment is not ready", cli_environment)
                cli_execution = execute_and_wait(client, args.cli_environment_id, f"cli-{suffix}")

        timeout_result = None
        if args.timeout_environment_id:
            timeout_result = client.call("wait_environment", {
                "environment_id": args.timeout_environment_id,
                "timeout_ms": 100,
                "poll_interval_ms": 100,
            })
            require(timeout_result.get("timed_out") is True,
                    "Wait did not report the expected timeout", timeout_result)

        failed_environment = None
        if args.failed_environment_id:
            failed_environment = client.call("wait_environment", {
                "environment_id": args.failed_environment_id,
                "timeout_ms": 30_000,
                "poll_interval_ms": 1_000,
            })
            failed = failed_environment.get("environment", {})
            require(failed_environment.get("timed_out") is False and failed.get("status") == "failed",
                    "Environment did not return a terminal failed state", failed_environment)

        summary = {
            "mcp_environment": None if ready is None else {"id": environment_id, "status": ready["status"]},
            "mcp_execution": None if execution is None else {
                "id": execution["execution"]["id"],
                "status": execution["execution"]["status"],
                "stdout": execution["execution"]["stdout"],
            },
            "cli_environment": None if cli_environment is None else {
                "id": cli_environment["id"], "status": cli_environment["status"],
            },
            "cli_environment_execution": None if cli_execution is None else {
                "id": cli_execution["execution"]["id"],
                "status": cli_execution["execution"]["status"],
                "stdout": cli_execution["execution"]["stdout"],
            },
            "timeout_case": None if timeout_result is None else {
                "id": timeout_result["environment"]["id"],
                "status": timeout_result["environment"]["status"],
                "timed_out": timeout_result["timed_out"],
            },
            "failed_environment_case": None if failed_environment is None else {
                "id": failed_environment["environment"]["id"],
                "status": failed_environment["environment"]["status"],
                "timed_out": failed_environment["timed_out"],
            },
        }
        print(json.dumps(summary, indent=2, sort_keys=True))
        return 0
    finally:
        client.close()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, json.JSONDecodeError) as error:
        print(f"Live MCP integration failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
