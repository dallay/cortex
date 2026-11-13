"""Assertions, per-run results, and cross-mode diagnosis.

The harness never lets the agent grade itself. Every verdict comes from
observable evidence: which tools were called, what they returned, and what the
final answer contained.
"""

from __future__ import annotations

import json
import re
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

# The direct mode calls the MCP tool without a model, so it performs no
# inference. Every other mode drives the agent and therefore calls the model.
INFERENCE_FREE_MODES = (MODE_DIRECT,)


def requires_inference_authorization(entries: Iterable[Mapping[str, Any]]) -> bool:
    """Return whether the planned runs perform at least one model inference."""

    return any(
        entry.get("mode") not in INFERENCE_FREE_MODES for entry in entries
    )

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

# Errors that come from the provider or from the infrastructure, not from
# the model under test. A run that failed this way produced no answer, so it
# says nothing about the memory system and it is not evaluated.
PROVIDER_ERROR_PATTERNS = (
    r"rate[\s_-]?limit",
    r"too many requests",
    r"\b429\b",
    r"quota",
    r"insufficient\s+(credits?|quota|balance)",
    r"(credits?|balance)\s+exhausted",
    r"tokens\s+per\s+(day|minute|second)",
    r"\b(tpd|tpm|rpm)\b",
    r"requests\s+per\s+(minute|hour|day)",
    r"upstream\s+(error|failure)",
    r"provider\s+(error|unavailable|overloaded|failed)",
    r"\b5\d{2}\b",
    r"(server error|bad gateway|service unavailable|gateway timeout)",
    r"connection\s+(refused|reset|aborted|closed)",
    r"\b(econnrefused|econnreset|etimedout|enotfound)\b",
    r"socket\s+hang\s?up",
    r"container\s+(crashed|exited|died|killed)",
    r"docker\s+(daemon|daemon\s+not\s+running)",
    r"timed?\s*out",
    r"timeout",
    r"deadline\s+exceeded",
    r"temporarily\s+unavailable",
    r"circuit\s+breaker",
)

# Errors that come from the model: it called a tool with arguments that the
# tool schema rejects. This is genuine evidence about tool use.
TOOL_SCHEMA_ERROR_PATTERNS = (
    r"did not match schema",
    r"expected\s+\w+,?\s+but got",
    r"parameters for tool",
    r"invalid\s+(tool\s+)?(arguments|parameters|input)",
    r"tool\s+(call|input|arguments)\s+.*(schema|invalid)",
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
    # A run that produced no answer is no evidence about the system under
    # test. Such a run is not evaluated and never counts as a failure.
    evaluated: bool = True

    @property
    def passed(self) -> bool:
        return self.evaluated and self.answer_correct and not self.leaked

    def to_manifest(self) -> dict[str, Any]:
        return {
            "case_id": self.case_id,
            "mode": self.mode,
            "condition": self.condition,
            "repetition": self.repetition,
            "passed": self.passed,
            "evaluated": self.evaluated,
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
    """Return the missing values, the leaked values, and the abstention."""

    missing = tuple(item for item in case.expected_contains if not _contains(answer, item))
    leaked = tuple(item for item in case.expected_absent if _contains(answer, item))
    lowered = answer.lower()
    abstained = any(marker in lowered for marker in ABSTENTION_MARKERS)
    return missing, leaked, abstained


def _matches(patterns: Sequence[str], text: str) -> bool:
    """Return whether one of the patterns matches the text."""

    return any(re.search(pattern, text, re.IGNORECASE) for pattern in patterns)


def _error_text(trace: RunTrace) -> str:
    """Return the error text of a failed run."""

    parts = [error for error in trace.errors if error]
    for call in trace.tool_calls:
        if call.error:
            parts.append(call.error)
    return "\n".join(parts)


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
        error_text = _error_text(trace)
        if _matches(PROVIDER_ERROR_PATTERNS, error_text):
            # The provider or the infrastructure failed. The run produced no
            # answer, so it is no evidence about the system under test.
            result.failure_code = "INFRASTRUCTURE_ERROR"
            result.diagnosis = DIAGNOSIS_INCOMPLETE
            result.evaluated = False
        elif _matches(TOOL_SCHEMA_ERROR_PATTERNS, error_text):
            # The model called a tool with arguments the schema rejects. This
            # is genuine evidence about the tool use of the model.
            result.failure_code = "TOOL_USE_ERROR"
            result.diagnosis = DIAGNOSIS_TOOL_USE
            result.answer_correct = False
            result.evaluated = True
        else:
            result.failure_code = "RUN_ERROR"
            result.diagnosis = DIAGNOSIS_INCOMPLETE
            result.evaluated = False
    elif mode == MODE_ORACLE:
        result.diagnosis = DIAGNOSIS_OK if result.answer_correct else DIAGNOSIS_MODEL
    elif not result.mcp_invoked:
        result.failure_code = "MCP_NOT_INVOKED"
        # A forced run that skips the tool is a tool use failure; an
        # autonomous run that skips it is a decision failure.
        result.diagnosis = (
            DIAGNOSIS_TOOL_USE if mode == MODE_FORCED else DIAGNOSIS_AUTONOMY
        )
    elif not result.answer_correct:
        # Only a wrong answer is a failure. An abstention case that correctly
        # retrieves nothing passes, and a passing run never carries a failure
        # diagnosis.
        if not result.evidence_retrieved:
            result.failure_code = "EVIDENCE_NOT_RETRIEVED"
            result.diagnosis = DIAGNOSIS_RETRIEVAL
        else:
            result.failure_code = "ANSWER_INCORRECT"
            result.diagnosis = DIAGNOSIS_UTILIZATION
    else:
        result.diagnosis = DIAGNOSIS_OK
    if result.passed and result.evaluated:
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
        # A failed MCP request did not exercise retrieval, so it cannot support
        # a retrieval verdict. Keep the transport error visible, but exclude it
        # from evaluated-result counts and cross-mode diagnosis.
        result.failure_code = "MCP_ERROR"
        result.diagnosis = DIAGNOSIS_INCOMPLETE
        result.evaluated = False
    elif not result.answer_correct:
        # Only a wrong answer is a retrieval failure. An abstention case that
        # correctly retrieves nothing passes, and a passing run never carries
        # a failure diagnosis.
        result.failure_code = (
            "EVIDENCE_NOT_RETRIEVED" if not result.evidence_retrieved else "EVIDENCE_INCORRECT"
        )
        result.diagnosis = DIAGNOSIS_RETRIEVAL
    else:
        result.diagnosis = DIAGNOSIS_OK
    if result.passed and result.evaluated:
        result.failure_code = ""
    return result


def diagnose_case(results: Sequence[RunResult]) -> str:
    """Diagnose a case from the results of every mode that ran for it.

    The mapping follows the protocol: a failing direct retrieval points at the
    retrieval contract, a failing forced call points at tool use, a failing
    autonomous run points at agent decision making, and a failing oracle run
    points at the model itself.

    Only evaluated runs carry a verdict. A mode whose runs all failed before
    they produced an answer reports no signal, and the case stays incomplete.
    """

    by_mode: dict[str, list[RunResult]] = {}
    for result in results:
        by_mode.setdefault(result.mode, []).append(result)

    def passed(mode: str) -> bool | None:
        runs = [run for run in by_mode.get(mode, []) if run.evaluated]
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
    """Aggregate run results without claiming a quality score.

    The counts separate the evaluated runs from the runs that were excluded
    because they produced no answer.
    """

    evaluated = [result for result in results if result.evaluated]
    by_mode: dict[str, dict[str, int]] = {}
    for result in results:
        bucket = by_mode.setdefault(
            result.mode,
            {"runs": 0, "evaluated": 0, "not_evaluated": 0, "passed": 0},
        )
        bucket["runs"] += 1
        if result.evaluated:
            bucket["evaluated"] += 1
            bucket["passed"] += 1 if result.passed else 0
        else:
            bucket["not_evaluated"] += 1
    return {
        "runs": len(results),
        "evaluated": len(evaluated),
        "not_evaluated": len(results) - len(evaluated),
        "passed": sum(1 for result in evaluated if result.passed),
        "by_mode": by_mode,
        "note": "Counts describe this scenario only; they are not a parity score.",
    }
