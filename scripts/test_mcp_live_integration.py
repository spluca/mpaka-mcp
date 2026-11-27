"""Unit tests for the opt-in live integration runner modes."""

import contextlib
import io
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("mcp-live-integration.py")
SPEC = importlib.util.spec_from_file_location("mcp_live_integration", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
live = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(live)


class FakeMcpProcess:
    instance = None

    def __init__(self, _binary: Path) -> None:
        self.calls: list[tuple[str, dict]] = []
        self.instance = self
        FakeMcpProcess.instance = self

    def call(self, name: str, arguments: dict) -> dict:
        self.calls.append((name, arguments))
        if name == "execute_python":
            self.code = arguments["code"]
            return {"id": "execution-1"}
        if name == "wait_execution":
            return {
                "timed_out": False,
                "execution": {
                    "id": "execution-1", "status": "completed", "exit_code": 0,
                    "stdout": self.code.split("'", 2)[1],
                },
            }
        if name == "wait_environment":
            environment_id = arguments["environment_id"]
            if environment_id == "timeout-1":
                return {
                    "timed_out": True,
                    "environment": {"id": environment_id, "status": "building"},
                }
            if environment_id == "failed-1":
                return {
                    "timed_out": False,
                    "environment": {"id": environment_id, "status": "failed"},
                }
        raise AssertionError(f"unexpected MCP tool call: {name}")

    def close(self) -> None:
        pass


class LiveIntegrationTests(unittest.TestCase):
    def test_execution_only_skips_environment_creation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "mpaka-mcp"
            binary.touch()
            with (
                patch.object(sys, "argv", ["mcp-live-integration.py", "--binary", str(binary), "--execution-only"]),
                patch.dict(os.environ, {"MPAKA_API_KEY": "test-key"}),
                patch.object(live, "McpProcess", FakeMcpProcess),
                contextlib.redirect_stdout(io.StringIO()) as output,
            ):
                result = live.main()

        self.assertEqual(result, 0)
        self.assertNotIn("create_environment", [name for name, _ in FakeMcpProcess.instance.calls])
        self.assertIn('"status": "completed"', output.getvalue())
        self.assertIn('"stdout": "mpaka-mcp-live-', output.getvalue())

    def test_fixtures_only_checks_timeout_and_failed_environment(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "mpaka-mcp"
            binary.touch()
            with (
                patch.object(sys, "argv", [
                    "mcp-live-integration.py", "--binary", str(binary), "--fixtures-only",
                    "--timeout-environment-id", "timeout-1",
                    "--failed-environment-id", "failed-1",
                ]),
                patch.dict(os.environ, {"MPAKA_API_KEY": "test-key"}),
                patch.object(live, "McpProcess", FakeMcpProcess),
                contextlib.redirect_stdout(io.StringIO()) as output,
            ):
                result = live.main()

        names = [name for name, _ in FakeMcpProcess.instance.calls]
        self.assertEqual(result, 0)
        self.assertEqual(names, ["wait_environment", "wait_environment"])
        self.assertIn('"status": "building"', output.getvalue())
        self.assertIn('"status": "failed"', output.getvalue())


if __name__ == "__main__":
    unittest.main()
