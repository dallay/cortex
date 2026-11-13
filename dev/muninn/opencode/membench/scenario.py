"""Scenario loading and validation.

A scenario is the immutable, versioned description of what the benchmark
measures. It carries synthetic records, the prompts, and the observable
expectations for each case. The harness never invents expectations at runtime.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence

from .errors import BenchError

RECORD_KINDS = ("message", "preference")


@dataclass(frozen=True)
class SeedRecord:
    """One synthetic record written into the disposable database."""

    kind: str
    owner: str
    text: str
    category: str | None = None
    supersedes: str | None = None
    session_id: str | None = None

    def to_manifest(self) -> dict[str, Any]:
        return {
            "kind": self.kind,
            "owner": self.owner,
            "category": self.category,
            "session_id": self.session_id,
            "text_length": len(self.text),
            "supersedes": self.supersedes,
        }


@dataclass(frozen=True)
class Case:
    """One evaluation case: a fixture, a prompt, and observable expectations."""

    id: str
    goal: str
    prompt: str
    oracle_context: str
    owner: str
    empty_owner: str
    seed: tuple[SeedRecord, ...]
    expected_contains: tuple[str, ...]
    expected_absent: tuple[str, ...]
    abstain: bool
    requires_semantic: bool
    direct_tool: str
    direct_arguments: Mapping[str, Any]

    def to_manifest(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "goal": self.goal,
            "owner_scope": self.owner,
            "empty_owner_scope": self.empty_owner,
            "abstain": self.abstain,
            "requires_semantic": self.requires_semantic,
            "seed": [record.to_manifest() for record in self.seed],
            "expected_contains": list(self.expected_contains),
            "expected_absent": list(self.expected_absent),
        }


@dataclass(frozen=True)
class Scenario:
    """A versioned set of cases and the owner scopes they use."""

    version: str
    owner: str
    empty_owner: str
    cases: tuple[Case, ...]
    source: str = ""

    def case(self, case_id: str) -> Case:
        for candidate in self.cases:
            if candidate.id == case_id:
                return candidate
        raise BenchError(f"unknown case '{case_id}'")

    @property
    def case_ids(self) -> tuple[str, ...]:
        return tuple(case.id for case in self.cases)


def _require(mapping: Mapping[str, Any], key: str, where: str) -> Any:
    if key not in mapping:
        raise BenchError(f"{where} is missing '{key}'")
    return mapping[key]


def _require_text(mapping: Mapping[str, Any], key: str, where: str) -> str:
    value = _require(mapping, key, where)
    if not isinstance(value, str) or not value.strip():
        raise BenchError(f"{where} field '{key}' must be a non-empty string")
    return value


def _string_tuple(value: Any, where: str) -> tuple[str, ...]:
    if value is None:
        return ()
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        raise BenchError(f"{where} must be a list of strings")
    return tuple(value)


def _parse_record(raw: Any, where: str, default_owner: str) -> SeedRecord:
    if not isinstance(raw, Mapping):
        raise BenchError(f"{where} must be an object")
    kind = _require_text(raw, "kind", where)
    if kind not in RECORD_KINDS:
        raise BenchError(f"{where} kind '{kind}' is not one of {RECORD_KINDS}")
    text = _require_text(raw, "text", where)
    owner = raw.get("owner") or default_owner
    if not isinstance(owner, str) or not owner.strip():
        raise BenchError(f"{where} owner must be a non-empty string")
    category = raw.get("category")
    if kind == "preference" and not (isinstance(category, str) and category.strip()):
        raise BenchError(f"{where} preference records require a category")
    if kind == "message" and category is not None:
        raise BenchError(f"{where} message records must not declare a category")
    supersedes = raw.get("supersedes")
    if supersedes is not None and not isinstance(supersedes, str):
        raise BenchError(f"{where} supersedes must be a string")
    session_id = raw.get("session_id")
    if session_id is not None and not isinstance(session_id, str):
        raise BenchError(f"{where} session_id must be a string")
    return SeedRecord(
        kind=kind,
        owner=owner,
        text=text,
        category=category if isinstance(category, str) else None,
        supersedes=supersedes if isinstance(supersedes, str) else None,
        session_id=session_id,
    )


def _parse_case(
    raw: Any, index: int, default_owner: str, default_empty_owner: str
) -> Case:
    where = f"cases[{index}]"
    if not isinstance(raw, Mapping):
        raise BenchError(f"{where} must be an object")
    case_id = _require_text(raw, "id", where)
    owner = raw.get("owner") or default_owner
    if not isinstance(owner, str) or not owner.strip():
        raise BenchError(f"{where} owner must be a non-empty string")
    empty_owner = raw.get("empty_owner") or f"{default_empty_owner}-{case_id}"
    if not isinstance(empty_owner, str) or not empty_owner.strip():
        raise BenchError(f"{where} empty_owner must be a non-empty string")
    if owner == empty_owner:
        raise BenchError(f"{where} owner and empty_owner must differ")
    seed_raw = _require(raw, "seed", where)
    if not isinstance(seed_raw, list):
        raise BenchError(f"{where} seed must be a list")
    seed = tuple(
        _parse_record(item, f"{where}.seed[{position}]", owner)
        for position, item in enumerate(seed_raw)
    )
    direct = raw.get("direct") or {}
    if not isinstance(direct, Mapping):
        raise BenchError(f"{where} direct must be an object")
    arguments = direct.get("arguments") or {}
    if not isinstance(arguments, Mapping):
        raise BenchError(f"{where} direct arguments must be an object")
    abstain = raw.get("abstain", False)
    if not isinstance(abstain, bool):
        raise BenchError(f"{where} abstain must be a boolean")
    requires_semantic = raw.get("requires_semantic", False)
    if not isinstance(requires_semantic, bool):
        raise BenchError(f"{where} requires_semantic must be a boolean")
    expected_contains = _string_tuple(raw.get("expected_contains"), f"{where}.expected_contains")
    expected_absent = _string_tuple(raw.get("expected_absent"), f"{where}.expected_absent")
    if not abstain and not expected_contains:
        raise BenchError(f"{where} must declare expected_contains or set abstain")
    return Case(
        id=case_id,
        goal=_require_text(raw, "goal", where),
        prompt=_require_text(raw, "prompt", where),
        oracle_context=str(raw.get("oracle_context") or ""),
        owner=owner,
        empty_owner=empty_owner,
        seed=seed,
        expected_contains=expected_contains,
        expected_absent=expected_absent,
        abstain=abstain,
        requires_semantic=requires_semantic,
        direct_tool=str(direct.get("tool") or "muninn_search_preferences"),
        direct_arguments=dict(arguments),
    )


def load_scenario(path: str | Path) -> Scenario:
    """Load and validate a scenario file."""

    location = Path(path)
    try:
        payload = json.loads(location.read_text(encoding="utf-8"))
    except FileNotFoundError as error:
        raise BenchError(f"scenario file not found: {location}") from error
    except json.JSONDecodeError as error:
        raise BenchError(f"scenario file is not valid JSON: {location}: {error}") from error
    if not isinstance(payload, Mapping):
        raise BenchError("scenario root must be an object")
    version = _require_text(payload, "version", "scenario")
    owner = _require_text(payload, "owner", "scenario")
    empty_owner = _require_text(payload, "empty_owner", "scenario")
    if owner == empty_owner:
        raise BenchError("scenario owner and empty_owner must differ")
    cases_raw = _require(payload, "cases", "scenario")
    if not isinstance(cases_raw, list) or not cases_raw:
        raise BenchError("scenario cases must be a non-empty list")
    cases = tuple(
        _parse_case(item, index, owner, empty_owner) for index, item in enumerate(cases_raw)
    )
    identifiers = [case.id for case in cases]
    if len(set(identifiers)) != len(identifiers):
        raise BenchError("scenario case ids must be unique")
    scopes = [case.owner for case in cases]
    if len(set(scopes)) != len(scopes):
        raise BenchError("scenario cases must use distinct owner scopes")
    return Scenario(
        version=version,
        owner=owner,
        empty_owner=empty_owner,
        cases=cases,
        source=str(location),
    )


def case_ids(scenario: Scenario, requested: Sequence[str] | None) -> tuple[str, ...]:
    """Resolve the requested case identifiers against the scenario."""

    if not requested:
        return scenario.case_ids
    unknown = [case_id for case_id in requested if case_id not in scenario.case_ids]
    if unknown:
        raise BenchError(f"unknown case identifiers: {', '.join(sorted(unknown))}")
    return tuple(requested)
