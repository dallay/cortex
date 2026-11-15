"""Local, non-attested provenance assessment for H5 protocol gates."""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
from typing import Any
from pathlib import Path

_COMMON = {
    "protocol_hash", "review_reference", "operator_corroboration", "corpus_hash",
    "runner_hash", "quality_hash", "bridge_image_id", "bridge_container_id",
    "bridge_binary_hash", "model_bundle_hash", "readonly_mounts", "effective_config",
    "compose_inspect", "encoder_loaded_identity", "run_id", "started_at", "finished_at",
    "gate_results", "encoder_response_identity", "encoder_expected_identity",
    "encoder_response_binding",
}
_GATE = {"ann_exact": set(), "absolute_relevance": {"labels_hash", "rubric_hash", "adjudication_evidence"}}
_SECRET = re.compile(r"token|secret|password|credential|api[_-]?key", re.IGNORECASE)
_HEX_64 = re.compile(r"[0-9a-f]{64}\Z")
_HEX_40 = re.compile(r"[0-9a-f]{40}\Z")
_IDENTITY_FIELDS = {
    "declared": {"artifact_bundle_id", "model_id", "revision", "space_id"},
    "loaded": {"validated_profile", "config_sha256"},
}


class DockerEngine:
    """Read-only Docker Engine adapter. Tests can inject an inspect-compatible fake."""

    def inspect(self, container_id: str) -> list[dict[str, Any]]:
        completed = subprocess.run(
            ["docker", "inspect", container_id], check=True, capture_output=True,
            text=True, timeout=15,
        )
        payload = json.loads(completed.stdout)
        if type(payload) is not list or not payload or any(type(item) is not dict for item in payload):
            raise ValueError("Docker inspect returned malformed container data")
        return payload


class LocalObservations(dict[str, Any]):
    """Mapping returned by the local collector, not a caller-provided claim document."""


def _record(value: Any, *, source: str, method: str, observed_at: str) -> dict[str, Any]:
    return {"value": value, "source": source, "method": method, "observed_at": observed_at}


def collect_observations(engine: Any, *, container_ids: list[str], files: dict[str, str | Path],
                         run_id: str, started_at: str, finished_at: str,
                         caller_claims: dict[str, Any] | None = None) -> dict[str, Any]:
    """Collect Docker inspect and local-byte observations; caller claims are never evidence.

    This intentionally cannot establish which encoder/configuration was loaded in bridge memory.
    Container configuration and labels are observations of Docker metadata, not runtime attestation.
    """
    del caller_claims  # Explicitly reject the trust of supplied identity/configuration claims.
    if type(container_ids) is not list or any(type(item) is not str or not item for item in container_ids):
        raise ValueError("container_ids must be a list of non-empty IDs")
    if type(files) is not dict:
        raise ValueError("files must be a mapping of evidence names to local paths")
    inspected: list[dict[str, Any]] = []
    for container_id in container_ids:
        payload = engine.inspect(container_id)
        if type(payload) is not list or len(payload) != 1 or type(payload[0]) is not dict:
            raise ValueError("Docker inspect must identify exactly one container")
        inspected.append(payload[0])

    observed_at = finished_at
    result: LocalObservations = LocalObservations({
        "run_id": _record(run_id, source="runner", method="run correlation", observed_at=observed_at),
        "started_at": _record(started_at, source="runner", method="clock", observed_at=observed_at),
        "finished_at": _record(finished_at, source="runner", method="clock", observed_at=observed_at),
    })
    if inspected:
        bridge = inspected[0]
        result["bridge_image_id"] = _record(bridge.get("Image"), source="Docker Engine",
                                              method="container inspect", observed_at=observed_at)
        result["bridge_container_id"] = _record(bridge.get("Id"), source="Docker Engine",
                                                  method="container inspect", observed_at=observed_at)
        result["readonly_mounts"] = _record(
            [{"source": mount.get("Source"), "destination": mount.get("Destination"),
              "rw": mount.get("RW")} for item in inspected
             for mount in item.get("Mounts", []) if type(mount) is dict],
            source="Docker Engine", method="container inspect", observed_at=observed_at)
        result["compose_inspect"] = _record(
            [{"id": item.get("Id"), "image": item.get("Image"), "status":
              (item.get("State") or {}).get("Status") if type(item.get("State")) is dict else None,
              "compose_project": ((item.get("Config") or {}).get("Labels") or {}).get(
                  "com.docker.compose.project") if type(item.get("Config")) is dict else None}
             for item in inspected], source="Docker Engine", method="container inspect",
            observed_at=observed_at)
    for key, path in files.items():
        allowed_file_keys = _COMMON - {"encoder_loaded_identity", "effective_config"}
        if key not in allowed_file_keys and key not in {"labels_hash", "rubric_hash", "adjudication_evidence"}:
            raise ValueError(f"unsupported local evidence key: {key}")
        content = Path(path).read_bytes()
        result[key] = _record(sha256_bytes(content), source="local filesystem",
                              method="SHA-256 of file bytes", observed_at=observed_at)
    # Loaded encoder/config identity cannot be derived from Docker inspect or local file hashes.
    result.pop("encoder_loaded_identity", None)
    result.pop("effective_config", None)
    return result


def sha256_bytes(content: bytes) -> str:
    if type(content) is not bytes:
        raise TypeError("content must be bytes")
    return "sha256:" + hashlib.sha256(content).hexdigest()


def sanitize_encoder_identity(value: object) -> dict[str, dict[str, str]]:
    """Accept only the closed, non-sensitive identity returned with H5 search results."""
    if type(value) is not dict or set(value) != set(_IDENTITY_FIELDS):
        raise ValueError("encoder identity has unknown or missing groups")
    safe = {}
    for group, fields in _IDENTITY_FIELDS.items():
        record = value[group]
        if type(record) is not dict or set(record) != fields:
            raise ValueError(f"encoder identity {group} has unknown or missing properties")
        if any(type(item) is not str or not item for item in record.values()):
            raise ValueError(f"encoder identity {group} values must be non-empty strings")
        safe[group] = dict(record)
    declared = safe["declared"]
    loaded = safe["loaded"]
    if (not _HEX_64.fullmatch(declared["artifact_bundle_id"])
            or declared["model_id"] != "sentence-transformers/all-MiniLM-L6-v2"
            or not _HEX_40.fullmatch(declared["revision"])
            or not _HEX_64.fullmatch(declared["space_id"])
            or loaded["validated_profile"] != "muninn-minilm-profile-v1"
            or not _HEX_64.fullmatch(loaded["config_sha256"])):
        raise ValueError("encoder identity contains an unsupported value")
    return safe


def correlate_encoder_identity(
    response_identity: object, *, expected_identity: object,
    observed_image_id: str | None, observed_container_id: str | None,
    observed_run_id: str | None, response_run_id: str | None,
    response_query_id: str | None, expected_query_id: str | None,
) -> dict[str, Any]:
    """Correlate a response self-report with independent expected and execution facts."""
    try:
        response = sanitize_encoder_identity(response_identity)
    except ValueError:
        return {"status": "insufficient", "reason": "response_identity_missing_or_invalid"}
    if expected_identity is None:
        return {"status": "partial", "reason": "expected_protocol_identity_missing"}
    try:
        expected = sanitize_encoder_identity(expected_identity)
    except ValueError:
        return {"status": "insufficient", "reason": "expected_protocol_identity_invalid"}
    if response != expected:
        return {"status": "insufficient", "reason": "response_identity_mismatch"}
    if not all((observed_image_id, observed_container_id, observed_run_id,
                response_run_id, response_query_id, expected_query_id)):
        return {"status": "partial", "reason": "execution_or_query_binding_missing"}
    if response_run_id != observed_run_id or response_query_id != expected_query_id:
        return {"status": "insufficient", "reason": "response_execution_or_query_mismatch"}
    return {
        "status": "correlated_self_report",
        "response_identity": response,
        "observed_image_id": observed_image_id,
        "observed_container_id": observed_container_id,
        "observed_run_id": observed_run_id,
        "query_id": expected_query_id,
        "attestation": False,
    }


def redact(value: Any, key: str = "") -> Any:
    """Redact values under sensitive-key names while retaining surrounding facts."""
    if _SECRET.search(key):
        return "[REDACTED]"
    if type(value) is dict:
        return {name: redact(item, str(name)) for name, item in value.items()}
    if type(value) is list:
        return [redact(item) for item in value]
    return value


def _check_records(required: set[str], observations: dict[str, Any]) -> tuple[list[str], list[str]]:
    """Share record checks, not caller-selectable provenance baselines."""
    missing: list[str] = []
    conflicts: list[str] = []
    for key in sorted(required):
        item = observations.get(key)
        if (type(item) is not dict or not item.get("value")
                or any(type(item.get(field)) is not str or not item[field].strip()
                       for field in ("source", "method", "observed_at"))):
            missing.append(key)
            continue
        if "conflict" in item and type(item["conflict"]) is not bool:
            conflicts.append(key)
        elif item.get("conflict") is True:
            conflicts.append(key)
    return missing, conflicts


_POST_RESPONSE = {"gate_results", "encoder_response_identity", "encoder_response_binding",
                  "finished_at", "encoder_loaded_identity"}


def assess_preflight_provenance(gate: str, observations: object) -> dict[str, Any]:
    """Check prior local records only; eligibility is neither authorization nor verification.

    effective_config here contains available external configuration observations.
    This does not fill or verify response-dependent configuration for final assessment.
    The runner must separately corroborate values against protocol/corpus/run.
    """
    if gate not in _GATE:
        raise ValueError("unknown H5 gate")
    if not isinstance(observations, LocalObservations):
        raise ValueError("preflight requires local observations, not caller claims")
    required = (_COMMON - _POST_RESPONSE) | _GATE[gate]
    missing, conflicts = _check_records(required, observations)
    return {"status": "blocked" if missing or conflicts else "eligible",
            "observations": redact({key: observations[key] for key in sorted(required) if key in observations}),
            "missing_or_conflicting": sorted(set(missing + conflicts)),
            "trust_boundary": "repository, operator, host and Docker Engine; no cryptographic attestation"}


def assess_gate_provenance(gate: str, observations: object) -> dict[str, Any]:
    """Assess evidence records independently for one gate, without caller thresholds."""
    if gate not in _GATE:
        raise ValueError("unknown H5 gate")
    if not isinstance(observations, dict):
        raise ValueError("observations must be a mapping")
    required = _COMMON | _GATE[gate]
    missing, conflicts = _check_records(required, observations)
    identity_records = [observations.get(key) for key in (
        "encoder_response_identity", "encoder_expected_identity", "encoder_response_binding",
        "bridge_image_id", "bridge_container_id", "run_id",
    )]
    if all(type(item) is dict and item.get("value") for item in identity_records):
        response, expected, binding, image, container, run = identity_records
        details = binding["value"]
        if type(details) is not dict or set(details) != {
            "response_run_id", "response_query_id", "expected_query_id"
        }:
            conflicts.append("encoder_response_binding")
        else:
            correlation = correlate_encoder_identity(
                response["value"], expected_identity=expected["value"],
                observed_image_id=image["value"], observed_container_id=container["value"],
                observed_run_id=run["value"], response_run_id=details["response_run_id"],
                response_query_id=details["response_query_id"],
                expected_query_id=details["expected_query_id"],
            )
            if correlation["status"] != "correlated_self_report":
                conflicts.append("encoder_response_identity")
    else:
        missing.extend(key for key in (
            "encoder_response_identity", "encoder_expected_identity", "encoder_response_binding"
        ) if key not in missing)
    unidentifiable = {"bridge_container_id", "run_id"}.intersection(missing)
    status = "insufficient" if conflicts or unidentifiable else "partial" if missing else "verified"
    return {"status": status, "observations": redact({key: observations[key] for key in sorted(required) if key in observations}),
            "missing_or_conflicting": sorted(set(missing + conflicts)),
            "trust_boundary": "repository, operator, host and Docker Engine; no cryptographic attestation"}
