"""Rendering of the immutable OpenCode benchmark configuration.

The template lives in the repository. Each run renders a derived copy with the
owner scope of the condition and the MCP command for that scope. The rendered
copy is hashed and stored with the evidence, so a run can be replayed exactly.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from pathlib import Path

MCP_SERVER_NAME = "muninn"
MUNINN_BINARY = "/usr/local/bin/muninn"

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
    agent_text: str

    def digest(self) -> str:
        return hashlib.sha256(self.text.encode("utf-8")).hexdigest()

    def agent_digest(self) -> str:
        return hashlib.sha256(self.agent_text.encode("utf-8")).hexdigest()


def render_config(model: str, owner_scope: str, mcp_enabled: bool = True) -> BenchmarkConfig:
    """Render the OpenCode project configuration for one run."""

    mcp_block = ""
    if mcp_enabled:
        mcp_block = f"""
  "mcp": {{
    "servers": {{
      "{MCP_SERVER_NAME}": {{
        "type": "local",
        "command": ["{MUNINN_BINARY}", "mcp", "--owner-scope", "{owner_scope}"],
        "enabled": true
      }}
    }}
  }},
"""
    text = f"""{{
  "$schema": "https://opencode.ai/config.json",
  "model": "{model}",
  "share": "disabled",
  "autoupdate": false,
{mcp_block}  "permission": {{
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
        agent_text=agent_text,
    )


def write_config(config: BenchmarkConfig, target: Path) -> Path:
    """Write the rendered configuration into ``target/.opencode``."""

    opencode_dir = target / ".opencode"
    agents_dir = opencode_dir / "agents"
    agents_dir.mkdir(parents=True, exist_ok=True)
    (opencode_dir / "opencode.jsonc").write_text(config.text, encoding="utf-8")
    (agents_dir / f"{AGENT_NAME}.md").write_text(config.agent_text, encoding="utf-8")
    return opencode_dir


def forced_tool_prompt(prompt: str, tool: str, arguments: dict[str, object]) -> str:
    """Prompt used by the forced tool use mode (B)."""

    rendered = ", ".join(f'"{key}": "{value}"' for key, value in arguments.items())
    return (
        f"{prompt}\n\n"
        "You must retrieve the stored information before answering. Call the "
        f"'{tool}' tool exactly once with these arguments: {{{rendered}}}. "
        "Then answer the question using the result of that call."
    )
