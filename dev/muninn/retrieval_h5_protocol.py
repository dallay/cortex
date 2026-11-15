"""Validation helpers for explicitly approved H5 protocol-runs.

V2 defines the closed offline ANN-only contract. An incomplete document is useful as a
schema example, but cannot authorize confirmatory work.
"""
from __future__ import annotations

import hashlib
import json
import math
import re
from datetime import datetime
from typing import Any

from retrieval_h5_provenance import sanitize_encoder_identity

_SCHEMA_VERSIONS = ("h5-acceptance/v1", "h5-acceptance/v2")
_TOP_LEVEL = {"schema_version", "protocol_id", "protocol_version", "repository_review", "run_inputs", "gates"}
_REVIEW_KEYS = {"commit", "review_reference", "approved_at", "approved_gates"}
_INPUT_KEYS = {
    "metrics_and_k", "acceptance_rules_and_tolerances", "run_count_and_aggregation",
    "corpus_and_query_candidate_scope", "labels_rubric_and_adjudication", "ann_exact_comparison_scope",
    "encoder_identity", "provenance",
}
_GATES = {"ann_exact", "absolute_relevance"}
_COMMON_INPUTS = {"metrics_and_k", "acceptance_rules_and_tolerances", "run_count_and_aggregation",
                  "corpus_and_query_candidate_scope", "encoder_identity", "provenance"}
_METRICS = {"ExactTopKRecall@K", "OrderedTopKMatch@K", "Recall@K", "nDCG@K"}
_RFC3339_TIMESTAMP = re.compile(
    r"^\d{4}-(?:0[1-9]|1[0-2])-(?:0[1-9]|[12]\d|3[01])"
    r"T(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?"
    r"(?:Z|[+-](?:[01]\d|2[0-3]):[0-5]\d)$"
)


def _object(**fields: object) -> dict[str, object]:
    return {"kind": "object", "fields": fields}


def _string() -> dict[str, object]:
    return {"kind": "string"}


def _integer() -> dict[str, object]:
    return {"kind": "integer"}


def _decision(value_schema: dict[str, object]) -> dict[str, object]:
    return {"kind": "decision", "value": value_schema}


_VALUE_SCHEMAS = {
    "metrics_and_k": _object(k_by_metric=_object(**{metric: _integer() for metric in sorted(_METRICS)})),
    "acceptance_rules_and_tolerances": _object(),
    "run_count_and_aggregation": _object(run_count=_integer(), aggregation=_string(),
                                         tie_policy=_string(), undefined_policy=_string()),
    "corpus_and_query_candidate_scope": _object(
        corpus_id=_string(), version=_string(), queries={"kind": "array", "items": _object(
            query_id=_string(), candidate_ids={"kind": "array", "items": _string()})}),
    "labels_rubric_and_adjudication": _object(labels_version=_string(), rubric=_string(), adjudication=_string()),
    "ann_exact_comparison_scope": _object(retrieval_path=_string(), candidate_scope=_string()),
    "encoder_identity": _object(
        declared=_object(artifact_bundle_id={"kind": "pattern", "pattern": r"^[0-9a-f]{64}$"},
                         model_id={"kind": "const", "value": "sentence-transformers/all-MiniLM-L6-v2"},
                         revision={"kind": "pattern", "pattern": r"^[0-9a-f]{40}$"},
                         space_id={"kind": "pattern", "pattern": r"^[0-9a-f]{64}$"}),
        loaded=_object(validated_profile={"kind": "const", "value": "muninn-minilm-profile-v1"},
                       config_sha256={"kind": "pattern", "pattern": r"^[0-9a-f]{64}$"})),
    "provenance": _object(encoder_model=_string(), build_bridge=_string(), dependencies=_string(),
                          corpus_labels=_string(), retrieval_configuration=_string(), runtime_environment=_string()),
}


# V2 specializes only these values; all envelopes and approval checks stay shared.
_V2_VALUE_SCHEMAS = {
    "metrics_and_k": _object(k_by_metric=_object(**{
        metric: {"kind": "integer", "minimum": 10, "maximum": 10}
        for metric in ("ExactTopKRecall@K", "OrderedTopKMatch@K")})),
    "acceptance_rules_and_tolerances": _object(ann_exact=_object(
        criterion=_string(), threshold={"kind": "number", "minimum": 1, "maximum": 1},
        policy_id={"kind": "const", "value": "ann-exact-per-query-equality-v1"})),
    "run_count_and_aggregation": _object(
        run_count={"kind": "integer", "minimum": 1, "maximum": 1},
        aggregation={"kind": "const", "value": "per-query-governing-macro-descriptive-v1"},
        tie_policy={"kind": "const", "value": "observed-ranking-no-retrospective-tiebreak-v1"},
        undefined_policy={"kind": "const", "value": "required-undefined-blocks-pass-v1"}),
    "corpus_and_query_candidate_scope": _object(
        **_VALUE_SCHEMAS["corpus_and_query_candidate_scope"]["fields"],
        corpus_hash={"kind": "pattern", "pattern": r"^sha256:[0-9a-f]{64}$"}),
    "ann_exact_comparison_scope": _object(
        retrieval_path={"kind": "const", "value": "/h5/diagnostic/search"},
        candidate_scope={"kind": "const", "value": "same-query-candidates-v1"},
        search_config=_object(limit={"kind": "integer", "minimum": 10, "maximum": 100},
                              threshold={"kind": "number", "minimum": 0, "maximum": 1})),
}


def _finding(path: str, reason: str) -> dict[str, str]:
    return {"path": path, "reason": reason}


def _is_placeholder(value: str) -> bool:
    normalized = value.strip().casefold()
    return (normalized in {"unknown", "none", "null", "n/a", "tbc", "...", "<value>"}
            or normalized.startswith(("pending", "placeholder", "todo", "tbd", "change_me", "choose ", "<")))


def _validate_value(value: object, schema: dict[str, object], path: str,
                    missing: list[dict[str, str]], malformed: list[dict[str, str]]) -> None:
    kind = schema["kind"]
    if kind == "object":
        fields = schema["fields"]
        if type(value) is not dict:
            malformed.append(_finding(path, "expected object"))
            return
        if not value:
            malformed.append(_finding(path, "object must not be empty"))
        if any(type(name) is not str for name in value):
            malformed.append(_finding(path, "object property names must be strings"))
            return
        for name in sorted(value.keys() - fields.keys()):
            malformed.append(_finding(f"{path}.{name}", "unknown property"))
        for name in sorted(fields.keys() - value.keys()):
            missing.append(_finding(f"{path}.{name}", "required property missing"))
        for name in sorted(fields.keys() & value.keys()):
            _validate_value(value[name], fields[name], f"{path}.{name}", missing, malformed)
    elif kind == "array":
        if type(value) is not list:
            malformed.append(_finding(path, "expected array"))
        elif not value:
            malformed.append(_finding(path, "array must not be empty"))
        else:
            for index, item in enumerate(value):
                _validate_value(item, schema["items"], f"{path}[{index}]", missing, malformed)
    elif kind in ("integer", "number"):
        if "minimum" in schema:
            # JSON Schema integers include finite integral representations such as 10.0.
            valid = (type(value) in (int, float)
                     and schema["minimum"] <= value <= schema["maximum"]
                     and (type(value) is int or math.isfinite(value))
                     and (kind != "integer" or value == int(value)))
        elif kind == "integer":
            valid = type(value) is int and value > 0
        else:
            valid = type(value) in (int, float) and math.isfinite(value)
        if not valid:
            malformed.append(_finding(path, "expected finite number in declared domain"))
    elif kind in ("string", "pattern", "const"):
        if type(value) is not str or not value.strip():
            malformed.append(_finding(path, "expected non-empty string"))
            return
        if _is_placeholder(value):
            malformed.append(_finding(path, "placeholder value is not allowed"))
        elif kind == "pattern":
            import re
            if re.fullmatch(schema["pattern"], value) is None:
                malformed.append(_finding(path, "value has invalid format"))
        elif kind == "const" and value != schema["value"]:
            malformed.append(_finding(path, "value does not match required identifier"))


def canonical_protocol_hash(protocol: object) -> str:
    """Hash canonical JSON bytes, excluding no caller-controlled fields."""
    if type(protocol) is not dict:
        raise ValueError("protocol must be an object")
    canonical = json.dumps(protocol, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def validate_protocol(protocol: object) -> dict[str, Any]:
    """Validate shape and report missing per-run decisions without defaults."""
    if type(protocol) is not dict or set(protocol) != _TOP_LEVEL:
        raise ValueError("protocol has unknown or missing top-level properties")
    if protocol["schema_version"] not in _SCHEMA_VERSIONS:
        raise ValueError("unsupported protocol schema_version")
    missing: list[dict[str, str]] = []
    pending: list[dict[str, str]] = []
    malformed: list[dict[str, str]] = []
    for name in ("protocol_id", "protocol_version"):
        if type(protocol[name]) is not str or not protocol[name].strip():
            raise ValueError(f"{name} must be a non-empty string")
        if _is_placeholder(protocol[name]):
            pending.append(_finding(name, "metadata is pending"))
    review = protocol["repository_review"]
    if type(review) is not dict or set(review) != _REVIEW_KEYS:
        raise ValueError("repository_review has unknown or missing properties")
    for name in ("commit", "review_reference", "approved_at"):
        if type(review[name]) is not str or not review[name].strip():
            raise ValueError(f"repository_review.{name} is required")
        if _is_placeholder(review[name]):
            pending.append(_finding(f"repository_review.{name}", "metadata is pending"))
    approved_at = review["approved_at"]
    if not _is_placeholder(approved_at):
        try:
            parsed_approved_at = datetime.fromisoformat(approved_at.replace("Z", "+00:00"))
            if (_RFC3339_TIMESTAMP.fullmatch(approved_at) is None
                    or parsed_approved_at.tzinfo is None or parsed_approved_at.utcoffset() is None):
                raise ValueError("timestamp must include timezone")
        except ValueError:
            malformed.append(_finding("repository_review.approved_at", "expected RFC3339 timestamp with timezone"))
    approved_gates = review["approved_gates"]
    if (type(approved_gates) is not list
            or any(type(gate) is not str or gate not in _GATES for gate in approved_gates)
            or len(set(approved_gates)) != len(approved_gates)):
        raise ValueError("approved_gates must list known gates without duplicates")
    inputs = protocol["run_inputs"]
    if type(inputs) is not dict or inputs.keys() - _INPUT_KEYS:
        raise ValueError("run_inputs has unknown properties")
    gates = protocol["gates"]
    if type(gates) is not dict or set(gates) != _GATES or any(type(v) is not bool for v in gates.values()):
        raise ValueError("gates must explicitly enable or disable both known gates")
    if protocol["schema_version"] == "h5-acceptance/v2":
        for gate, expected in (("ann_exact", True), ("absolute_relevance", False)):
            if gates[gate] is not expected:
                malformed.append(_finding(f"gates.{gate}", "v2 requires ANN-only gates"))
    required = set(_COMMON_INPUTS)
    if gates["absolute_relevance"]:
        required.add("labels_rubric_and_adjudication")
    if gates["ann_exact"]:
        required.add("ann_exact_comparison_scope")
    for name in sorted(required):
        path = f"run_inputs.{name}"
        if name not in inputs:
            missing.append(_finding(path, "applicable decision missing"))
            continue
        envelope = inputs[name]
        if type(envelope) is not dict:
            malformed.append(_finding(path, "expected decision envelope"))
            continue
        if any(type(key) is not str for key in envelope):
            malformed.append(_finding(path, "decision property names must be strings"))
            continue
        expected_keys = {"status", "value", "rationale"}
        for key in sorted(envelope.keys() - expected_keys):
            malformed.append(_finding(f"{path}.{key}", "unknown property"))
        for key in sorted(expected_keys - envelope.keys()):
            missing.append(_finding(f"{path}.{key}", "required property missing"))
        if envelope.keys() - expected_keys or expected_keys - envelope.keys():
            continue
        status, value, rationale = envelope["status"], envelope["value"], envelope["rationale"]
        if status not in ("pending", "approved") or type(status) is not str:
            malformed.append(_finding(f"{path}.status", "expected pending or approved"))
            continue
        _validate_value(rationale, _string(), f"{path}.rationale", missing, malformed)
        if status == "pending":
            if value is not None:
                malformed.append(_finding(f"{path}.value", "pending decision requires null value"))
            else:
                pending.append(_finding(path, "decision is pending"))
        elif value is None:
            malformed.append(_finding(f"{path}.value", "approved decision requires a value"))
        else:
            value_schema = _VALUE_SCHEMAS[name]
            if name == "metrics_and_k":
                applicable_metrics = set()
                if gates["ann_exact"]:
                    applicable_metrics.update(("ExactTopKRecall@K", "OrderedTopKMatch@K"))
                if gates["absolute_relevance"]:
                    applicable_metrics.update(("Recall@K", "nDCG@K"))
                value_schema = _object(k_by_metric=_object(**{
                    metric: _integer() for metric in sorted(applicable_metrics)
                }))
            elif name == "acceptance_rules_and_tolerances":
                applicable_rules = {}
                if gates["ann_exact"]:
                    applicable_rules["ann_exact"] = _object(criterion=_string(), threshold={"kind": "number"})
                if gates["absolute_relevance"]:
                    applicable_rules["absolute_relevance"] = _object(criterion=_string(), threshold={"kind": "number"})
                value_schema = _object(**applicable_rules)
            if protocol["schema_version"] == "h5-acceptance/v2":
                value_schema = _V2_VALUE_SCHEMAS.get(name, value_schema)
            _validate_value(value, value_schema, f"{path}.value", missing, malformed)
            if name == "encoder_identity" and not any(item["path"].startswith(f"{path}.value") for item in malformed):
                try:
                    sanitize_encoder_identity(value)
                except (TypeError, ValueError):
                    malformed.append(_finding(f"{path}.value", "invalid encoder identity"))
    for gate, enabled in sorted(gates.items()):
        if enabled and gate not in approved_gates:
            missing.append(_finding(f"repository_review.approved_gates[{gate}]", "gate approval is not recorded"))
    for collection in (missing, pending, malformed):
        collection.sort(key=lambda item: (item["path"], item["reason"]))
    all_missing_paths = [item["path"] for item in missing]
    all_missing_paths.extend(item["path"] for item in pending)
    all_missing_paths.extend(item["path"] for item in malformed)
    return {"gates": dict(gates), "missing": missing, "pending": pending, "malformed": malformed,
            "missing_fields": sorted(set(all_missing_paths)),
            "confirmatory_ready": not (missing or pending or malformed),
            "sha256": canonical_protocol_hash(protocol)}


def corroborate_approval(protocol: object, *, expected_sha256: str, operator_record: object) -> bool:
    """Validate recorded operator process evidence; make no cryptographic claim."""
    validated = validate_protocol(protocol)
    if type(operator_record) is not dict or set(operator_record) != {
        "protocol_sha256", "commit", "review_reference", "corroborated_at", "corroborated_by"
    }:
        return False
    review = protocol["repository_review"]
    if (type(expected_sha256) is not str or expected_sha256 != validated["sha256"]
            or operator_record["protocol_sha256"] != validated["sha256"]
            or operator_record["commit"] != review["commit"]
            or operator_record["review_reference"] != review["review_reference"]
            or type(operator_record["corroborated_by"]) is not str
            or not operator_record["corroborated_by"].strip()):
        return False
    try:
        approved_at = datetime.fromisoformat(review["approved_at"].replace("Z", "+00:00"))
        corroborated_at = datetime.fromisoformat(operator_record["corroborated_at"].replace("Z", "+00:00"))
    except (AttributeError, TypeError, ValueError):
        return False
    if (approved_at.tzinfo is None or approved_at.utcoffset() is None
            or corroborated_at.tzinfo is None or corroborated_at.utcoffset() is None):
        return False
    if corroborated_at < approved_at:
        return False
    return validated["confirmatory_ready"]
