# MCP by profile and transport

## Scope

This unit captures the **runtime tool registry** for three profiles of the
pinned `agent-memory` oracle. It also performs `initialize` and `tools/list`
through stdio and streamable HTTP using the real MCP SDK. It neither implements
Muninn nor calls memory, backend, or LLM tools.

The client HTTP example port is `8001`, but the pinned FastMCP server default
is **8000**. The local runner forwards host `127.0.0.1:18001` to container 8000;
it does not change upstream.

## Observed profiles

| Profile | Backend | Tools | Transports | Limitation |
| --- | --- | ---: | --- | --- |
| `core` | disabled | 6 | stdio + HTTP | Registry; no backend call |
| `extended-no-backend` | disabled | 16 | stdio + HTTP | Conditional tools absent |
| `extended-nams-offline` | NAMS offline | 20 | stdio + HTTP | Real NAMS client construction without I/O |

An additional capture constructs the reference default configuration:
**6 tools**, backend `neo4j`, with **2 driver-creation attempts blocked before
network access**. This characterizes the enabled default without opening Neo4j.

The four tools that appear only with a non-null backend are `create_entities`,
`create_relations`, `update_entity`, and `update_relation`. The remaining
extended-only functions register even when the backend is disabled; registration
does not imply successful invocation.

NAMS uses a loopback HTTP URL and `NAM_AUTH_DISABLED=true`. That makes its
experimental client constructible offline. It does **not** prove authentication,
permissions, server compatibility, persistent storage, or exception parity.

## Schemas and stability

The files under
[the evidence directory](../../.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/) include:

- observed names, descriptions, and `inputSchema` from `list_tools()`;
- SHA-256 of the ordered input schema;
- upstream source and SHA for each tool;
- upstream and local source hashes, dependency lock, and versions;
- client/server protocol version and HTTP session initialization;
- normalized `list_prompts` failure from the pinned oracle.

The [matrix](specs/mcp-baseline.csv) contains 78 rows: 6 + 6 + 16 + 20 tools
and 30 transport-clause rows across the three explicit profiles. These are not
78 separate capabilities. stdio and HTTP produce the same registry within each
profile. The protocol negotiated in all six sessions is **2025-11-25**.

In the two explicit `core` sessions, `list_prompts` returns
`McpError(-32602, "Unknown method: prompts/list")`; the server has no callable
prompt hook. Capabilities omit `prompts`. This does not affect `tools/list`.

## Exact limits

- Runtime imports are verified against `source-integrity.json` and load only
  authorized bytes copied into an isolated temporary tree. Instrumentation
  runs after verification and does not alter the upstream checkout.
- Profile overrides explicitly select core/extended mode and enable/disable
  the backend. NAMS avoids optional dependencies and external services; it does
  not stand in for a real NAMS server.
- A global network guard rejects `socket.connect`, `connect_ex`, and `bind`
  outside loopback. A separate guard blocks Neo4j driver construction before I/O.
- `tools/call`, real authentication, search, graph mutation, and LLMs remain pending.

## Open gates

Live NAMS/Neo4j MCP sessions, positive and negative permission tests, invocation
exception behavior, advanced extended paths, and Muninn equivalence remain
unexecuted. The observed registry is a discoverability contract, not functional
proof.
