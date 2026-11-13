"""Tests for the MCP stdio client.

The tests run a small JSON-RPC server implemented in Python, so they verify the
client without Docker, Neo4j, or a model.
"""

from __future__ import annotations

import json
import sys
import unittest

from membench.errors import BenchError
from membench.mcpclient import McpStdioClient

SERVER = """
import json
import sys

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    message = json.loads(line)
    method = message.get("method")
    if method == "initialize":
        response = {"protocolVersion": "2024-11-05", "capabilities": {}}
    elif method == "tools/list":
        response = {"tools": [{"name": "muninn_search_preferences", "description": "d"}]}
    elif method == "tools/call":
        response = {"content": [{"type": "text", "text": '[{"content": "Juniper Harbor"}]'}]}
    elif method == "boom":
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": message["id"],
                                     "error": {"code": -32000, "message": "no"}}) + "\\n")
        sys.stdout.flush()
        continue
    else:
        continue
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": response}) + "\\n")
    sys.stdout.flush()
"""


class McpClientTests(unittest.TestCase):
    def test_handshake_and_tool_list(self) -> None:
        with McpStdioClient([sys.executable, "-c", SERVER], timeout=30.0) as client:
            client.initialize()
            tools = client.list_tools()
        self.assertEqual([tool.name for tool in tools], ["muninn_search_preferences"])

    def test_tool_call_returns_the_payload(self) -> None:
        with McpStdioClient([sys.executable, "-c", SERVER], timeout=30.0) as client:
            client.initialize()
            payload = client.call_tool("muninn_search_preferences", {"query": "report"})
        self.assertIn("Juniper Harbor", payload["content"][0]["text"])

    def test_server_error_is_reported(self) -> None:
        with McpStdioClient([sys.executable, "-c", SERVER], timeout=30.0) as client:
            client.initialize()
            with self.assertRaises(BenchError):
                client.request("boom")

    def test_missing_binary_is_reported(self) -> None:
        with self.assertRaises(BenchError):
            McpStdioClient(["/nonexistent/mcp-server"], timeout=5.0).start()


if __name__ == "__main__":
    unittest.main()
