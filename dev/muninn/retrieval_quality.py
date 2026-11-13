"""Pure retrieval ranking and quality metrics for judged candidate sets."""

from __future__ import annotations

import math
import hashlib
import csv
import io
import json
import re
from copy import deepcopy
from datetime import datetime
from typing import TypedDict


class RankedResult(TypedDict):
    id: str
    score: int | float


def _require_exact_keys(value: object, keys: set[str], name: str) -> dict[str, object]:
    if type(value) is not dict or set(value) != keys:
        raise ValueError(f"{name} must contain exactly: {', '.join(sorted(keys))}")
    return value


def _require_identifier(value: object, name: str) -> str:
    if type(value) is not str or not value.strip():
        raise ValueError(f"{name} must be a non-empty string")
    return value


def validate_corpus(corpus: object) -> dict[str, object]:
    """Validate the strict unlabelled evaluation-corpus schema without changing text."""
    corpus = _require_exact_keys(corpus, {"schema_version", "corpus_id", "queries"}, "corpus")
    if corpus["schema_version"] != "1.0":
        raise ValueError("unsupported corpus schema_version")
    _require_identifier(corpus["corpus_id"], "corpus_id")
    queries = corpus["queries"]
    if type(queries) is not list or not queries:
        raise ValueError("queries must be a non-empty list")
    query_ids: set[str] = set()
    document_texts: dict[str, str] = {}
    for query in queries:
        query = _require_exact_keys(query, {"id", "text", "candidates"}, "query")
        query_id = _require_identifier(query["id"], "query ID")
        if query_id in query_ids:
            raise ValueError(f"duplicate query ID: {query_id}")
        query_ids.add(query_id)
        if type(query["text"]) is not str or not query["text"].strip():
            raise ValueError(f"query text must be non-empty: {query_id}")
        candidates = query["candidates"]
        if type(candidates) is not list or not candidates:
            raise ValueError(f"candidates must be a non-empty list: {query_id}")
        local_ids: set[str] = set()
        for candidate in candidates:
            candidate = _require_exact_keys(candidate, {"id", "text"}, "candidate")
            document_id = _require_identifier(candidate["id"], "document ID")
            if document_id in local_ids:
                raise ValueError(f"duplicate document ID: {document_id}")
            local_ids.add(document_id)
            if type(candidate["text"]) is not str or not candidate["text"].strip():
                raise ValueError(f"document text must be non-empty: {document_id}")
            prior_text = document_texts.get(document_id)
            if prior_text is not None and prior_text != candidate["text"]:
                raise ValueError(f"document ID has inconsistent text: {document_id}")
            document_texts[document_id] = candidate["text"]
    return corpus


def corpus_hash(corpus: object) -> str:
    """Return the deterministic SHA-256 identity of a validated corpus."""
    validated = validate_corpus(corpus)
    canonical = json.dumps(validated, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return "sha256:" + hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def validate_annotations(annotations: object, corpus: object) -> dict[str, object]:
    """Validate complete human judgments against one exact corpus identity."""
    corpus = validate_corpus(corpus)
    annotations = _require_exact_keys(
        annotations,
        {"schema_version", "corpus_id", "corpus_hash", "reviewer_id", "rubric_version",
         "adjudication", "judgments"},
        "annotations",
    )
    if annotations["schema_version"] != "1.0":
        raise ValueError("unsupported annotations schema_version")
    if annotations["corpus_id"] != corpus["corpus_id"]:
        raise ValueError("annotation corpus_id does not match corpus")
    if annotations["corpus_hash"] != corpus_hash(corpus):
        raise ValueError("annotation corpus_hash does not match corpus")
    reviewer_id = annotations["reviewer_id"]
    if type(reviewer_id) is not str or not re.fullmatch(r"reviewer-[a-z0-9]{4,32}", reviewer_id):
        raise ValueError("reviewer_id must be a pseudonymous reviewer-* identifier")
    if type(annotations["rubric_version"]) is not str or annotations["rubric_version"] != "muninn-retrieval-v1.0":
        raise ValueError("unsupported rubric_version")
    adjudication = _require_exact_keys(
        annotations["adjudication"], {"status", "reviewer_count"}, "adjudication"
    )
    if (type(adjudication["status"]) is not str
            or adjudication["status"] != "single_reviewer_not_adjudicated"
            or type(adjudication["reviewer_count"]) is not int
            or adjudication["reviewer_count"] != 1):
        raise ValueError("adjudication metadata must identify the single-reviewer status")
    judgments = annotations["judgments"]
    if type(judgments) is not list:
        raise ValueError("judgments must be a list")
    expected = {query["id"]: {candidate["id"] for candidate in query["candidates"]}
                for query in corpus["queries"]}
    seen_queries: set[str] = set()
    for judgment in judgments:
        judgment = _require_exact_keys(judgment, {"query_id", "grades"}, "judgment")
        query_id = _require_identifier(judgment["query_id"], "judgment query_id")
        if query_id not in expected or query_id in seen_queries:
            raise ValueError(f"unknown or duplicate judgment query_id: {query_id}")
        seen_queries.add(query_id)
        grades = judgment["grades"]
        if type(grades) is not list:
            raise ValueError("grades must be a list")
        seen_documents: set[str] = set()
        for item in grades:
            item = _require_exact_keys(item, {"document_id", "grade"}, "grade")
            document_id = _require_identifier(item["document_id"], "judgment document_id")
            if document_id not in expected[query_id] or document_id in seen_documents:
                raise ValueError(f"unknown or duplicate candidate judgment: {document_id}")
            seen_documents.add(document_id)
            if type(item["grade"]) is not int or item["grade"] not in (0, 1, 2, 3):
                raise ValueError("grades must be integers from 0 through 3")
        if seen_documents != expected[query_id]:
            raise ValueError(f"judgments must cover every candidate for query: {query_id}")
    if seen_queries != set(expected):
        raise ValueError("judgments must cover every corpus query")
    return annotations


_ANNOTATION_CSV_COLUMNS = (
    "corpus_id", "corpus_hash", "reviewer_id", "rubric_version", "adjudication_status",
    "query_id", "query_text", "candidate_id", "candidate_text", "grade",
)


def annotations_from_csv(csv_text: str, corpus: object) -> dict[str, object]:
    """Convert a completed pilot CSV into strict annotations in corpus order."""
    corpus = validate_corpus(corpus)
    if type(csv_text) is not str:
        raise ValueError("csv_text must be a string")

    try:
        reader = csv.reader(io.StringIO(csv_text, newline=""), strict=True)
        header = next(reader, None)
        if header != list(_ANNOTATION_CSV_COLUMNS):
            raise ValueError("CSV header does not match the annotation template")
        rows = list(reader)
    except csv.Error as error:
        raise ValueError("malformed annotation CSV") from error

    expected_count = sum(len(query["candidates"]) for query in corpus["queries"])
    if len(rows) != expected_count:
        raise ValueError(f"CSV must contain exactly {expected_count} annotation rows")

    expected_hash = corpus_hash(corpus)
    reviewer_id: str | None = None
    pairs: dict[tuple[str, str], int] = {}
    for row in rows:
        if len(row) != len(_ANNOTATION_CSV_COLUMNS):
            raise ValueError("each CSV row must contain exactly the template columns")
        values = dict(zip(_ANNOTATION_CSV_COLUMNS, row))
        if any(not value.strip() for value in row):
            raise ValueError("CSV fields must not be blank")
        if values["corpus_id"] != corpus["corpus_id"]:
            raise ValueError("CSV corpus_id does not match corpus")
        if values["corpus_hash"] != expected_hash:
            raise ValueError("CSV corpus_hash does not match corpus")
        if values["rubric_version"] != "muninn-retrieval-v1.0":
            raise ValueError("unsupported rubric_version")
        if values["adjudication_status"] != "single_reviewer_not_adjudicated":
            raise ValueError("unsupported adjudication_status")
        if reviewer_id is None:
            reviewer_id = values["reviewer_id"]
        elif values["reviewer_id"] != reviewer_id:
            raise ValueError("reviewer_id must be consistent across CSV rows")

        query = next((item for item in corpus["queries"] if item["id"] == values["query_id"]), None)
        if query is None or query["text"] != values["query_text"]:
            raise ValueError("CSV query metadata does not match corpus")
        candidate = next((item for item in query["candidates"]
                          if item["id"] == values["candidate_id"]), None)
        if candidate is None or candidate["text"] != values["candidate_text"]:
            raise ValueError("CSV candidate metadata does not match corpus")
        pair = (values["query_id"], values["candidate_id"])
        if pair in pairs:
            raise ValueError("CSV contains a duplicate query-candidate pair")
        if re.fullmatch(r"(?:0|[1-3])", values["grade"]) is None:
            raise ValueError("grade must be exactly one of 0, 1, 2, or 3")
        pairs[pair] = int(values["grade"])

    expected_pairs = {
        (query["id"], candidate["id"])
        for query in corpus["queries"] for candidate in query["candidates"]
    }
    if pairs.keys() != expected_pairs:
        raise ValueError("CSV must cover every corpus query-candidate pair")

    annotations = {
        "schema_version": "1.0",
        "corpus_id": corpus["corpus_id"],
        "corpus_hash": expected_hash,
        "reviewer_id": reviewer_id,
        "rubric_version": "muninn-retrieval-v1.0",
        "adjudication": {"status": "single_reviewer_not_adjudicated", "reviewer_count": 1},
        "judgments": [
            {"query_id": query["id"], "grades": [
                {"document_id": candidate["id"],
                 "grade": pairs[(query["id"], candidate["id"])]}
                for candidate in query["candidates"]
            ]}
            for query in corpus["queries"]
        ],
    }
    return validate_annotations(annotations, corpus)


def _validate_inputs(candidates: dict[str, int], ranked: list[dict[str, object]]) -> None:
    if type(candidates) is not dict:
        raise ValueError("candidates must be a dict")
    for document_id, grade in candidates.items():
        if not isinstance(document_id, str) or not document_id:
            raise ValueError("candidate IDs must be non-empty strings")
        if type(grade) is not int or grade not in (0, 1, 2, 3):
            raise ValueError("grades must be integers from 0 through 3")

    if type(ranked) is not list:
        raise ValueError("ranked must be a list")
    seen_ids: set[str] = set()
    for result in ranked:
        if type(result) is not dict or "id" not in result or "score" not in result:
            raise ValueError("each ranked result must contain id and score")
        document_id = result["id"]
        score = result["score"]
        if not isinstance(document_id, str) or not document_id:
            raise ValueError("result IDs must be non-empty strings")
        if document_id in seen_ids:
            raise ValueError(f"duplicate result ID: {document_id}")
        seen_ids.add(document_id)
        if document_id not in candidates:
            raise ValueError(f"result has no candidate judgment: {document_id}")
        if isinstance(score, bool) or not isinstance(score, (int, float)):
            raise ValueError("scores must be finite numbers")
        try:
            finite = math.isfinite(score)
        except OverflowError:
            finite = False
        if not finite:
            raise ValueError("scores must be finite numbers")



def _validate_ks(ks: tuple[int, ...]) -> None:
    if type(ks) is not tuple or not ks:
        raise ValueError("ks must be a non-empty tuple of positive integers")
    if any(type(k) is not int or k <= 0 for k in ks):
        raise ValueError("K values must be positive integers")
    if len(set(ks)) != len(ks):
        raise ValueError("K values must be unique")


def _dcg(grades: list[int]) -> float:
    return sum((2**grade - 1) / math.log2(rank + 2) for rank, grade in enumerate(grades))


def evaluate_query(
    candidates: dict[str, int], ranked: list[dict[str, object]], ks: tuple[int, ...]
) -> dict[str, object]:
    """Validate and evaluate one query without performing any I/O."""
    _validate_ks(ks)
    _validate_inputs(candidates, ranked)

    ordered = sorted(ranked, key=lambda result: (-result["score"], result["id"]))
    ordered_ids = [result["id"] for result in ordered]
    ordered_scores = [result["score"] for result in ordered]
    relevant_count = sum(grade >= 2 for grade in candidates.values())
    recall_at_k: dict[str, float | None] = {}
    ndcg_at_k: dict[str, float | None] = {}
    ideal_grades = sorted(candidates.values(), reverse=True)

    for k in ks:
        key = str(k)
        if relevant_count:
            retrieved_relevant = sum(candidates[document_id] >= 2 for document_id in ordered_ids[:k])
            recall_at_k[key] = retrieved_relevant / relevant_count
        else:
            recall_at_k[key] = None

        ideal_dcg = _dcg(ideal_grades[:k])
        if ideal_dcg == 0:
            ndcg_at_k[key] = None
        else:
            observed_grades = [candidates[document_id] for document_id in ordered_ids[:k]]
            ndcg_at_k[key] = _dcg(observed_grades) / ideal_dcg

    first_relevant_rank = next(
        (rank for rank, document_id in enumerate(ordered_ids, start=1)
         if candidates[document_id] >= 2),
        None,
    )
    if not relevant_count:
        mrr = None
    elif first_relevant_rank is None:
        mrr = 0.0
    else:
        mrr = 1 / first_relevant_rank

    return {
        "ordered_ids": ordered_ids,
        "ordered_scores": ordered_scores,
        "recall_at_k": recall_at_k,
        "mrr": mrr,
        "ndcg_at_k": ndcg_at_k,
    }


_REQUIRED_PROVENANCE = (
    "corpus_id",
    "corpus_hash",
    "judgment_hash",
    "model_bundle",
    "runtime_build",
    "search_config",
    "run_timestamp",
)


def _validate_provenance(provenance: dict[str, object]) -> None:
    if type(provenance) is not dict:
        raise ValueError("provenance must be a dict")
    if set(provenance) != set(_REQUIRED_PROVENANCE):
        raise ValueError("provenance must contain exactly the approved fields")
    for field in _REQUIRED_PROVENANCE[:5]:
        value = provenance[field]
        if not isinstance(value, str) or not value.strip():
            raise ValueError(f"{field} must be a non-empty string")
    search_config = provenance["search_config"]
    if type(search_config) is not dict or not search_config:
        raise ValueError("search_config must be a non-empty dict")
    timestamp = provenance["run_timestamp"]
    if not isinstance(timestamp, str) or not timestamp.strip():
        raise ValueError("run_timestamp must be an ISO-8601 timestamp")
    try:
        datetime.fromisoformat(timestamp.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("run_timestamp must be an ISO-8601 timestamp") from error


def compose_report(
    queries: list[dict[str, object]],
    ks: tuple[int, ...],
    provenance: dict[str, object],
) -> dict[str, object]:
    """Compose descriptive metrics from supplied judgments, rankings, and provenance."""
    if type(queries) is not list:
        raise ValueError("queries must be a list")
    _validate_ks(ks)
    _validate_provenance(provenance)

    seen_query_ids: set[str] = set()
    per_query: list[dict[str, object]] = []
    for query in queries:
        if type(query) is not dict or not {"id", "candidates", "ranked"}.issubset(query):
            raise ValueError("each query must contain id, candidates, and ranked")
        query_id = query["id"]
        if not isinstance(query_id, str) or not query_id:
            raise ValueError("query IDs must be non-empty strings")
        if query_id in seen_query_ids:
            raise ValueError(f"duplicate query ID: {query_id}")
        seen_query_ids.add(query_id)
        metrics = evaluate_query(query["candidates"], query["ranked"], ks)
        per_query.append({"query_id": query_id, "metrics": metrics})

    macro_recall: dict[str, float | None] = {}
    macro_ndcg: dict[str, float | None] = {}
    undefined_recall: dict[str, int] = {}
    undefined_ndcg: dict[str, int] = {}
    for k in ks:
        key = str(k)
        recall_values = [
            item["metrics"]["recall_at_k"][key]
            for item in per_query
            if item["metrics"]["recall_at_k"][key] is not None
        ]
        ndcg_values = [
            item["metrics"]["ndcg_at_k"][key]
            for item in per_query
            if item["metrics"]["ndcg_at_k"][key] is not None
        ]
        macro_recall[key] = sum(recall_values) / len(recall_values) if recall_values else None
        macro_ndcg[key] = sum(ndcg_values) / len(ndcg_values) if ndcg_values else None
        undefined_recall[key] = len(per_query) - len(recall_values)
        undefined_ndcg[key] = len(per_query) - len(ndcg_values)

    mrr_values = [item["metrics"]["mrr"] for item in per_query if item["metrics"]["mrr"] is not None]
    return {
        "schema_version": "1.0",
        "state": "measured_not_accepted",
        "provenance": deepcopy(provenance),
        "formulas": {
            "recall": "relevant retrieved at K / relevant candidates (grade >= 2)",
            "mrr": "reciprocal rank of first returned relevant result (grade >= 2)",
            "ndcg": "DCG gain (2**grade - 1), discount log2(rank + 1)",
        },
        "ks": list(ks),
        "per_query": per_query,
        "macro_metrics": {
            "recall_at_k": macro_recall,
            "mrr": sum(mrr_values) / len(mrr_values) if mrr_values else None,
            "ndcg_at_k": macro_ndcg,
        },
        "undefined_query_counts": {
            "recall_at_k": undefined_recall,
            "mrr": len(per_query) - len(mrr_values),
            "ndcg_at_k": undefined_ndcg,
        },
    }
