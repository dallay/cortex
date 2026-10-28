"""Small, dependency-free helpers for validating Server-Sent Events."""

import json


def parse_sse_frames(body: str) -> list[tuple[str, dict[str, object]]]:
    """Parse each SSE event header together with its own JSON data payload."""
    frames = []
    for frame_text in body.split("\n\n"):
        if not frame_text.strip():
            continue
        event_name = None
        data_lines = []
        for line in frame_text.splitlines():
            if line.startswith("event:"):
                event_name = line.removeprefix("event:").strip()
            elif line.startswith("data:"):
                data_lines.append(line.removeprefix("data:").lstrip())
        if not data_lines:
            continue
        if event_name is None:
            raise AssertionError(f"SSE frame has data but no event header: {frame_text!r}")
        data = json.loads("\n".join(data_lines))
        if not isinstance(data, dict) or not isinstance(data.get("type"), str):
            raise AssertionError(f"SSE frame data must contain a string type: {data!r}")
        if event_name != data["type"]:
            raise AssertionError(
                f"SSE event header {event_name!r} does not match its data type {data['type']!r}"
            )
        frames.append((event_name, data))
    return frames
