"""Tests for the scenario loader."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from membench.errors import BenchError
from membench.scenario import load_scenario

SCENARIO_PATH = (
    Path(__file__).resolve().parent.parent / "scenarios" / "synthetic-memory-recall-v2.json"
)


def write_scenario(payload: dict) -> Path:
    """Write a scenario payload to a temporary file."""

    handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False, encoding="utf-8")
    with handle:
        json.dump(payload, handle)
    return Path(handle.name)


class ScenarioTests(unittest.TestCase):
    def test_repository_scenario_is_valid(self) -> None:
        scenario = load_scenario(SCENARIO_PATH)
        self.assertEqual(scenario.version, "synthetic-memory-recall-v2")
        self.assertEqual(len(scenario.cases), 7)
        self.assertIn("category-exact", scenario.case_ids)
        for case in scenario.cases:
            self.assertNotEqual(case.owner, case.empty_owner)

    def test_cases_use_distinct_scopes(self) -> None:
        scenario = load_scenario(SCENARIO_PATH)
        scopes = [case.owner for case in scenario.cases]
        self.assertEqual(len(set(scopes)), len(scopes))

    def test_missing_field_is_rejected(self) -> None:
        payload = {"version": "v", "owner": "a", "empty_owner": "b", "cases": [{"id": "x"}]}
        with self.assertRaises(BenchError):
            load_scenario(write_scenario(payload))

    def test_preference_requires_category(self) -> None:
        payload = {
            "version": "v",
            "owner": "a",
            "empty_owner": "b",
            "cases": [
                {
                    "id": "x",
                    "goal": "g",
                    "prompt": "p",
                    "seed": [{"kind": "preference", "text": "t"}],
                    "expected_contains": ["t"],
                }
            ],
        }
        with self.assertRaises(BenchError):
            load_scenario(write_scenario(payload))

    def test_message_must_not_declare_category(self) -> None:
        payload = {
            "version": "v",
            "owner": "a",
            "empty_owner": "b",
            "cases": [
                {
                    "id": "x",
                    "goal": "g",
                    "prompt": "p",
                    "seed": [{"kind": "message", "text": "t", "category": "c"}],
                    "expected_contains": ["t"],
                }
            ],
        }
        with self.assertRaises(BenchError):
            load_scenario(write_scenario(payload))

    def test_case_without_expectations_is_rejected(self) -> None:
        payload = {
            "version": "v",
            "owner": "a",
            "empty_owner": "b",
            "cases": [{"id": "x", "goal": "g", "prompt": "p", "seed": []}],
        }
        with self.assertRaises(BenchError):
            load_scenario(write_scenario(payload))

    def test_duplicate_case_ids_are_rejected(self) -> None:
        case = {
            "id": "x",
            "goal": "g",
            "prompt": "p",
            "seed": [],
            "expected_contains": ["t"],
            "owner": "a1",
        }
        payload = {
            "version": "v",
            "owner": "a",
            "empty_owner": "b",
            "cases": [case, dict(case, owner="a2")],
        }
        with self.assertRaises(BenchError):
            load_scenario(write_scenario(payload))

    def test_missing_file_is_reported(self) -> None:
        with self.assertRaises(BenchError):
            load_scenario(Path("/nonexistent/scenario.json"))


if __name__ == "__main__":
    unittest.main()
