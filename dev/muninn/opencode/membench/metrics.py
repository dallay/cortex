"""Assertions, per-run results, and cross-mode diagnosis.

The harness never lets the agent grade itself. Every verdict comes from
observable evidence: which tools were called, what they returned, and what the
final answer contained.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Iterable, Mapping, Sequence

from .events import RunTrace
from .scenario import Case

MODE_DIRECT = "A"
MODE_FORCED = "B"
MODE_AUTONOMOUS = "C"
MODE_ORACLE = "D"

CONDITION_EMPTY = "C0"
CONDITION_MEMORY = "C1"
CONDITION_ORACLE = "C2"

MODES = (MODE_DIRECT, MODE_FORCED, MODE_AUTONOMOUS, MODE_ORACLE)

DIAGNOSIS_OK = "OK"
DIAGNOSIS_RETRIEVAL = "RETRIEVAL_FAILURE"
DIAGNOSIS_TOOL_USE = "TOOL_USE_FAILURE"
DIAGNOSIS_AUTONOMY = "AUTONOMY_FAILURE"
DIAGNOSIS_UTILIZATION = "MODEL_UTILIZATION_FAILURE"
DIAGNOSIS_MODEL = "MODEL_ANSWER_FAILURE"
DIAGNOSIS_INCOMPLETE = "INCOMPLETE_EVIDENCE"

ABSTENTION_MARKERS = (
    "unknown",
    "not found",
    "no record",
    "not available",
    "cannot find",
    "could not find",
    "no information",
    "not stored",
)


@dataclass
class RunResult:
    """The verdict for one run of one case in one mode and condition."""

    case_id: str
    mode: str
    condition: str
    repetition: int
    mcp_invoked: bool = False
    evidence_retrieved: bool = False
    answer_correct: bool = False
    abstained: bool = False
    missing: tuple[str, ...] = ()
    leaked: tuple[str, ...] = ()
    failure_code: str = ""
    diagnosis: str = DIAGNOSIS_OK
    tokens: int = 0
    cost: float = 0.0
    latency_ms: int = 0
    notes: list[str] = field(default_factory=list)

    @property
    def passed(self) -> bool:
        return self.answer_correct and not self.leaked

    def to_manifest(self) -> dict[str, Any]:
        return {
            "case_id": self.case_id,
            "mode": self.mode,
            "condition": self.condition,
            "repetition": self.repetition,
            "passed": self.passed,
            "mcp_invoked": self.mcp_invoked,
            "evidence_retrieved": self.evidence_retrieved,
            "answer_correct": self.answer_correct,
            "abstained": self.abstained,
            "missing": list(self.missing),
            "leaked": list(self.leaked),
            "failure_code": self.failure_code,
            "diagnosis": self.diagnosis,
            "tokens": self.tokens,
            "cost": self.cost,
            "latency_ms": self.latency_ms,
            "notes": list(self.notes),
        }


def _contains(haystack: str, needle: str) -> bool:
    return needle.strip().lower() in haystack.lower()


def extract_hits(output: Any) -> list[Any]:
    """Extract the record list from a retrieval tool output.

    The MCP tools return a JSON array of hits. The payload can arrive as a
    JSON string or as an already decoded structure.
    """

    if output is None:
        return []
    if isinstance(output, str):
        text = output.strip()
        if not text:
            return []
        try:
            output = json.loads(text)
        except json.JSONDecodeError:
            return []
    if isinstance(output, list):
        return output
    if isinstance(output, Mapping):
        for key in ("result", "hits", "records", "content", "data"):
            value = output.get(key)
            if isinstance(value, list):
                return value
            if isinstance(value, str):
                try:
                    parsed = json.loads(value)
                except json.JSONDecodeError:
                    continue
                if isinstance(parsed, list):
                    return parsed
    return []


def _hit_texts(hits: Iterable[Any]) -> str:
    parts: list[str] = []
    for hit in hits:
        if isinstance(hit, Mapping):
            for key in ("content", "text", "preference", "value", "title"):
                value = hit.get(key)
                if isinstance(value, str):
                    parts.append(value)
        elif isinstance(hit, str):
            parts.append(hit)
    return "\n".join(parts)


def _answer_checks(case: Case, answer: str) -> tuple[tuple[str, ...], tuple[str, ...], bool]:
    missing = tuple(item for item in case.expected_contains if not _contains(answer, item))
    leaked = tuple(item for item in case.expected_absent if _contains(answer, item))
    lowered = answer.lower()
    abstained = any(marker in lowered for marker in ABSTENTION_MARKERS)
    return missing, leaked, abstained


def evaluate_agent_run(
    case: Case,
    mode: str,
    condition: str,
    repetition: int,
    trace: RunTrace,
    latency_ms: int = 0,
) -> RunResult:
    """Evaluate an agent run (modes B, C, D) against the case expectations."""

    result = RunResult(
        case_id=case.id,
        mode=mode,
        condition=condition,
        repetition=repetition,
        latency_ms=latency_ms,
        tokens=trace.input_tokens + trace.output_tokens,
        cost=trace.cost,
    )
    muninn_calls = [call for call in trace.muninn_tool_calls if call.status == "completed"]
    result.mcp_invoked = bool(muninn_calls)
    hits: list[Any] = []
    for call in muninn_calls:
        hits.extend(extract_hits(call.output))
    result.evidence_retrieved = bool(hits)

    missing, leaked, abstained = _answer_checks(case, trace.text)
    result.missing = missing
    result.leaked = leaked
    result.abstained = abstained
    if case.abstain:
        result.answer_correct = not leaked and abstained
    else:
        result.answer_correct = not missing and not leaked

    if trace.failed:
        result.failure_code = "RUN_ERROR"
        result.diagnosis = DIAGNOSIS_MODEL
    elif mode == MODE_ORACLE:
        result.diagnosis = DIAGNOSIS_OK if result.answer_correct else DIAGNOSIS_MODEL
    elif not result.mcp_invoked:
        result.failure_code = "MCP_NOT_INVOKED"
        # A forced run that skips the tool is a tool use failure; an
        # autonomous run that skips it is a decision failure.
        result.diagnosis = (
            DIAGNOSIS_TOOL_USE if mode == MODE_FORCED else DIAGNOSIS_AUTONOMY
        )
    elif not result.evidence_retrieved:
        result.failure_code = "EVIDENCE_NOT_RETRIEVED"
        result.diagnosis = DIAGNOSIS_RETRIEVAL
    elif not result.answer_correct:
        result.failure_code = "ANSWER_INCORRECT"
        result.diagnosis = DIAGNOSIS_UTILIZATION
    else:
        result.diagnosis = DIAGNOSIS_OK
    if result.passed:
        result.failure_code = ""
    return result


def evaluate_direct_retrieval(
    case: Case,
    condition: str,
    repetition: int,
    hits: Sequence[Any],
    latency_ms: int = 0,
    error: str | None = None,
) -> RunResult:
    """Evaluate a direct MCP retrieval (mode A) against the case expectations."""

    result = RunResult(
        case_id=case.id,
        mode=MODE_DIRECT,
        condition=condition,
        repetition=repetition,
        latency_ms=latency_ms,
    )
    result.mcp_invoked = error is None
    result.evidence_retrieved = bool(hits)
    evidence = _hit_texts(hits)
    missing = tuple(item for item in case.expected_contains if not _contains(evidence, item))
    leaked = tuple(item for item in case.expected_absent if _contains(evidence, item))
    result.missing = missing
    result.leaked = leaked
    if case.abstain:
        result.answer_correct = not leaked
        result.abstained = not leaked
    else:
        result.answer_correct = not missing and not leaked
    if error:
        result.failure_code = "MCP_ERROR"
        result.diagnosis = DIAGNOSIS_RETRIEVAL
    elif not result.evidence_retrieved:
        result.failure_code = "EVIDENCE_NOT_RETRIEVED"
        result.diagnosis = DIAGNOSIS_RETRIEVAL
    elif not result.answer_correct:
        result.failure_code = "EVIDENCE_INCORRECT"
        result.diagnosis = DIAGNOSIS_RETRIEVAL
    else:
        result.diagnosis = DIAGNOSIS_OK
    if result.passed:
        result.failure_code = ""
    return result


def diagnose_case(results: Sequence[RunResult]) -> str:
    """Diagnose a case from the results of every mode that ran for it.

    The mapping follows the protocol: a failing direct retrieval points at the
    retrieval contract, a failing forced call points at tool use, a failing
    autonomous run points at agent decision making, and a failing oracle run
    points at the model itself.
    """

    by_mode: dict[str, list[RunResult]] = {}
    for result in results:
        by_mode.setdefault(result.mode, []).append(result)

    def passed(mode: str) -> bool | None:
        runs = by_mode.get(mode)
        if not runs:
            return None
        return all(run.passed for run in runs)

    direct = passed(MODE_DIRECT)
    forced = passed(MODE_FORCED)
    autonomous = passed(MODE_AUTONOMOUS)
    oracle = passed(MODE_ORACLE)

    if direct is False:
        return DIAGNOSIS_RETRIEVAL
    if forced is False:
        return DIAGNOSIS_TOOL_USE
    if autonomous is False:
        if oracle is True:
            return DIAGNOSIS_UTILIZATION
        return DIAGNOSIS_AUTONOMY if oracle is None else DIAGNOSIS_MODEL
    if autonomous is True and oracle is False:
        return DIAGNOSIS_MODEL
    if autonomous is None or oracle is None:
        return DIAGNOSIS_INCOMPLETE
    return DIAGNOSIS_OK


def summarize(results: Sequence[RunResult]) -> dict[str, Any]:
    """Aggregate run results without claiming a quality score."""

    total = len(results)
    passed = sum(1 for result in results if result.passed)
    by_mode: dict[str, dict[str, int]] = {}
    for result in results:
        bucket = by_mode.setdefault(result.mode, {"runs": 0, "passed": 0})
        bucket["runs"] += 1
        bucket["passed"] += 1 if result.passed else 0
    return {
        "runs": total,
        "passed": passed,
        "by_mode": by_mode,
        "note": "Counts describe this scenario only; they are not a parity score.",
    }
