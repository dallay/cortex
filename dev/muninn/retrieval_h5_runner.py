"""One-shot evaluator for the isolated Muninn H5 retrieval pilot."""
from __future__ import annotations

import argparse
from copy import deepcopy
import hashlib
import json
import math
import os
import platform
import re
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Protocol
from urllib.parse import urlsplit

from retrieval_quality import (annotations_from_csv, compose_report, corpus_hash, gate_result,
                               serialize_gate_metrics, validate_corpus)
from retrieval_h5_protocol import _is_placeholder, corroborate_approval, validate_protocol
from retrieval_h5_provenance import (LocalObservations, assess_gate_provenance, assess_preflight_provenance,
                                     sanitize_encoder_identity)

KS = (1, 3, 5)
SESSION_PREFIX = "muninn-h5-retrieval-pilot-v1"


def serialize_protocol_gate_metrics(ann_queries: list[dict], quality_queries: list[dict], *, k: int) -> dict:
    """Serialize explicit per-query gate metrics without evaluating acceptance or running retrieval."""
    return serialize_gate_metrics(ann_queries, quality_queries, k)


def integrate_protocol_gate_metrics(
    report: dict,
    *,
    k: int,
    ann_queries: list[dict] | None = None,
    quality_queries: list[dict] | None = None,
) -> dict:
    """Attach already-evaluated per-query metrics to their gate without changing gate status."""
    if type(report) is not dict or type(report.get("gates")) is not dict:
        raise ValueError("protocol report must contain a gates object")
    if type(k) is not int or k <= 0:
        raise ValueError("K must be a positive integer")
    if ann_queries is not None and type(ann_queries) is not list:
        raise ValueError("ann_queries must be a list when supplied")
    if quality_queries is not None and type(quality_queries) is not list:
        raise ValueError("quality_queries must be a list when supplied")

    integrated = deepcopy(report)
    if not ann_queries and not quality_queries:
        return integrated

    serialized = serialize_protocol_gate_metrics(ann_queries or [], quality_queries or [], k=k)
    available = {
        "ann_exact": ann_queries,
        "absolute_relevance": quality_queries,
    }
    for gate_name, queries in available.items():
        gate = integrated["gates"].get(gate_name)
        if not queries or type(gate) is not dict or gate.get("status") == "not_evaluated":
            continue
        gate["results"] = serialized[gate_name]
    return integrated


class Bridge(Protocol):
    def post(self, route: str, payload: dict) -> object: ...


def validate_endpoint(endpoint: str) -> str:
    parsed = urlsplit(endpoint)
    if (parsed.scheme != "http" or parsed.hostname not in {"127.0.0.1", "localhost", "::1"}
            or not parsed.port or parsed.path not in ("", "/") or parsed.query or parsed.fragment
            or parsed.username or parsed.password):
        raise ValueError("bridge endpoint must be an explicit HTTP loopback URL with a port")
    return endpoint.rstrip("/")


def runner_connection(endpoint: str, environment: dict[str, str]) -> tuple[str, str]:
    """Validate the isolated H5 runner route and load its staged bridge credential."""
    if (environment.get("MUNINN_H5_DISPOSABLE") != "muninn-h5-v1"
            or environment.get("MUNINN_H5_COMPOSE_PROJECT") != "muninn-h5-pilot-v1"):
        raise ValueError("missing H5 disposable Compose environment markers")
    internal = environment.get("MUNINN_H5_INTERNAL_RUNNER") == "muninn-h5-v1"
    if internal and endpoint == "http://bridge:8080":
        validated = endpoint
    else:
        validated = validate_endpoint(endpoint)
        if validated not in {"http://127.0.0.1:18080", "http://localhost:18080"}:
            raise ValueError("endpoint does not match the H5 loopback port")
    token_file = environment.get("MUNINN_BRIDGE_TOKEN_FILE")
    if token_file:
        token = Path(token_file).read_text(encoding="utf-8").strip()
    else:
        # Compatibility for the pre-container host invocation only.
        token = environment.get("MUNINN_BRIDGE_TOKEN", "")
    if not token:
        raise ValueError("MUNINN_BRIDGE_TOKEN_FILE is required")
    return validated, token


class HttpBridge:
    def __init__(self, endpoint: str, token: str, *, allow_internal: bool = False):
        if allow_internal and endpoint == "http://bridge:8080":
            self.endpoint = endpoint
        else:
            self.endpoint = validate_endpoint(endpoint)
        self.token = token
        # Explicitly disable environment proxy discovery; only loopback is allowed.
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def post(self, route: str, payload: dict) -> object:
        req = urllib.request.Request(
            self.endpoint + route, data=json.dumps(payload).encode(), method="POST",
            headers={"Content-Type": "application/json", "Authorization": f"Bearer {self.token}"},
        )
        try:
            with self.opener.open(req, timeout=30) as response:
                return json.loads(response.read())
        except (urllib.error.URLError, json.JSONDecodeError) as error:
            raise RuntimeError(f"bridge request failed: {route}") from error


def sanitize_diagnostic(response: object, allowed: set[str]) -> dict:
    """Validate projected aliases and the exact encoder identity in one response."""
    if type(response) is not dict or set(response) != {"ann", "exact", "encoder_identity"}:
        raise ValueError("bridge returned malformed H5 diagnostic response")
    safe = {"encoder_identity": sanitize_encoder_identity(response["encoder_identity"])}
    for method in ("ann", "exact"):
        rows = response[method]
        if type(rows) is not list:
            raise ValueError("bridge returned malformed H5 diagnostic rankings")
        sanitized = []
        seen = set()
        for rank, row in enumerate(rows, 1):
            if (type(row) is not dict or set(row) != {"alias", "score", "rank"}
                    or type(row["alias"]) is not str or type(row["rank"]) is not int
                    or row["rank"] != rank or isinstance(row["score"], bool)
                    or not isinstance(row["score"], (int, float))
                    or not math.isfinite(row["score"])):
                raise ValueError("bridge returned malformed H5 diagnostic result")
            alias = row["alias"]
            if alias not in allowed or alias in seen:
                raise ValueError("H5 diagnostic returned an unknown or duplicate candidate")
            seen.add(alias)
            sanitized.append({"id": alias, "score": row["score"], "rank": rank})
        safe[method] = sanitized
    return safe


def run_evaluation(*, corpus: object, annotation_csv: str, raw_csv_sha256: str,
                   bridge_identity: str, model_bundle_id: str, source_build_identity: str,
                   bridge: Bridge, limit: int, threshold: float, host_facts: dict | None = None) -> dict:
    # Complete all local input checks before the first request.
    corpus = validate_corpus(corpus)
    annotations = annotations_from_csv(annotation_csv, corpus)
    actual_csv_hash = hashlib.sha256(annotation_csv.encode("utf-8")).hexdigest()
    if not re.fullmatch(r"[0-9a-f]{64}", raw_csv_sha256) or actual_csv_hash != raw_csv_sha256:
        raise ValueError("raw annotation CSV SHA-256 does not match supplied provenance")
    for label, value in (("bridge identity", bridge_identity), ("model bundle ID", model_bundle_id),
                         ("source/build identity", source_build_identity)):
        if type(value) is not str or not value.strip():
            raise ValueError(f"{label} is required")
    if type(limit) is not int or not 1 <= limit <= 100:
        raise ValueError("search limit must be between 1 and 100")
    if isinstance(threshold, bool) or not isinstance(threshold, (float, int)) or not 0 <= threshold <= 1:
        raise ValueError("search threshold must be between 0 and 1")

    # ANN applies top-k globally before the bridge applies its session filter. Request
    # enough rows to cover every pilot candidate, while retaining the requested metric Ks.
    candidate_count = sum(len(query["candidates"]) for query in corpus["queries"])
    search_limit = max(limit, candidate_count)
    if search_limit > 100:
        raise ValueError("pilot candidate count exceeds the bridge's maximum search limit")

    uuid_to_id: dict[str, str] = {}
    by_query: dict[str, list[dict]] = {}
    for query in corpus["queries"]:
        session = f"{SESSION_PREFIX}-{query['id']}"
        for candidate in query["candidates"]:
            response = bridge.post("/add_message", {
                "session_id": session, "role": "assistant", "content": candidate["text"],
                "metadata": {"h5_corpus_id": corpus["corpus_id"], "h5_candidate_id": candidate["id"]},
            })
            if type(response) is not dict or type(response.get("id")) is not str or not response["id"]:
                raise ValueError("bridge returned malformed add_message response")
            raw_id = response["id"]
            if raw_id in uuid_to_id:
                raise ValueError("duplicate raw candidate ID")
            uuid_to_id[raw_id] = candidate["id"]

    grades_by_query = {j["query_id"]: {g["document_id"]: g["grade"] for g in j["grades"]}
                       for j in annotations["judgments"]}
    for query in corpus["queries"]:
        allowed = {candidate["id"] for candidate in query["candidates"]}
        query_aliases = {raw_id: alias for raw_id, alias in uuid_to_id.items() if alias in allowed}
        response = bridge.post("/h5/diagnostic/search", {
            "query": query["text"], "session_id": f"{SESSION_PREFIX}-{query['id']}",
            "limit": search_limit, "threshold": float(threshold),
            "candidate_aliases": query_aliases,
        })
        diagnostic = sanitize_diagnostic(response, allowed)
        ranked = []
        for hit in diagnostic["ann"]:
            ranked.append({"id": hit["id"], "score": hit["score"]})
        by_query[query["id"]] = {"id": query["id"], "candidates": grades_by_query[query["id"]], "ranked": ranked}
        by_query[query["id"]]["diagnostic"] = diagnostic
        # The bridge response is self-report only. This runner path does not have
        # independent protocol identity and Docker/run observations to correlate it.
        by_query[query["id"]]["identity_provenance"] = {
            "status": "partial",
            "reason": "expected_protocol_identity_and_external_execution_observations_missing",
            "query_id": query["id"],
            "response_identity": diagnostic["encoder_identity"],
            "attestation": False,
        }
    provenance = {
        "corpus_id": corpus["corpus_id"], "corpus_hash": corpus_hash(corpus),
        "judgment_hash": "sha256:" + actual_csv_hash, "model_bundle": model_bundle_id,
        "runtime_build": f"{source_build_identity};bridge={bridge_identity}",
        "search_config": {"limit": search_limit, "metric_ks": list(KS),
                          "threshold": float(threshold), "session_scope": "query-specific"},
        "run_timestamp": datetime.now(timezone.utc).isoformat(),
    }
    report = compose_report(list(by_query.values()), KS, provenance)
    report["ann_exact_diagnostic"] = {
        "queries": [{"query_id": item["id"], **item["diagnostic"],
                     "identity_provenance": item["identity_provenance"]}
                    for item in by_query.values()],
        "scope": "same_query_owner_and_session",
        "acceptance": "descriptive_only",
    }
    for item in by_query.values():
        del item["diagnostic"]
    report["annotation_provenance"] = {
        "raw_csv_sha256": actual_csv_hash, "rubric_version": annotations["rubric_version"],
        "reviewer_id": annotations["reviewer_id"], "adjudication": annotations["adjudication"],
    }
    report["host_runtime"] = host_facts or {"platform": platform.platform(), "python": platform.python_version()}
    source_files = (Path(__file__), Path(__file__).with_name("retrieval_quality.py"))
    source_digest = hashlib.sha256()
    for source_file in source_files:
        source_digest.update(source_file.name.encode("utf-8") + b"\0")
        source_digest.update(source_file.read_bytes())
    report["provenance"]["current_source"] = "sha256:" + source_digest.hexdigest()
    report["provenance"]["identity_verification"] = {
        "bridge_identity": "caller_supplied_not_verified",
        "source_build_identity": "externally_supplied_input",
    }
    return report


def _decision_identifier(value: object) -> bool:
    return type(value) is str and bool(value.strip()) and not _is_placeholder(value)


def _search_config_valid(value: object) -> bool:
    if type(value) is not dict or set(value) != {"limit", "threshold"}:
        return False
    limit, threshold = value["limit"], value["threshold"]
    return (type(limit) in (int, float) and 10 <= limit <= 100
            and limit == int(limit) and type(threshold) in (int, float)
            and 0 <= threshold <= 1)


def _validate_protocol_run_inputs(*, protocol: object, corpus: object,
                                  operator_record: object, run_id: str,
                                  search_config_override: object = None) -> dict:
    """Pure local U9 boundary, not evidence acquisition or authorization to execute.

    Returned working copies/pairs are internal only. The future public orchestrator
    must still check local observations and publish only the safe context.
    """
    findings = []
    copies = {}
    for name, item in (("protocol", protocol), ("corpus", corpus), ("operator", operator_record)):
        try:
            copies[name] = deepcopy(item)
        except RecursionError:  # Preserve fail-closed behavior without imposing a new depth limit.
            copies[name] = None
            copies[name + "_failed"] = True
    protocol, corpus, operator_record = copies["protocol"], copies["corpus"], copies["operator"]
    context = {"sha256": None, "version": None, "schema_version": None,
               "confirmatory_ready": False, "operator_corroborated": False,
               "findings": {name: [] for name in ("missing", "pending", "malformed")}}
    result = {"protocol": context, "corpus": {"id": None, "sha256": None, "version": None},
              "run_id": run_id if _decision_identifier(run_id) else None,
              "findings": findings, "pairs": []}

    def reject(path, reason):
        findings.append({"path": path, "reason": reason})

    def finish():
        findings[:] = [dict(path=path, reason=reason) for path, reason in sorted(
            {(item["path"], item["reason"]) for item in findings})]
        return result

    if result["run_id"] is None:
        reject("run_id", "preflight_evidence_missing_or_conflicting")
    try:
        if copies.get("protocol_failed"):
            raise ValueError("protocol could not be copied")
        validated = validate_protocol(protocol)
    except (ValueError, TypeError, OverflowError, RecursionError):
        reject("protocol", "protocol_invalid")
        return finish()
    context.update(sha256=validated["sha256"], version=protocol["protocol_version"],
                   schema_version=protocol["schema_version"],
                   confirmatory_ready=validated["confirmatory_ready"],
                   findings={name: validated[name] for name in ("missing", "pending", "malformed")})
    if protocol["schema_version"] != "h5-acceptance/v2":
        reject("protocol.schema_version", "contract_unsupported")
    if protocol["gates"] != {"ann_exact": True, "absolute_relevance": False}:
        reject("protocol.gates", "contract_unsupported")
    if not validated["confirmatory_ready"]:
        reject("protocol", "protocol_invalid")
        return finish()
    if findings:
        return finish()
    try:
        context["operator_corroborated"] = not copies.get("operator_failed") and corroborate_approval(
            protocol, expected_sha256=validated["sha256"], operator_record=operator_record)
    except RecursionError:
        context["operator_corroborated"] = False
    if not context["operator_corroborated"]:
        reject("operator_record", "approval_mismatch")
        return finish()
    try:
        if copies.get("corpus_failed"):
            raise ValueError("corpus could not be copied")
        validate_corpus(corpus)
        digest = corpus_hash(corpus)
    except (ValueError, TypeError, OverflowError, RecursionError):
        reject("corpus", "corpus_mismatch")
        return finish()
    scope = protocol["run_inputs"]["corpus_and_query_candidate_scope"]["value"]
    result["corpus"].update(id=corpus["corpus_id"], sha256=digest, version=scope["version"])
    pairs = {(q["id"], c["id"]) for q in corpus["queries"] for c in q["candidates"]}
    scope_pairs = [(q["query_id"], cid) for q in scope["queries"] for cid in q["candidate_ids"]]
    query_ids = [q["query_id"] for q in scope["queries"]]
    if (scope["corpus_id"] != corpus["corpus_id"] or scope["corpus_hash"] != digest
            or len(query_ids) != len(set(query_ids)) or len(scope_pairs) != len(set(scope_pairs))
            or set(scope_pairs) != pairs or any(len(q["candidates"]) < 10 for q in corpus["queries"])):
        reject("protocol.run_inputs.corpus_and_query_candidate_scope.value", "corpus_mismatch")
    config = protocol["run_inputs"]["ann_exact_comparison_scope"]["value"]["search_config"]
    if len(pairs) > config["limit"]:
        reject("protocol.run_inputs.ann_exact_comparison_scope.value.search_config.limit", "scope_unsupported")
    if search_config_override is not None and (
            not _search_config_valid(search_config_override) or search_config_override != config):
        reject("search_config_override", "configuration_mismatch")
    if not findings:
        result.update(pairs=sorted(pairs), working_protocol=protocol, working_corpus=corpus,
                      operator_record=operator_record)
    return finish()


class ProtocolRunEvidence(Protocol):
    def before_run(self, *, protocol: dict, corpus: dict, run_id: str) -> LocalObservations: ...
    def after_response(self, *, run_id: str, query_id: str, diagnostic: dict) -> LocalObservations: ...
    def after_run(self, *, run_id: str) -> LocalObservations: ...


def _closed_object(value: object, keys: set[str]) -> bool:
    return type(value) is dict and set(value) == keys


def _protocol_evidence_findings(local: dict, observations: LocalObservations) -> list[dict]:
    """Compare acquired records; never repair observations with expectations."""
    findings = []
    missing = "preflight_evidence_missing_or_conflicting"

    def reject(path, reason=missing):
        findings.append({"path": "observations." + path, "reason": reason})

    for key in assess_preflight_provenance("ann_exact", observations)["missing_or_conflicting"]:
        reject(key)
    keys = {
        "corpus_freeze_review": {"corpus_id", "version", "corpus_hash", "status", "review_reference"},
        "storage_scope": {"run_id", "bridge_container_id", "storage_id", "isolation",
                          "observed_candidates", "planned_candidates"},
        "effective_config": {"run_id", "retrieval_path", "candidate_scope", "search_config"},
        "operator_corroboration": {"protocol_sha256", "commit", "review_reference",
                                   "corroborated_at", "corroborated_by"},
    }
    valid = {}
    for name, fields in keys.items():
        item = observations.get(name)
        if (type(item) is not dict or set(item) not in (
                {"value", "source", "method", "observed_at"},
                {"value", "source", "method", "observed_at", "conflict"})
                or any(type(item.get(k)) is not str or not item[k].strip()
                       for k in ("source", "method", "observed_at"))
                or ("conflict" in item and (type(item["conflict"]) is not bool or item["conflict"]))):
            reject(name)
            continue
        value = item["value"]
        if not _closed_object(value, fields):
            reject(name + ".value")
            continue
        strings = fields - {"observed_candidates", "planned_candidates", "search_config"}
        if any(not _decision_identifier(value[k]) for k in strings):
            reject(name + ".value")
            continue
        if name == "corpus_freeze_review" and not re.fullmatch(r"sha256:[0-9a-f]{64}", value["corpus_hash"]):
            reject(name + ".value.corpus_hash")
            continue
        valid[name] = value
    inputs = local["working_protocol"]["run_inputs"]
    scope = inputs["corpus_and_query_candidate_scope"]["value"]
    approved = inputs["ann_exact_comparison_scope"]["value"]

    def observed_value(name):
        item = observations.get(name)
        return item.get("value") if type(item) is dict else None

    bindings = {"protocol_hash": (local["protocol"]["sha256"], "approval_mismatch"),
                "review_reference": (local["operator_record"]["review_reference"], "approval_mismatch"),
                "corpus_hash": (local["corpus"]["sha256"], "corpus_mismatch"),
                "run_id": (local["run_id"], missing)}
    for name, (expected, reason) in bindings.items():
        if observed_value(name) != expected:
            reject(name + ".value", reason)
    try:
        identity = sanitize_encoder_identity(observed_value("encoder_expected_identity"))
    except ValueError:
        identity = None
    if identity != inputs["encoder_identity"]["value"]:
        reject("encoder_expected_identity.value")
    freeze = valid.get("corpus_freeze_review")
    if freeze:
        for key in ("corpus_id", "version", "corpus_hash", "status"):
            if freeze[key] != ("frozen" if key == "status" else scope[key]):
                reject("corpus_freeze_review.value." + key, "corpus_mismatch")
    operator = valid.get("operator_corroboration")
    if operator and operator != local["operator_record"]:
        reject("operator_corroboration.value", "approval_mismatch")
    storage = valid.get("storage_scope")
    if storage:
        for key, expected in (("run_id", local["run_id"]),
                              ("bridge_container_id", observed_value("bridge_container_id")),
                              ("isolation", "run-exclusive-disposable-v1"), ("observed_candidates", [])):
            if type(storage[key]) is not type(expected) or storage[key] != expected:
                reject("storage_scope.value." + key, "scope_unsupported")
        if any(c in storage["storage_id"] for c in ("/", "\\")) or storage["storage_id"].startswith("~"):
            reject("storage_scope.value.storage_id", "scope_unsupported")
        plan = storage["planned_candidates"]
        if (type(plan) is not list or not plan or any(
                not _closed_object(pair, {"query_id", "candidate_id"})
                or any(not _decision_identifier(v) for v in pair.values()) for pair in plan)):
            reject("storage_scope.value.planned_candidates", "scope_unsupported")
        else:
            pairs = [(p["query_id"], p["candidate_id"]) for p in plan]
            if len(pairs) != len(set(pairs)) or set(pairs) != set(local["pairs"]):
                reject("storage_scope.value.planned_candidates", "scope_unsupported")
    config = valid.get("effective_config")
    if config:
        for key in ("run_id", "retrieval_path", "candidate_scope", "search_config"):
            expected = local["run_id"] if key == "run_id" else approved[key]
            if (key == "search_config" and not _search_config_valid(config[key])) or config[key] != expected:
                reject("effective_config.value." + key, "configuration_mismatch")
    return findings


def _session_id(run_id: str, query_id: str) -> str:
    return f"muninn-h5-v2:{len(run_id)}:{run_id}:{len(query_id)}:{query_id}"


def run_protocol_evaluation(*, protocol: object, corpus: object, operator_record: object,
                            run_id: str, bridge: Bridge, evidence: ProtocolRunEvidence,
                            search_config_override: dict | None = None) -> dict:
    """U10 isolated execution with per-query candidate mappings and bindings."""
    local = _validate_protocol_run_inputs(protocol=protocol, corpus=corpus,
        operator_record=operator_record, run_id=run_id, search_config_override=search_config_override)
    findings = local["findings"]
    if not findings:
        try:
            observations = evidence.before_run(protocol=deepcopy(local["working_protocol"]),
                corpus=deepcopy(local["working_corpus"]), run_id=local["run_id"])
        except (OSError, ValueError, RuntimeError, TypeError):
            # Provider contract violations fail closed; exception text is never public.
            observations = None
        fixed = {"path": "observations", "reason": "preflight_evidence_missing_or_conflicting"}
        if not isinstance(observations, LocalObservations):
            findings.append(fixed)
        else:
            try:
                findings.extend(_protocol_evidence_findings(local, deepcopy(observations)))
            except RecursionError:
                findings[:] = [fixed]
    # Unknown protocol properties are reported at their container, never with caller-chosen names.
    context = deepcopy(local["protocol"])
    parents = {}

    def collect_paths(value, path=""):
        if type(value) is dict:
            for key, item in value.items():
                if type(key) is str:
                    child = path + "." + key if path else key
                    parents.setdefault(child, []).append(path)
                    collect_paths(item, child)
        elif type(value) is list:
            for index, item in enumerate(value):
                collect_paths(item, f"{path}[{index}]")

    try:
        collect_paths(protocol)
    except RecursionError:
        parents.clear()
    for group in context["findings"].values():
        for finding in group:
            if finding["reason"] == "unknown property":
                finding["path"] = min(parents.get(finding["path"], ["protocol"]), key=len)
    findings = [dict(path=path, reason=reason) for path, reason in sorted(
        {(f["path"], f["reason"]) for f in findings})]
    blocked = bool(findings)
    if blocked:
        return {"preflight": {"status": "blocked", "findings": findings},
                "gates": {"ann_exact": {"status": "blocked", "results": {}},
                          "absolute_relevance": {"status": "not_evaluated", "results": {}}},
                "measurement_started": False, "protocol": context,
                "corpus": deepcopy(local["corpus"]), "run_id": local["run_id"]}

    preflight_report = {"status": "eligible", "findings": []}
    measurement_started = True
    absolute_relevance_gate = {"status": "not_evaluated", "results": {}}

    try:
        seen_uuids: set[str] = set()
        query_candidate_to_uuid: dict[str, dict[str, str]] = {}
        query_uuid_to_candidate: dict[str, dict[str, str]] = {}

        # 1. Candidate insertion per query
        for query in local["working_corpus"]["queries"]:
            q_id = query["id"]
            session = _session_id(local["run_id"], q_id)
            c_to_u: dict[str, str] = {}
            u_to_c: dict[str, str] = {}
            for candidate in query["candidates"]:
                c_id = candidate["id"]
                resp = bridge.post("/add_message", {
                    "session_id": session,
                    "role": "assistant",
                    "content": candidate["text"],
                    "metadata": {
                        "h5_corpus_id": local["working_corpus"]["corpus_id"],
                        "h5_candidate_id": c_id,
                    },
                })
                if type(resp) is not dict or type(resp.get("id")) is not str or not resp["id"]:
                    raise ValueError("invalid add_message response")
                raw_uuid = resp["id"]
                if raw_uuid in seen_uuids:
                    raise ValueError("duplicate candidate uuid across run")
                seen_uuids.add(raw_uuid)
                c_to_u[c_id] = raw_uuid
                u_to_c[raw_uuid] = c_id
            query_candidate_to_uuid[q_id] = c_to_u
            query_uuid_to_candidate[q_id] = u_to_c

        # 2. Search execution per query
        search_config = (
            local["working_protocol"]["run_inputs"]["ann_exact_comparison_scope"]["value"]["search_config"]
        )
        search_limit = int(search_config["limit"])
        search_threshold = float(search_config["threshold"])

        ann_queries = []
        diagnostics = {}
        query_evidence_map = {}

        for query in local["working_corpus"]["queries"]:
            q_id = query["id"]
            session = _session_id(local["run_id"], q_id)
            allowed_cids = {c["id"] for c in query["candidates"]}
            aliases = query_uuid_to_candidate[q_id]
            resp = bridge.post("/h5/diagnostic/search", {
                "query": query["text"],
                "session_id": session,
                "limit": search_limit,
                "threshold": search_threshold,
                "candidate_aliases": aliases,
            })
            diagnostic = sanitize_diagnostic(resp, allowed_cids)
            diagnostics[q_id] = diagnostic
            ann_queries.append({
                "query_id": q_id,
                "ann": diagnostic["ann"],
                "exact": diagnostic["exact"],
            })
            query_evidence_map[q_id] = evidence.after_response(
                run_id=local["run_id"],
                query_id=q_id,
                diagnostic=diagnostic,
            )

        post_run_obs = evidence.after_run(run_id=local["run_id"])

    except Exception:
        return {
            "preflight": preflight_report,
            "gates": {
                "ann_exact": {
                    "status": "blocked",
                    "findings": [{"path": "bridge_response", "reason": "response_invalid"}],
                    "results": {},
                },
                "absolute_relevance": absolute_relevance_gate,
            },
            "measurement_started": True,
            "protocol": context,
            "corpus": deepcopy(local["corpus"]),
            "run_id": local["run_id"],
        }

    post_findings = []
    try:
        re_proto = validate_protocol(protocol)
        if re_proto["sha256"] != local["protocol"]["sha256"]:
            post_findings.append({"path": "protocol", "reason": "approval_mismatch"})
    except Exception:
        post_findings.append({"path": "protocol", "reason": "approval_mismatch"})

    try:
        re_corpus_hash = corpus_hash(corpus)
        if re_corpus_hash != local["corpus"]["sha256"]:
            post_findings.append({"path": "corpus", "reason": "corpus_mismatch"})
    except Exception:
        post_findings.append({"path": "corpus", "reason": "corpus_mismatch"})

    expected_qids = [q["id"] for q in local["working_corpus"]["queries"]]
    diagnostics_complete = (set(diagnostics) == set(expected_qids)
                            and len(diagnostics) == len(expected_qids))
    evidence_complete = (set(query_evidence_map) == set(expected_qids)
                         and len(query_evidence_map) == len(expected_qids)
                         and all(isinstance(query_evidence_map[qid], LocalObservations)
                                 for qid in expected_qids))
    coverage_complete = diagnostics_complete and evidence_complete
    if not coverage_complete:
        post_findings.append({"path": "coverage", "reason": "coverage_incomplete"})

    serialized = serialize_protocol_gate_metrics(ann_queries, [], k=10)
    per_query = serialized["ann_exact"]["per_query"]

    recalls = [row["metrics"]["exact_top_k_recall"] for row in per_query]
    matches = [row["metrics"]["ordered_top_k_match"] for row in per_query]

    undef_recalls = sum(1 for v in recalls if v is None)
    undef_matches = sum(1 for v in matches if v is None)

    macro_recall = (sum(recalls) / len(recalls)) if (undef_recalls == 0 and len(recalls) > 0) else None
    macro_match = (sum(matches) / len(matches)) if (undef_matches == 0 and len(matches) > 0) else None

    macro_descriptive = {
        "exact_top_k_recall": {
            "value": macro_recall,
            "undefined_count": undef_recalls,
        },
        "ordered_top_k_match": {
            "value": macro_match,
            "undefined_count": undef_matches,
        },
    }

    if undef_recalls > 0 or undef_matches > 0:
        post_findings.append({"path": "gates.ann_exact", "reason": "required_metric_undefined"})

    provenance_verified = True
    base_obs = dict(observations) if isinstance(observations, dict) else {}
    if isinstance(post_run_obs, dict):
        base_obs.update(post_run_obs)

    expected_identity = local["working_protocol"]["run_inputs"]["encoder_identity"]["value"]

    for q_id in expected_qids:
        q_obs = dict(base_obs)
        if isinstance(query_evidence_map.get(q_id), dict):
            q_obs.update(query_evidence_map[q_id])

        prov_assessment = assess_gate_provenance("ann_exact", q_obs)
        if prov_assessment["status"] != "verified":
            provenance_verified = False
            for item in prov_assessment["missing_or_conflicting"]:
                post_findings.append({"path": f"observations.{item}", "reason": "provenance_incomplete"})

        if q_obs.get("run_id", {}).get("value") != local["run_id"]:
            provenance_verified = False
            post_findings.append({"path": "observations.run_id", "reason": "response_binding_mismatch"})
        if q_obs.get("protocol_hash", {}).get("value") != local["protocol"]["sha256"]:
            provenance_verified = False
            post_findings.append({"path": "observations.protocol_hash", "reason": "approval_mismatch"})
        if q_obs.get("corpus_hash", {}).get("value") != local["corpus"]["sha256"]:
            provenance_verified = False
            post_findings.append({"path": "observations.corpus_hash", "reason": "corpus_mismatch"})

        binding_val = q_obs.get("encoder_response_binding", {}).get("value")
        if (type(binding_val) is not dict
                or binding_val.get("response_run_id") != local["run_id"]
                or binding_val.get("response_query_id") != q_id
                or binding_val.get("expected_query_id") != q_id):
            provenance_verified = False
            post_findings.append({"path": "observations.encoder_response_binding", "reason": "response_binding_mismatch"})

        resp_ident = q_obs.get("encoder_response_identity", {}).get("value")
        if resp_ident != expected_identity:
            provenance_verified = False
            post_findings.append({"path": "observations.encoder_response_identity", "reason": "response_binding_mismatch"})

    if not coverage_complete or undef_recalls > 0 or undef_matches > 0:
        rule_satisfied = None
    elif all(r == 1.0 for r in recalls) and all(m == 1 for m in matches):
        rule_satisfied = True
    else:
        rule_satisfied = False

    provenance_status = "verified" if (provenance_verified and not post_findings) else "insufficient"
    gate_status = gate_result(
        protocol_complete=True,
        approved=True,
        provenance_status=provenance_status,
        rule_satisfied=rule_satisfied,
    )

    ann_exact_gate = {
        "status": gate_status,
        "results": {
            "k": 10,
            "per_query": per_query,
            "macro_descriptive": macro_descriptive,
        },
    }
    if post_findings:
        unique_findings = [dict(path=path, reason=reason) for path, reason in sorted(
            {(f["path"], f["reason"]) for f in post_findings})]
        ann_exact_gate["findings"] = unique_findings

    return {
        "preflight": preflight_report,
        "gates": {
            "ann_exact": ann_exact_gate,
            "absolute_relevance": absolute_relevance_gate,
        },
        "measurement_started": True,
        "protocol": context,
        "corpus": deepcopy(local["corpus"]),
        "run_id": local["run_id"],
    }


def protocol_preflight(protocol: object, bridge: Bridge | None = None, *,
                       provenance_observations: dict | None = None, run_id: str | None = None,
                       historical_report: dict | None = None,
                       operator_record: object = None) -> dict:
    """Build a per-gate preflight report without bridge calls or historical mutation.

    ``historical_report`` is accepted for compatibility/immutability checks and deliberately
    excluded from the new report. Only local observation records count as provenance.
    """
    del bridge, historical_report
    validated = validate_protocol(protocol)
    operator_corroborated = corroborate_approval(
        protocol, expected_sha256=validated["sha256"], operator_record=operator_record)
    observations = (provenance_observations
                   if isinstance(provenance_observations, LocalObservations) else {})
    if observations:
        observations = LocalObservations(observations)
    identity_decision = protocol["run_inputs"].get("encoder_identity")
    expected_encoder_identity = (
        identity_decision.get("value")
        if type(identity_decision) is dict and identity_decision.get("status") == "approved"
        else None
    )
    if expected_encoder_identity is not None:
        expected_record = {
            "value": expected_encoder_identity,
            "source": "validated protocol document",
            "method": "protocol identity field",
            "observed_at": protocol["repository_review"]["approved_at"],
        }
        supplied_record = observations.get("encoder_expected_identity")
        if type(supplied_record) is dict and supplied_record.get("value") != expected_encoder_identity:
            expected_record["conflict"] = True
        observations["encoder_expected_identity"] = expected_record
        expected_values = {"protocol_hash": validated["sha256"], "run_id": run_id}
        for key, expected in expected_values.items():
            if expected is not None and key in observations:
                item = observations[key]
                if type(item) is not dict or item.get("value") != expected:
                    observations[key] = dict(item, conflict=True) if type(item) is dict else {
                        "value": None, "source": "invalid local observation", "method": "validation",
                        "observed_at": "", "conflict": True,
                    }
    gates = {}
    for gate, enabled in validated["gates"].items():
        if not enabled:
            gates[gate] = {"status": "not_evaluated", "provenance": {"status": "insufficient",
                              "observations": {}, "missing_or_conflicting": []}, "results": {}}
            continue
        provenance = assess_gate_provenance(gate, observations)
        protocol_missing = list(validated["missing_fields"])
        approved = operator_corroborated and gate in protocol["repository_review"]["approved_gates"]
        protocol_complete = validated["confirmatory_ready"] and not protocol_missing
        # No criterion is evaluated by preflight. Undefined/missing treatment blocks eligibility.
        status = gate_result(protocol_complete=protocol_complete, approved=approved,
                             provenance_status=provenance["status"], rule_satisfied=None)
        gates[gate] = {
            "status": status,
            "protocol": {"complete": protocol_complete, "approved": approved,
                         "missing_fields": protocol_missing,
                         "findings": {name: list(validated[name]) for name in ("missing", "pending", "malformed")}},
            "provenance": provenance,
            "results": {"status": "not_evaluated", "reason": "preflight does not run measurements"},
        }
    protocol_report = {"sha256": validated["sha256"], "version": protocol["protocol_version"],
                       "commit": protocol["repository_review"]["commit"],
                       "review_reference": protocol["repository_review"]["review_reference"],
                       "operator_corroborated": operator_corroborated,
                       "confirmatory_ready": validated["confirmatory_ready"],
                       "findings": {name: list(validated[name]) for name in ("missing", "pending", "malformed")}}
    if run_id is not None:
        protocol_report["run_id"] = run_id
    return {"protocol": protocol_report, "gates": gates, "measurement_started": False}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--annotations", type=Path, required=True)
    parser.add_argument("--raw-csv-sha256", required=True)
    parser.add_argument("--bridge-identity", required=True)
    parser.add_argument("--model-bundle-id", required=True)
    parser.add_argument("--source-build-identity", required=True)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--limit", type=int, required=True)
    parser.add_argument("--threshold", type=float, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        endpoint, token = runner_connection(args.endpoint, os.environ)
        report = run_evaluation(
            corpus=json.loads(args.corpus.read_text()),
            annotation_csv=args.annotations.read_bytes().decode("utf-8"),
            raw_csv_sha256=args.raw_csv_sha256, bridge_identity=args.bridge_identity,
            model_bundle_id=args.model_bundle_id, source_build_identity=args.source_build_identity,
            bridge=HttpBridge(
                endpoint, token,
                allow_internal=os.environ.get("MUNINN_H5_INTERNAL_RUNNER") == "muninn-h5-v1",
            ),
            limit=args.limit, threshold=args.threshold,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"H5 runner refused: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
