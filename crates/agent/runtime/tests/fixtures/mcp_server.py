"""Deterministic stdio MCP fixture; no third party packages."""
import json
import os
import sys
import time

marker = os.environ["MARKER"]
with open(marker, "a", encoding="utf-8") as output:
    output.write("started\n")
for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method = request["method"]
    if method == "initialize":
        result = {"protocolVersion": request["params"]["protocolVersion"],
                  "capabilities": {"tools": {}},
                  "serverInfo": {"name": "fixture", "version": "1"}}
    elif method == "tools/list":
        result = {"tools": [{"name": "echo", "description": "Echo fixture",
                             "inputSchema": {"type": "object", "properties": {}}}]}
    elif method == "tools/call":
        with open(marker, "a", encoding="utf-8") as output:
            output.write("called\n")
        mode = request["params"].get("arguments", {}).get("mode")
        if mode == "crash":
            sys.exit(1)
        if mode == "timeout":
            time.sleep(30)
        if mode == "protocol_error":
            print(json.dumps({"jsonrpc": "2.0", "id": request["id"],
                              "error": {"code": -32602, "message": "invalid params"}}), flush=True)
            continue
        result = {"content": [{"type": "text", "text": "fixture result"}], "isError": False}
    elif method == "ping":
        result = {}
    else:
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
