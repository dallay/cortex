"""Evidence writers.

Every run writes a manifest, a sanitized event trace, and a human readable
report into an evidence directory. The directory is the unit of review.
"""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Mapping, Sequence

from .sanitize import sanitize


def utc_stamp() -> str:
    """Return a compact UTC timestamp for run identifiers."""

    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def write_json(path: Path, payload: Mapping[str, Any]) -> Path:
    """Write a sanitized JSON document."""

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(sanitize(dict(payload)), indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    return path


def write_text(path: Path, text: str) -> Path:
    """Write a text document."""

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def render_report(manifest: Mapping[str, Any], results: Sequence[Mapping[str, Any]]) -> str:
    """Render the human readable run report."""

    lines: list[str] = []
    lines.append(f"# Run report — {manifest.get('run_id', 'unknown')}")
    lines.append("")
    lines.append("## Scope")
    lines.append("")
    lines.append(
        "This report describes one synthetic scenario executed with one model. "
        "It is not a memory quality score, a parity percentage, or evidence of "
        "global parity with any other memory system."
    )
    lines.append("")
    lines.append(f"- Scenario: `{manifest.get('scenario_version', 'unknown')}`")
    lines.append(f"- Model: `{manifest.get('model', 'unknown')}`")
    lines.append(f"- Repetitions per condition: {manifest.get('repetitions', 0)}")
    lines.append(f"- Runner image: `{manifest.get('runner_image', 'unknown')}`")
    lines.append(f"- OpenCode image: `{manifest.get('opencode_image', 'unknown')}`")
    lines.append("")
    lines.append("## Preflight")
    lines.append("")
    lines.append("| Check | State | Detail |")
    lines.append("| --- | --- | --- |")
    for check in manifest.get("preflight", []) or []:
        lines.append(
            f"| {check.get('name')} | {check.get('state')} | {check.get('detail')} |"
        )
    lines.append("")
    lines.append("## Results")
    lines.append("")
    lines.append("| Case | Mode | Condition | Run | MCP called | Evidence | Answer | Diagnosis |")
    lines.append("| --- | --- | --- | --- | --- | --- | --- | --- |")
    for result in results:
        lines.append(
            "| {case} | {mode} | {condition} | {run} | {mcp} | {evidence} | {answer} | {diagnosis} |".format(
                case=result.get("case_id"),
                mode=result.get("mode"),
                condition=result.get("condition"),
                run=result.get("repetition"),
                mcp="yes" if result.get("mcp_invoked") else "no",
                evidence="yes" if result.get("evidence_retrieved") else "no",
                answer="yes" if result.get("answer_correct") else "no",
                diagnosis=result.get("diagnosis"),
            )
        )
    lines.append("")
    return "\n".join(lines)
