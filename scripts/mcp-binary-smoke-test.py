#!/usr/bin/env python3
"""Tests for the packaged MCP binary protocol smoke validator."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
SMOKE = ROOT / "scripts" / "mcp-binary-smoke.py"
EXPECTED_TOOLS = [
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
]


FAKE_SERVER = r'''#!/usr/bin/env python3
import json
import os
import sys

requests = [json.loads(line) for line in sys.stdin if line.strip()]
methods = [request.get("method") for request in requests]
if "tools/list" not in methods or "tools/call" not in methods:
    print("smoke omitted tools/list or tools/call", file=sys.stderr)
    raise SystemExit(2)

names = [
    "execute_python", "get_execution", "cancel_execution", "list_executions",
    "wait_execution", "create_environment", "get_environment",
    "list_environments", "wait_environment", "get_resource_profiles",
    "get_usage", "get_usage_summary", "get_quota",
]
mode = os.environ.get("MCP_SMOKE_TEST_MODE", "valid")
if mode == "missing-tool":
    names.pop()
tools = []
for name in names:
    tool = {"name": name, "inputSchema": {"type": "object", "properties": {}}}
    if mode != "missing-output-schema":
        valid_schema = {"type": "string"}
        if mode == "one-of-overlap":
            valid_schema = {"oneOf": [{"type": "string"}, {"enum": ["yes"]}]}
        tool["outputSchema"] = {"type": "object", "properties": {"valid": valid_schema}, "required": ["valid"]} if name == "get_resource_profiles" else {"type": "object", "properties": {}}
    tools.append(tool)

responses = [
    {
        "jsonrpc": "2.0", "id": 1,
        "result": {"protocolVersion": "2025-06-18", "capabilities": {}, "serverInfo": {"name": "mpaka-mcp", "version": "0.1.0"}},
    },
    {"jsonrpc": "2.0", "id": 2, "result": {"tools": tools}},
]
for request in requests:
    if request.get("method") == "tools/call":
        name = request.get("params", {}).get("name")
        content = {"valid": 42 if mode == "invalid-tool-result" else "yes"} if name == "get_resource_profiles" else {}
        result = {"isError": True, "content": [{"type": "text", "text": "upstream unavailable"}]} if mode == "all-api-errors" else {"structuredContent": content}
        responses.append({"jsonrpc": "2.0", "id": request["id"], "result": result})
for response in responses:
    print(json.dumps(response))
'''


class McpBinarySmokeTests(unittest.TestCase):
    def invoke_smoke(self, mode: str = "valid") -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "fake-mcp-server"
            binary.write_text(FAKE_SERVER, encoding="utf-8")
            binary.chmod(0o755)
            environment = os.environ.copy()
            environment["MCP_SMOKE_TEST_MODE"] = mode
            return subprocess.run(
                [sys.executable, str(SMOKE), str(binary)],
                capture_output=True,
                text=True,
                env=environment,
                check=False,
                timeout=10,
            )

    def test_accepts_all_tools_with_input_and_output_schemas(self) -> None:
        result = self.invoke_smoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("successful calls", result.stdout)

    def test_rejects_missing_tool(self) -> None:
        result = self.invoke_smoke("missing-tool")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("get_quota", result.stderr)

    def test_rejects_missing_output_schema(self) -> None:
        result = self.invoke_smoke("missing-output-schema")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("output schema", result.stderr.lower())

    def test_rejects_tool_call_result_that_does_not_match_output_schema(self) -> None:
        result = self.invoke_smoke("invalid-tool-result")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("JSON Schema", result.stderr)

    def test_enforces_one_of_exclusivity(self) -> None:
        result = self.invoke_smoke("one-of-overlap")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("valid under each", result.stderr)

    def test_rejects_smoke_with_no_successful_tool_calls(self) -> None:
        result = self.invoke_smoke("all-api-errors")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("all 13 tool calls", result.stderr.lower())


if __name__ == "__main__":
    unittest.main()
