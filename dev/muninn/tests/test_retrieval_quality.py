import csv
import io
import json
import math
import unittest
from pathlib import Path

from retrieval_quality import (
    annotations_from_csv, compose_report, evaluate_query, corpus_hash, validate_annotations, validate_corpus,
)


class RetrievalQualityTests(unittest.TestCase):
    def _pilot_inputs(self):
        root = Path(__file__).parents[1]
        corpus = json.loads((root / "fixtures/retrieval-pilot-v1-corpus.json").read_text())
        template = (root / "fixtures/retrieval-pilot-v1-annotation-template.csv").read_text()
        rows = list(csv.DictReader(io.StringIO(template, newline="")))
        for index, row in enumerate(rows):
            row["reviewer_id"] = "reviewer-k7m2"
            row["grade"] = str(index % 4)
        output = io.StringIO(newline="")
        writer = csv.DictWriter(output, fieldnames=rows[0].keys(), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
        return corpus, output.getvalue(), rows

    def test_imports_filled_pilot_csv_into_strict_schema_in_corpus_order(self):
        corpus, csv_text, rows = self._pilot_inputs()
        shuffled = [rows[index] for index in (11, 4, 0, 8, 3, 9, 2, 6, 1, 10, 5, 7)]
        output = io.StringIO(newline="")
        writer = csv.DictWriter(output, fieldnames=rows[0].keys(), lineterminator="\n")
        writer.writeheader()
        writer.writerows(shuffled)

        annotations = annotations_from_csv(output.getvalue(), corpus)

        self.assertEqual(annotations["corpus_hash"], corpus_hash(corpus))
        self.assertNotIn("csv_hash", annotations)
        self.assertEqual(validate_annotations(annotations, corpus), annotations)
        self.assertEqual(
            [(judgment["query_id"], grade["document_id"], grade["grade"])
             for judgment in annotations["judgments"] for grade in judgment["grades"]],
            [(query["id"], candidate["id"], int(row["grade"]))
             for query in corpus["queries"] for candidate, row in zip(
                 query["candidates"], [next(r for r in rows if r["query_id"] == query["id"]
                                            and r["candidate_id"] == c["id"])
                                       for c in query["candidates"]])],
        )

    def test_csv_import_rejects_bad_headers_rows_pairs_fields_metadata_and_grades(self):
        corpus, csv_text, rows = self._pilot_inputs()
        header, body = csv_text.split("\n", 1)
        invalid_csvs = [
            "wrong,header\n" + body,
            header + ",extra\n" + body,
            header + "\n" + "\n".join(body.splitlines()[:-1]) + "\n",
            csv_text + body.splitlines()[0] + "\n",
            csv_text.replace(",reviewer-k7m2,", ",,", 1),
            csv_text.replace(",0\n", ",\n", 1),
            csv_text.replace(",0\n", ",4\n", 1),
            csv_text.replace(",0\n", ",1.0\n", 1),
            csv_text.replace(",0\n", ",true\n", 1),
            csv_text.replace(",reviewer-k7m2,", ",reviewer-a8b9,", 1),
            csv_text[:-2] + '"unterminated',
            csv_text.replace("retrieval-pilot-v1-draft,", "other-corpus,", 1),
            csv_text.replace("muninn-retrieval-v1.0,", "other-rubric,", 1),
            csv_text.replace("single_reviewer_not_adjudicated,", "adjudicated,", 1),
            csv_text.replace("sha256:" + corpus_hash(corpus).split(":", 1)[1], "sha256:" + "0" * 64, 1),
            csv_text.replace("¿Cómo impedir que dos solicitudes cobren la misma compra?", "changed query", 1),
            csv_text.replace("candidate-01,", "candidate-02,", 1),
            csv_text.replace("candidate-01,", "candidate-01,", 1).replace(",0\n", ",0,extra\n", 1),
        ]
        for malformed in invalid_csvs:
            with self.subTest(csv=malformed[:100]), self.assertRaises(ValueError):
                annotations_from_csv(malformed, corpus)

        duplicate_rows = rows + [dict(rows[0])]
        output = io.StringIO(newline="")
        writer = csv.DictWriter(output, fieldnames=rows[0].keys(), lineterminator="\n")
        writer.writeheader()
        writer.writerows(duplicate_rows)
        with self.assertRaises(ValueError):
            annotations_from_csv(output.getvalue(), corpus)

    def _corpus(self):
        return {
            "schema_version": "1.0",
            "corpus_id": "draft-corpus-v1",
            "queries": [{
                "id": "query-1", "text": "Exact query text",
                "candidates": [
                    {"id": "document-1", "text": "Exact document text"},
                    {"id": "document-2", "text": "Second document"},
                ],
            }],
        }

    def test_validates_strict_unlabelled_corpus_and_annotations(self):
        corpus = self._corpus()
        self.assertEqual(validate_corpus(corpus), corpus)
        annotations = {
            "schema_version": "1.0",
            "corpus_id": corpus["corpus_id"],
            "corpus_hash": corpus_hash(corpus),
            "reviewer_id": "reviewer-k7m2",
            "rubric_version": "muninn-retrieval-v1.0",
            "adjudication": {"status": "single_reviewer_not_adjudicated", "reviewer_count": 1},
            "judgments": [{"query_id": "query-1", "grades": [
                {"document_id": "document-1", "grade": 0},
                {"document_id": "document-2", "grade": 3},
            ]}],
        }
        self.assertEqual(validate_annotations(annotations, corpus), annotations)

    def test_pilot_fixture_is_exactly_six_unlabelled_pairs_and_twelve_template_rows(self):
        root = Path(__file__).parents[1]
        corpus = json.loads((root / "fixtures/retrieval-pilot-v1-corpus.json").read_text())
        self.assertEqual(len(validate_corpus(corpus)["queries"]), 6)
        self.assertEqual(sum(len(query["candidates"]) for query in corpus["queries"]), 12)
        self.assertEqual(
            {query["id"] for query in corpus["queries"]},
            {f"query-{index:02d}" for index in range(1, 7)},
        )
        self.assertEqual(
            {candidate["id"] for query in corpus["queries"] for candidate in query["candidates"]},
            {f"candidate-{index:02d}" for index in range(1, 13)},
        )
        with (root / "fixtures/retrieval-pilot-v1-annotation-template.csv").open() as template:
            rows = list(csv.DictReader(template))
        self.assertEqual(len(rows), 12)
        self.assertTrue(all(row["grade"] == "" for row in rows))
        self.assertTrue(all(row["corpus_id"] == corpus["corpus_id"] for row in rows))
        self.assertTrue(all(row["corpus_hash"] == corpus_hash(corpus) for row in rows))
        self.assertTrue(all(row["reviewer_id"] == "" for row in rows))
        self.assertTrue(all(row["rubric_version"] == "muninn-retrieval-v1.0" for row in rows))
        self.assertTrue(all(row["adjudication_status"] == "single_reviewer_not_adjudicated" for row in rows))
        self.assertEqual(
            {(row["query_id"], row["candidate_id"]) for row in rows},
            {(query["id"], candidate["id"]) for query in corpus["queries"] for candidate in query["candidates"]},
        )
        corpus_pairs = {
            (query["id"], query["text"], candidate["id"], candidate["text"])
            for query in corpus["queries"] for candidate in query["candidates"]
        }
        template_pairs = {
            (row["query_id"], row["query_text"], row["candidate_id"], row["candidate_text"])
            for row in rows
        }
        self.assertEqual(template_pairs, corpus_pairs)

    def test_allows_same_document_identity_across_queries(self):
        corpus = self._corpus()
        second = {
            "id": "query-2", "text": "Second query",
            "candidates": [
                {"id": "document-1", "text": "Exact document text"},
                {"id": "document-3", "text": "Third document"},
            ],
        }
        corpus["queries"].append(second)
        self.assertEqual(validate_corpus(corpus), corpus)

        annotations = {
            "schema_version": "1.0", "corpus_id": corpus["corpus_id"],
            "corpus_hash": corpus_hash(corpus),
            "reviewer_id": "reviewer-k7m2", "rubric_version": "muninn-retrieval-v1.0",
            "adjudication": {"status": "single_reviewer_not_adjudicated", "reviewer_count": 1},
            "judgments": [
                {"query_id": "query-1", "grades": [
                    {"document_id": "document-1", "grade": 2},
                    {"document_id": "document-2", "grade": 0},
                ]},
                {"query_id": "query-2", "grades": [
                    {"document_id": "document-1", "grade": 3},
                    {"document_id": "document-3", "grade": 1},
                ]},
            ],
        }
        self.assertEqual(validate_annotations(annotations, corpus), annotations)

        corpus["queries"][1]["candidates"][0]["text"] = "Different text for same ID"
        with self.assertRaises(ValueError):
            validate_corpus(corpus)

    def test_rejects_string_subclasses_at_corpus_boundaries(self):
        class MisleadingString(str):
            def strip(self, *args, **kwargs):
                return "not-empty"

        for field in ("corpus_id", "query_id", "query_text", "document_text"):
            corpus = self._corpus()
            if field == "corpus_id":
                corpus["corpus_id"] = MisleadingString("   ")
            elif field == "query_id":
                corpus["queries"][0]["id"] = MisleadingString("")
            elif field == "query_text":
                corpus["queries"][0]["text"] = MisleadingString("   ")
            else:
                corpus["queries"][0]["candidates"][0]["text"] = MisleadingString("   ")
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate_corpus(corpus)

    def test_rejects_invalid_corpus_and_annotation_contracts(self):
        corpus = self._corpus()
        cases = []
        extra = self._corpus(); extra["role"] = "positive"
        cases.append(extra)
        duplicate_query = self._corpus(); duplicate_query["queries"].append(dict(duplicate_query["queries"][0]))
        cases.append(duplicate_query)
        empty_text = self._corpus(); empty_text["queries"][0]["text"] = ""
        cases.append(empty_text)
        duplicate_document = self._corpus()
        duplicate_document["queries"][0]["candidates"][1]["id"] = "document-1"
        cases.append(duplicate_document)
        extra_candidate = self._corpus()
        extra_candidate["queries"][0]["candidates"][0]["role"] = "positive"
        cases.append(extra_candidate)
        for malformed in cases:
            with self.subTest(corpus=malformed), self.assertRaises(ValueError):
                validate_corpus(malformed)

        base = {"schema_version": "1.0", "corpus_id": corpus["corpus_id"],
                "corpus_hash": corpus_hash(corpus), "reviewer_id": "reviewer-k7m2",
                "rubric_version": "muninn-retrieval-v1.0",
                "adjudication": {"status": "single_reviewer_not_adjudicated", "reviewer_count": 1},
                "judgments": [{"query_id": "query-1",
                "grades": [{"document_id": "document-1", "grade": 1},
                           {"document_id": "document-2", "grade": 2}]}]}
        invalid = []
        extra_annotation = dict(base, inferred=True); invalid.append(extra_annotation)
        missing = dict(base, judgments=[{"query_id": "query-1", "grades": base["judgments"][0]["grades"][:1]}]); invalid.append(missing)
        bad_grade = dict(base, judgments=[{"query_id": "query-1", "grades": [
            {"document_id": "document-1", "grade": True}, base["judgments"][0]["grades"][1]]}]); invalid.append(bad_grade)
        wrong_hash = dict(base, corpus_hash="sha256:" + "0" * 64); invalid.append(wrong_hash)
        wrong_identity = dict(base, corpus_id="another-corpus"); invalid.append(wrong_identity)
        for field in ("reviewer_id", "rubric_version", "adjudication"):
            missing_metadata = dict(base); del missing_metadata[field]; invalid.append(missing_metadata)
        for reviewer in ("", "reviewer@example.com", "Jane Doe"):
            invalid.append(dict(base, reviewer_id=reviewer))
        invalid.append(dict(base, rubric_version=""))
        invalid.append(dict(base, adjudication={"status": "complete", "reviewer_count": 1}))
        invalid.append(dict(base, adjudication={"status": "single_reviewer_not_adjudicated", "reviewer_count": True}))
        unknown_reference = dict(base, judgments=[{"query_id": "missing-query", "grades": []}]); invalid.append(unknown_reference)
        duplicate_grade = dict(base, judgments=[{"query_id": "query-1", "grades": [
            base["judgments"][0]["grades"][0], base["judgments"][0]["grades"][0]]}]); invalid.append(duplicate_grade)
        unknown_document = dict(base, judgments=[{"query_id": "query-1", "grades": [
            {"document_id": "unknown", "grade": 1}, base["judgments"][0]["grades"][1]]}]); invalid.append(unknown_document)
        for annotations in invalid:
            with self.subTest(annotations=annotations), self.assertRaises(ValueError):
                validate_annotations(annotations, corpus)

    def test_composes_per_query_metrics_and_eligible_macro_averages(self):
        provenance = {
            "corpus_id": "corpus-v1",
            "corpus_hash": "sha256:corpus",
            "judgment_hash": "sha256:judgments",
            "model_bundle": "minilm@sha256:model",
            "runtime_build": "muninn@build-123",
            "search_config": {"top_k": 2, "metric": "cosine"},
            "run_timestamp": "2026-11-06T12:00:00Z",
        }
        queries = [
            {"id": "relevant", "candidates": {"a": 2, "b": 0},
             "ranked": [{"id": "a", "score": 1.0}]},
            {"id": "miss", "candidates": {"a": 2, "b": 3}, "ranked": []},
            {"id": "no-relevant", "candidates": {"a": 0, "b": 1}, "ranked": []},
            {"id": "all-zero", "candidates": {"a": 0, "b": 0}, "ranked": []},
        ]
        original_queries = [dict(query) for query in queries]
        original_provenance = dict(provenance)

        report = compose_report(queries, (1, 2), provenance)

        self.assertEqual(report["state"], "measured_not_accepted")
        self.assertEqual([item["query_id"] for item in report["per_query"]],
                         ["relevant", "miss", "no-relevant", "all-zero"])
        by_id = {item["query_id"]: item["metrics"] for item in report["per_query"]}
        self.assertEqual(by_id["relevant"]["recall_at_k"]["1"], 1.0)
        self.assertEqual(by_id["relevant"]["mrr"], 1.0)
        self.assertEqual(by_id["miss"]["recall_at_k"]["1"], 0.0)
        self.assertEqual(by_id["miss"]["mrr"], 0.0)
        self.assertIsNone(by_id["no-relevant"]["recall_at_k"]["1"])
        self.assertIsNone(by_id["no-relevant"]["mrr"])
        self.assertIsNone(by_id["all-zero"]["ndcg_at_k"]["1"])
        self.assertEqual(report["macro_metrics"]["recall_at_k"]["1"], 0.5)
        self.assertEqual(report["macro_metrics"]["mrr"], 0.5)
        self.assertAlmostEqual(report["macro_metrics"]["ndcg_at_k"]["1"], 1 / 3)
        self.assertEqual(report["undefined_query_counts"], {
            "recall_at_k": {"1": 2, "2": 2}, "mrr": 2,
            "ndcg_at_k": {"1": 1, "2": 1},
        })
        self.assertEqual(queries, original_queries)
        self.assertEqual(provenance, original_provenance)

    def test_rejects_incomplete_or_malformed_provenance(self):
        required = {
            "corpus_id": "corpus-v1", "corpus_hash": "sha256:corpus",
            "judgment_hash": "sha256:judgments", "model_bundle": "model@bundle",
            "runtime_build": "runtime@build", "search_config": {"top_k": 3},
            "run_timestamp": "2026-11-06T12:00:00Z",
        }
        for field in required:
            with self.subTest(field=field):
                incomplete = dict(required)
                del incomplete[field]
                with self.assertRaises(ValueError):
                    compose_report([], (1,), incomplete)
        for field, value in (("corpus_hash", ""), ("search_config", {}),
                             ("run_timestamp", "yesterday")):
            with self.subTest(field=field):
                malformed = dict(required)
                malformed[field] = value
                with self.assertRaises(ValueError):
                    compose_report([], (1,), malformed)

        unexpected = dict(required, api_token="must-not-be-copied")
        with self.assertRaises(ValueError):
            compose_report([], (1,), unexpected)

    def test_rejects_invalid_k_values_without_queries(self):
        provenance = {
            "corpus_id": "corpus-v1", "corpus_hash": "sha256:corpus",
            "judgment_hash": "sha256:judgments", "model_bundle": "model@bundle",
            "runtime_build": "runtime@build", "search_config": {"top_k": 3},
            "run_timestamp": "2026-11-06T12:00:00Z",
        }
        for ks in ((), (0,), (-1,), (1.5,), (True,), (1, 1), [1]):
            with self.subTest(ks=ks), self.assertRaises(ValueError):
                compose_report([], ks, provenance)

    def test_orders_equal_scores_by_document_id_and_computes_metrics(self):
        result = evaluate_query(
            candidates={"doc-b": 2, "doc-a": 3, "doc-c": 0},
            ranked=[
                {"id": "doc-b", "score": 0.8},
                {"id": "doc-a", "score": 0.8},
                {"id": "doc-c", "score": 0.2},
            ],
            ks=(1, 3),
        )

        self.assertEqual(result["ordered_ids"], ["doc-a", "doc-b", "doc-c"])
        self.assertEqual(result["ordered_scores"], [0.8, 0.8, 0.2])
        self.assertEqual(result["recall_at_k"]["1"], 0.5)
        self.assertEqual(result["recall_at_k"]["3"], 1.0)
        self.assertEqual(result["mrr"], 1.0)
        self.assertAlmostEqual(result["ndcg_at_k"]["1"], 1.0)

    def test_empty_ranking_with_relevant_candidates_returns_zero_mrr(self):
        result = evaluate_query({"relevant": 2}, [], (1, 3))

        self.assertEqual(result["ordered_ids"], [])
        self.assertEqual(result["ordered_scores"], [])
        self.assertEqual(result["recall_at_k"], {"1": 0.0, "3": 0.0})
        self.assertEqual(result["mrr"], 0.0)
        self.assertEqual(result["ndcg_at_k"], {"1": 0.0, "3": 0.0})

    def test_no_relevant_candidates_has_undefined_recall_and_mrr(self):
        result = evaluate_query({"zero": 0, "one": 1}, [{"id": "zero", "score": 1}], (1,))

        self.assertIsNone(result["recall_at_k"]["1"])
        self.assertIsNone(result["mrr"])
        self.assertEqual(result["ndcg_at_k"]["1"], 0.0)

    def test_all_zero_grades_have_undefined_ndcg(self):
        result = evaluate_query({"zero-a": 0, "zero-b": 0}, [{"id": "zero-a", "score": 1}], (1,))

        self.assertIsNone(result["ndcg_at_k"]["1"])

    def test_zero_grade_is_not_relevant_and_ndcg_uses_graded_gain(self):
        result = evaluate_query(
            {"zero": 0, "one": 1, "two": 2, "three": 3},
            [{"id": "zero", "score": 4}, {"id": "three", "score": 3},
             {"id": "two", "score": 2}, {"id": "one", "score": 1}],
            (1, 3, 5),
        )

        self.assertEqual(result["recall_at_k"]["1"], 0.0)
        self.assertEqual(result["recall_at_k"]["3"], 1.0)
        self.assertEqual(result["mrr"], 0.5)
        ideal = 7 + 3 / math.log2(3) + 1 / math.log2(4) + 0 / math.log2(5)
        observed = 7 / math.log2(3) + 3 / math.log2(4) + 1 / math.log2(5)
        self.assertAlmostEqual(result["ndcg_at_k"]["5"], observed / ideal)

    def test_rejects_invalid_grades(self):
        for grade in (-1, 4, 1.5, True, "2"):
            with self.subTest(grade=grade), self.assertRaises(ValueError):
                evaluate_query({"doc": grade}, [], (1,))

    def test_rejects_duplicate_candidate_and_result_ids(self):
        with self.assertRaises(ValueError):
            evaluate_query({"doc": 2}, [{"id": "doc", "score": 1}, {"id": "doc", "score": 0}], (1,))

    def test_rejects_unjudged_returned_id(self):
        with self.assertRaises(ValueError):
            evaluate_query({}, [{"id": "unknown", "score": 1}], (1,))

    def test_rejects_missing_candidate_judgment(self):
        with self.assertRaises(ValueError):
            evaluate_query({"judged": 2}, [{"id": "other", "score": 1}], (1,))

    def test_rejects_non_finite_or_non_numeric_scores(self):
        for score in (math.nan, math.inf, -math.inf, "1", True, 10**1000):
            with self.subTest(score=score), self.assertRaises(ValueError):
                evaluate_query({"doc": 2}, [{"id": "doc", "score": score}], (1,))

    def test_rejects_invalid_k_values(self):
        for ks in ((), (0,), (-1,), (1.5,), (True,), (1, 1), [1]):
            with self.subTest(ks=ks), self.assertRaises(ValueError):
                evaluate_query({"doc": 2}, [], ks)


if __name__ == "__main__":
    unittest.main()
