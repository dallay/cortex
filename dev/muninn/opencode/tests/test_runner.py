"""Tests for the run orchestration that do not need Docker."""

from __future__ import annotations

import unittest
from pathlib import Path

from membench.errors import BenchError
from membench.runner import Bench, RunOptions, direct_evidence
from membench.scenario import load_scenario

SCENARIO_PATH = (
    Path(__file__).resolve().parent.parent / "scenarios" / "synthetic-memory-recall-v2.json"
)


def options(**overrides) -> RunOptions:
    """Build run options for the repository scenario."""

    values = {
        "model": "provider/model",
        "scenario_path": SCENARIO_PATH,
        "repetitions": 1,
        "embedding_provider": "hash",
    }
    values.update(overrides)
    return RunOptions(**values)


class SeedPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scenario = load_scenario(SCENARIO_PATH)
        self.bench = Bench(options(), self.scenario)

    def test_records_are_deduplicated(self) -> None:
        plan = self.bench._seed_plan()
        texts = [record["text"] for record in plan["records"]]
        self.assertEqual(len(texts), len(set(texts)))

    def test_supersede_link_is_resolved_to_indexes(self) -> None:
        plan = self.bench._seed_plan()
        self.assertEqual(len(plan["supersedes"]), 1)
        link = plan["supersedes"][0]
        old = plan["records"][link["old_index"]]
        new = plan["records"][link["new_index"]]
        self.assertIn("2 spaces", old["text"])
        self.assertIn("4 spaces", new["text"])

    def test_records_carry_the_case_owner(self) -> None:
        plan = self.bench._seed_plan()
        owners = {record["owner"] for record in plan["records"]}
        self.assertIn("bench-other-owner-data", owners)


class PrepareTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scenario = load_scenario(SCENARIO_PATH)

    def test_semantic_case_requires_a_semantic_embedder(self) -> None:
        bench = Bench(options(embedding_provider="hash"), self.scenario)
        with self.assertRaises(BenchError):
            bench.prepare()

    def test_semantic_embedder_requires_a_bundle(self) -> None:
        bench = Bench(options(embedding_provider="minilm"), self.scenario)
        with self.assertRaises(BenchError):
            bench.prepare()


class PlanTests(unittest.TestCase):
    def test_plan_covers_every_mode_and_condition(self) -> None:
        scenario = load_scenario(SCENARIO_PATH)
        bench = Bench(options(repetitions=2), scenario)
        entries = bench.plan()
        # Seven cases, seven entries per repetition: A and B and C over C0 and
        # C1, plus D over C2.
        self.assertEqual(len(entries), 7 * 7 * 2)
        modes = {entry["mode"] for entry in entries}
        self.assertEqual(modes, {"A", "B", "C", "D"})
        oracle_entries = [entry for entry in entries if entry["mode"] == "D"]
        self.assertTrue(all(entry["condition"] == "C2" for entry in oracle_entries))


class DirectEvidenceTests(unittest.TestCase):
    """The evidence of a direct retrieval keeps the retrieved text."""

    def test_retrieved_text_is_kept(self) -> None:
        evidence = direct_evidence([{"content": "Juniper Harbor", "category": "reports"}])
        self.assertEqual(evidence[0]["text"], "Juniper Harbor")
        self.assertEqual(evidence[0]["category"], "reports")

    def test_only_five_hits_are_kept(self) -> None:
        hits = [{"content": f"hit {index}"} for index in range(9)]
        self.assertEqual(len(direct_evidence(hits)), 5)

    def test_the_text_is_truncated(self) -> None:
        evidence = direct_evidence([{"content": "x" * 900}])
        self.assertEqual(len(evidence[0]["text"]), 500)

    def test_the_embedding_is_not_kept(self) -> None:
        evidence = direct_evidence(
            [{"content": "Juniper Harbor", "embedding": [0.1, 0.2]}]
        )
        self.assertNotIn("embedding", evidence[0])


if __name__ == "__main__":
    unittest.main()
