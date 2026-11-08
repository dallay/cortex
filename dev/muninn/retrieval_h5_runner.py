"""One-shot evaluator for the isolated Muninn H5 retrieval pilot."""
from __future__ import annotations

import argparse
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

from retrieval_quality import annotations_from_csv, compose_report, corpus_hash, validate_corpus

KS = (1, 3, 5)
SESSION_PREFIX = "muninn-h5-retrieval-pilot-v1"


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
    """Validate server-projected corpus aliases and discard all other fields."""
    if type(response) is not dict or set(response) != {"ann", "exact"}:
        raise ValueError("bridge returned malformed H5 diagnostic response")
    safe = {}
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
        "queries": [{"query_id": item["id"], **item["diagnostic"]}
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
