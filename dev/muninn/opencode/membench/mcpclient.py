"""Minimal MCP client over stdio.

The harness uses this client for two things: the preflight handshake against
the Muninn MCP server, and the direct retrieval mode (A), where the harness
itself performs the query instead of an agent.
"""

from __future__ import annotations

import json
import queue
import subprocess
import threading
import time
from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from .errors import BenchError

# The Muninn MCP server speaks this revision of the protocol.
PROTOCOL_VERSION = "2026-07-28"
CLIENT_NAME = "membench"
CLIENT_VERSION = "1.0.0"


@dataclass
class McpTool:
    """One tool advertised by the MCP server."""

    name: str
    description: str = ""
    schema: Mapping[str, Any] | None = None


class McpStdioClient:
    """A newline-delimited JSON-RPC client for a local MCP server."""

    def __init__(self, command: Sequence[str], timeout: float = 60.0, env: Mapping[str, str] | None = None):
        self.command = list(command)
        self.timeout = timeout
        self.env = dict(env or {})
        self._process: subprocess.Popen[str] | None = None
        self._reader: threading.Thread | None = None
        self._lines: queue.Queue[str | None] = queue.Queue()
        self._next_id = 0

    def __enter__(self) -> "McpStdioClient":
        self.start()
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def start(self) -> None:
        try:
            self._process = subprocess.Popen(  # noqa: S603 - command is provided by the operator
                self.command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                bufsize=1,
                env={**dict(_base_env()), **self.env} if self.env else None,
            )
        except OSError as error:
            raise BenchError(f"could not start MCP server: {error}") from error
        self._reader = threading.Thread(target=self._pump, daemon=True)
        self._reader.start()

    def _pump(self) -> None:
        assert self._process is not None and self._process.stdout is not None
        for line in self._process.stdout:
            self._lines.put(line)
        self._lines.put(None)

    def close(self) -> None:
        if self._process is None:
            return
        try:
            if self._process.stdin:
                self._process.stdin.close()
            self._process.terminate()
            self._process.wait(timeout=10)
        except Exception:  # noqa: BLE001 - teardown must not mask a result
            self._process.kill()
        finally:
            for stream in (self._process.stdout, self._process.stderr):
                if stream is not None:
                    try:
                        stream.close()
                    except Exception:  # noqa: BLE001
                        pass
            self._process = None

    def _read_message(self) -> dict[str, Any]:
        deadline = time.monotonic() + self.timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise BenchError("timed out waiting for an MCP response")
            try:
                line = self._lines.get(timeout=remaining)
            except queue.Empty as error:
                raise BenchError("timed out waiting for an MCP response") from error
            if line is None:
                raise BenchError("the MCP server closed the connection")
            stripped = line.strip()
            if not stripped:
                continue
            try:
                message = json.loads(stripped)
            except json.JSONDecodeError:
                continue
            if isinstance(message, dict):
                return message

    def request(self, method: str, params: Mapping[str, Any] | None = None) -> Any:
        if self._process is None:
            raise BenchError("the MCP client is not started")
        self._next_id += 1
        request_id = self._next_id
        payload = {"jsonrpc": "2.0", "id": request_id, "method": method}
        request_params = dict(params) if params is not None else {}
        # The server requires the protocol metadata on every request that
        # follows the initialization.
        meta = dict(request_params.get("_meta") or {})
        meta.setdefault("io.modelcontextprotocol/protocolVersion", PROTOCOL_VERSION)
        meta.setdefault("io.modelcontextprotocol/clientCapabilities", {})
        meta.setdefault(
            "io.modelcontextprotocol/clientInfo",
            {"name": CLIENT_NAME, "version": CLIENT_VERSION},
        )
        request_params["_meta"] = meta
        payload["params"] = request_params
        assert self._process.stdin is not None
        self._process.stdin.write(json.dumps(payload) + "\n")
        self._process.stdin.flush()
        while True:
            message = self._read_message()
            if message.get("id") != request_id:
                continue
            if "error" in message:
                error = message["error"]
                message_text = error.get("message") if isinstance(error, Mapping) else str(error)
                raise BenchError(f"MCP error for '{method}': {message_text}")
            return message.get("result")

    def notify(self, method: str, params: Mapping[str, Any] | None = None) -> None:
        if self._process is None:
            raise BenchError("the MCP client is not started")
        payload: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
        notification_params = dict(params) if params is not None else {}
        meta = dict(notification_params.get("_meta") or {})
        meta.setdefault("io.modelcontextprotocol/protocolVersion", PROTOCOL_VERSION)
        meta.setdefault("io.modelcontextprotocol/clientCapabilities", {})
        notification_params["_meta"] = meta
        payload["params"] = notification_params
        assert self._process.stdin is not None
        self._process.stdin.write(json.dumps(payload) + "\n")
        self._process.stdin.flush()

    def initialize(self) -> dict[str, Any]:
        result = self.request(
            "initialize",
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": CLIENT_NAME, "version": CLIENT_VERSION},
            },
        )
        self.notify("notifications/initialized")
        return result if isinstance(result, dict) else {}

    def list_tools(self) -> list[McpTool]:
        result = self.request("tools/list")
        tools: list[McpTool] = []
        if isinstance(result, Mapping):
            for item in result.get("tools") or []:
                if not isinstance(item, Mapping):
                    continue
                schema = item.get("inputSchema")
                tools.append(
                    McpTool(
                        name=str(item.get("name") or ""),
                        description=str(item.get("description") or ""),
                        schema=schema if isinstance(schema, Mapping) else None,
                    )
                )
        return tools

    def call_tool(self, name: str, arguments: Mapping[str, Any]) -> Any:
        result = self.request("tools/call", {"name": name, "arguments": dict(arguments)})
        return result


def _base_env() -> dict[str, str]:
    import os

    return dict(os.environ)
