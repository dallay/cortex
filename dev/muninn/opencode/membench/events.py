"""Parser for the OpenCode non-interactive JSON event stream.

``opencode run --format json`` writes one JSON object per line. Text parts
arrive incrementally, tool calls arrive once per call, and token and cost
figures arrive with the step summary. The parser is tolerant: an unknown event
type is recorded and ignored instead of aborting a run.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Iterable, Iterator

from .sanitize import sanitize

MCP_TOOL_PREFIX = "muninn_"


@dataclass
class ToolCall:
    """One tool invocation observed in the event stream."""

    tool: str
    status: str
    input: Any
    output: Any
    error: str | None = None

    @property
    def is_muninn(self) -> bool:
        return self.tool.startswith(MCP_TOOL_PREFIX)

    def to_manifest(self) -> dict[str, Any]:
        return {
            "tool": self.tool,
            "status": self.status,
            "is_muninn": self.is_muninn,
            "input": sanitize(self.input),
            "output": sanitize(self.output),
            "error": self.error,
        }


@dataclass
class RunTrace:
    """The observable result of one ``opencode run`` invocation."""

    session_id: str = ""
    text: str = ""
    tool_calls: list[ToolCall] = field(default_factory=list)
    errors: list[str] = field(default_factory=list)
    input_tokens: int = 0
    output_tokens: int = 0
    cost: float = 0.0
    steps: int = 0
    events: int = 0
    unknown_events: list[str] = field(default_factory=list)
    noise: list[str] = field(default_factory=list)

    @property
    def muninn_tool_calls(self) -> list[ToolCall]:
        return [call for call in self.tool_calls if call.is_muninn]

    @property
    def failed(self) -> bool:
        return bool(self.errors)

    def to_manifest(self) -> dict[str, Any]:
        return {
            "session_id": self.session_id,
            "text_length": len(self.text),
            "tool_calls": [call.to_manifest() for call in self.tool_calls],
            "errors": list(self.errors),
            "input_tokens": self.input_tokens,
            "output_tokens": self.output_tokens,
            "cost": self.cost,
            "steps": self.steps,
            "events": self.events,
            "noise": list(self.noise),
        }


def _iter_lines(payload: str) -> Iterator[str]:
    for line in payload.splitlines():
        stripped = line.strip()
        if stripped:
            yield stripped


def _tokens(value: Any) -> tuple[int, int]:
    if not isinstance(value, dict):
        return 0, 0
    prompt = value.get("input") or value.get("prompt") or value.get("cache_read") or 0
    completion = value.get("output") or value.get("completion") or 0
    return int(prompt or 0), int(completion or 0)


def _error_message(error: Any) -> str:
    """Extract a readable message from an error event."""

    if isinstance(error, dict):
        data = error.get("data")
        if isinstance(data, dict):
            message = data.get("message")
            if isinstance(message, str) and message.strip():
                return message
        message = error.get("message")
        if isinstance(message, str) and message.strip():
            return message
        name = error.get("name")
        if isinstance(name, str) and name.strip():
            return name
    if isinstance(error, str) and error.strip():
        return error
    return "unknown error"


def parse_events(payload: str) -> RunTrace:
    """Parse an OpenCode JSON event stream into a :class:`RunTrace`."""

    trace = RunTrace()
    text_parts: dict[str, str] = {}
    for line in _iter_lines(payload):
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            # Docker and the CLI can print progress lines next to the event
            # stream. They are recorded as noise and never counted as a run
            # failure.
            trace.noise.append(line[:200])
            continue
        if not isinstance(event, dict):
            trace.noise.append(line[:200])
            continue
        trace.events += 1
        session_id = event.get("sessionID")
        if isinstance(session_id, str) and session_id and not trace.session_id:
            trace.session_id = session_id
        kind = str(event.get("type") or "")
        # The CLI writes hyphenated event names; other builds of the same
        # stream use underscores. Both are accepted.
        normalized = kind.replace("-", "_")
        part = event.get("part") if isinstance(event.get("part"), dict) else {}
        if normalized == "text":
            identifier = str(part.get("id") or part.get("messageID") or "text")
            text_parts[identifier] = text_parts.get(identifier, "") + str(part.get("text") or "")
        elif normalized == "reasoning":
            continue
        elif normalized in {"tool_use", "tool"}:
            state = part.get("state") if isinstance(part.get("state"), dict) else {}
            trace.tool_calls.append(
                ToolCall(
                    tool=str(part.get("tool") or ""),
                    status=str(state.get("status") or "unknown"),
                    input=state.get("input"),
                    output=state.get("output"),
                    error=state.get("error") if isinstance(state.get("error"), str) else None,
                )
            )
        elif normalized == "step_start":
            trace.steps += 1
        elif normalized == "step_finish":
            prompt_tokens, completion_tokens = _tokens(part.get("tokens"))
            trace.input_tokens += prompt_tokens
            trace.output_tokens += completion_tokens
            cost = part.get("cost")
            if isinstance(cost, (int, float)):
                trace.cost += float(cost)
        elif normalized == "error":
            trace.errors.append(_error_message(event.get("error")))
        elif normalized in {"retry", "background_wait"}:
            continue
        elif isinstance(kind, str):
            trace.unknown_events.append(kind)
    trace.text = "\n".join(part for part in text_parts.values() if part)
    return trace


def parse_event_lines(lines: Iterable[str]) -> RunTrace:
    """Parse an iterable of event lines."""

    return parse_events("\n".join(lines))
