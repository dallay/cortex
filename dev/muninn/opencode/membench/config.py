"""Rendering of the immutable OpenCode benchmark configuration.

The template lives in the repository. Each run renders a derived copy with the
owner scope of the condition and the MCP command for that scope. The rendered
copy is hashed and stored with the evidence, so a run can be replayed exactly.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from pathlib import Path

MCP_SERVER_NAME = "muninn"
MUNINN_BINARY = "/usr/local/bin/muninn"

# The Muninn MCP server only speaks the 2026-07-28 revision of the
# protocol; it rejects the legacy handshake.
MCP_PROTOCOL = "2026-07-28"

AGENT_NAME = "evaluation"

AGENT_SYSTEM_PROMPT = """You are a memory evaluation agent.

Answer the question using only the tools you are allowed to call.

Rules:
- Use the Muninn memory tools to look up stored information before answering.
- Never invent identifiers, titles, columns, or values that a tool did not return.
- If the tools return nothing relevant, say that the information is not available.
- Answer with the requested values only. Do not describe your process.
"""

AGENT_FRONT_MATTER = """---
description: Least-privilege agent for Muninn memory evaluation
mode: primary
steps: 12
permissions:
  - action: "*"
    resource: "*"
    effect: deny
  - action: "muninn_*"
    resource: "*"
    effect: allow
---
"""


@dataclass(frozen=True)
class BenchmarkConfig:
    """A rendered OpenCode configuration for one run."""

    model: str
    owner_scope: str
    mcp_enabled: bool
    text: str
    global_text: str
    agent_text: str

    def digest(self) -> str:
        """Return a digest that covers every file of the configuration."""

        return hashlib.sha256(
            (self.text + self.global_text + self.agent_text).encode("utf-8")
        ).hexdigest()

    def agent_digest(self) -> str:
        return hashlib.sha256(self.agent_text.encode("utf-8")).hexdigest()


def render_config(model: str, owner_scope: str, mcp_enabled: bool = True) -> BenchmarkConfig:
    """Render the OpenCode project configuration for one run."""

    text = f"""{{
  "$schema": "https://opencode.ai/config.json",
  "model": "{model}",
  "share": "disabled",
  "autoupdate": false,
  "permission": {{
    "edit": "deny",
    "bash": "deny",
    "webfetch": "deny"
  }}
}}
"""
    agent_text = AGENT_FRONT_MATTER + "\n" + AGENT_SYSTEM_PROMPT
    return BenchmarkConfig(
        model=model,
        owner_scope=owner_scope,
        mcp_enabled=mcp_enabled,
        text=text,
        global_text=render_global_config(owner_scope, mcp_enabled=mcp_enabled),
        agent_text=agent_text,
    )


def render_global_config(owner_scope: str, mcp_enabled: bool = True) -> str:
    """Render the global configuration that declares the MCP server.

    OpenCode 2.2 reads the MCP servers from the global configuration only. A
    project configuration with an ``mcp`` block is loaded for the model and the
    permissions, but its servers are never registered.
    """

    if not mcp_enabled:
        return "{}\n"
    # The global configuration carries no "$schema" key: OpenCode 2.2 drops the
    # whole global configuration when the schema reference is present.
    return f"""{{
  "mcp": {{
    "servers": {{
      "{MCP_SERVER_NAME}": {{
        "type": "local",
        "command": ["{MUNINN_BINARY}", "mcp", "--owner-scope", "{owner_scope}"],
        "protocol": "{MCP_PROTOCOL}",
        "codemode": false
      }}
    }}
  }}
}}
"""


def write_config(config: BenchmarkConfig, target: Path, owner_scope: str) -> Path:
    """Write the rendered configuration into ``target``.

    The project configuration holds the model and the permissions, the agent
    definition lives in ``.opencode/agents`` and the global configuration holds
    the MCP server.
    """

    opencode_dir = target / ".opencode"
    agents_dir = opencode_dir / "agents"
    agents_dir.mkdir(parents=True, exist_ok=True)
    (opencode_dir / "opencode.jsonc").write_text(config.text, encoding="utf-8")
    (agents_dir / f"{AGENT_NAME}.md").write_text(config.agent_text, encoding="utf-8")
    global_dir = target / "global"
    global_dir.mkdir(parents=True, exist_ok=True)
    (global_dir / "opencode.json").write_text(
        config.global_text, encoding="utf-8"
    )
    return opencode_dir


def exposed_tool_name(tool: str) -> str:
    """Return the name that OpenCode gives to a tool of the MCP server.

    OpenCode joins the name of the server and the name of the tool, so the
    model only knows ``muninn_muninn_search_messages``.
    """

    return f"{MCP_SERVER_NAME}_{tool}"


def forced_tool_prompt(prompt: str, tool: str, arguments: dict[str, object]) -> str:
    """Prompt used by the forced tool use mode (B).

    The arguments are rendered as a JSON object, because the model has to
    reproduce them exactly. A quoted string would make the model send a
    string where the tool schema demands an integer, and the call would be
    rejected before it reaches the server.
    """

    rendered = json.dumps(dict(arguments), ensure_ascii=False)
    return (
        f"{prompt}\n\n"
        "You must retrieve the stored information before answering. Call the "
        f"'{tool}' tool exactly once with these arguments: {rendered}. "
        "Then answer the question using the result of that call."
    )
