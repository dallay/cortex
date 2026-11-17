"""Tests for the command line entry point of the benchmark harness."""

from __future__ import annotations

import argparse
import unittest
from pathlib import Path
from unittest import mock

from membench import cli
from membench.metrics import MODE_DIRECT, MODE_FORCED, requires_inference_authorization

SCENARIO_PATH = (
    Path(__file__).resolve().parent.parent / "scenarios" / "synthetic-memory-recall-v2.json"
)


class _FakePreflight:
    """Preflight result without any check."""

    def __init__(self) -> None:
        self.checks: list[object] = []
        self.ok = True


class _FakeBench:
    """Benchmark double that records whether the runs were executed."""

    def __init__(self, options, scenario) -> None:
        self.options = options
        self.scenario = scenario
        self.executed = False
        self.started = False

    def prepare(self) -> None:
        """Create the run directories."""

    def plan(self) -> list[dict[str, object]]:
        """Describe the planned runs."""

        return [
            {
                "case_id": "case",
                "mode": mode,
                "condition": "C1",
                "repetition": 1,
            }
            for mode in self.options.modes
        ]

    def preflight_static(self) -> _FakePreflight:
        """Return the static preflight result."""

        return _FakePreflight()

    def start(self) -> None:
        """Start the disposable containers."""

        self.started = True

    def preflight_runtime(self) -> _FakePreflight:
        """Return the runtime preflight result."""

        return _FakePreflight()

    def execute(self) -> tuple[list[object], dict[str, object]]:
        """Execute the planned runs."""

        self.executed = True
        return [], {}

    def write_evidence(self, results, manifest) -> Path:
        """Write the evidence of the run."""

        return Path("evidence")

    def stop(self) -> None:
        """Stop the disposable containers."""

    def cleanup(self) -> None:
        """Remove the disposable containers."""


def arguments(**overrides) -> argparse.Namespace:
    """Build the parsed arguments of a run."""

    values = {
        "model": "provider/model",
        "scenario": SCENARIO_PATH,
        "evidence_root": Path("evidence"),
        "repetitions": 1,
        "timeout": 900.0,
        "modes": "A",
        "conditions": "C0,C1,C2",
        "embedding_provider": "hash",
        "embedding_bundle_path": None,
        "embedding_bundle_id": None,
        "auth_dir": Path("auth"),
        "provider_env": [],
        "authorize_inference": False,
        "keep_containers": False,
    }
    values.update(overrides)
    return argparse.Namespace(**values)


class InferenceAuthorizationTests(unittest.TestCase):
    """The authorization gate follows the modes of the plan."""

    def test_plan_without_model_inference_does_not_require_authorization(self) -> None:
        entries = [{"mode": MODE_DIRECT}]
        self.assertFalse(requires_inference_authorization(entries))

    def test_plan_with_an_agent_mode_requires_authorization(self) -> None:
        entries = [{"mode": MODE_DIRECT}, {"mode": MODE_FORCED}]
        self.assertTrue(requires_inference_authorization(entries))

    def test_direct_run_executes_without_the_authorization_flag(self) -> None:
        with mock.patch.object(cli, "Bench", _FakeBench):
            code = cli.command_run(arguments(modes="A"))
        self.assertEqual(code, 0)

    def test_agent_run_is_refused_without_the_authorization_flag(self) -> None:
        with mock.patch.object(cli, "Bench", _FakeBench):
            code = cli.command_run(arguments(modes="A,B"))
        self.assertEqual(code, 2)

    def test_agent_run_executes_with_the_authorization_flag(self) -> None:
        with mock.patch.object(cli, "Bench", _FakeBench):
            code = cli.command_run(arguments(modes="A,B", authorize_inference=True))
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
