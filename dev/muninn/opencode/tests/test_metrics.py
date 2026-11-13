"""Tests for the assertions and the diagnosis rules."""

from __future__ import annotations

import json
import unittest

from membench.events import parse_events
from membench.manifest import render_report
from membench.metrics import (
    CONDITION_EMPTY,
    CONDITION_MEMORY,
    CONDITION_ORACLE,
    DIAGNOSIS_AUTONOMY,
    DIAGNOSIS_INCOMPLETE,
    DIAGNOSIS_MODEL,
    DIAGNOSIS_OK,
    DIAGNOSIS_RETRIEVAL,
    DIAGNOSIS_TOOL_USE,
    DIAGNOSIS_UTILIZATION,
    MODE_AUTONOMOUS,
    MODE_DIRECT,
    MODE_FORCED,
    MODE_ORACLE,
    RunResult,
    diagnose_case,
    evaluate_agent_run,
    evaluate_direct_retrieval,
    extract_hits,
    summarize,
)
from membench.scenario import load_scenario
from pathlib import Path

SCENARIO_PATH = (
    Path(__file__).resolve().parent.parent / "scenarios" / "synthetic-memory-recall-v2.json"
)


def tool_event(name: str, output: str) -> str:
    """Build one tool event."""

    return (
        '{"type":"tool_use","part":{"id":"t1","tool":"'
        + name
        + '","state":{"status":"completed","input":{},"output":'
        + output
        + "}}}"
    )


def error_event(message: str) -> str:
    """Build one error event."""

    return json.dumps({"type": "error", "error": {"data": {"message": message}}})


def agent_stream(answer: str, with_tool: bool = True, hits: str = "[]") -> str:
    """Build a synthetic agent stream."""

    lines = ['{"type":"text","part":{"id":"p1","text":"' + answer + '"}}']
    if with_tool:
        lines.append(tool_event("muninn_search_preferences", hits))
    return "\n".join(lines)


class ExtractHitsTests(unittest.TestCase):
    def test_json_string_is_decoded(self) -> None:
        self.assertEqual(extract_hits('[{"content":"a"}]'), [{"content": "a"}])

    def test_content_wrapper_is_unwrapped(self) -> None:
        self.assertEqual(extract_hits({"content": [{"content": "a"}]}), [{"content": "a"}])

    def test_invalid_payload_yields_no_hits(self) -> None:
        self.assertEqual(extract_hits("not json"), [])


class AgentEvaluationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scenario = load_scenario(SCENARIO_PATH)
        self.case = self.scenario.case("category-exact")

    def test_correct_answer_passes(self) -> None:
        trace = parse_events(
            agent_stream(
                "Title Juniper Harbor with columns Task, Owner, Status.",
                hits='[{"content":"Juniper Harbor"}]',
            )
        )
        result = evaluate_agent_run(self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertTrue(result.passed)
        self.assertEqual(result.diagnosis, DIAGNOSIS_OK)

    def test_missing_answer_fails(self) -> None:
        trace = parse_events(agent_stream("I do not know.", hits='[{"content":"Juniper Harbor"}]'))
        result = evaluate_agent_run(self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertFalse(result.passed)
        self.assertIn("Juniper Harbor", result.missing)

    def test_agent_that_never_calls_the_tool_is_reported(self) -> None:
        trace = parse_events(agent_stream("Juniper Harbor, Task, Owner, Status", with_tool=False))
        result = evaluate_agent_run(self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertFalse(result.mcp_invoked)
        self.assertEqual(result.diagnosis, DIAGNOSIS_AUTONOMY)

    def test_forced_run_that_skips_the_tool_is_a_tool_use_failure(self) -> None:
        trace = parse_events(agent_stream("Juniper Harbor, Task, Owner, Status", with_tool=False))
        result = evaluate_agent_run(self.case, MODE_FORCED, CONDITION_MEMORY, 1, trace)
        self.assertEqual(result.diagnosis, DIAGNOSIS_TOOL_USE)

    def test_agent_that_calls_the_tool_without_results_is_a_retrieval_failure(self) -> None:
        trace = parse_events(agent_stream("Nothing found.", hits="[]"))
        result = evaluate_agent_run(self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertFalse(result.evidence_retrieved)
        self.assertEqual(result.diagnosis, DIAGNOSIS_RETRIEVAL)

    def test_abstention_case_passes_when_the_agent_declines(self) -> None:
        case = self.scenario.case("preference-missing")
        trace = parse_events(agent_stream("That information is not available.", hits="[]"))
        result = evaluate_agent_run(case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertTrue(result.passed)

    def test_abstention_case_that_passed_is_not_reported_as_a_failure(self) -> None:
        case = self.scenario.case("preference-missing")
        trace = parse_events(agent_stream("That information is not available.", hits="[]"))
        result = evaluate_agent_run(case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertEqual(result.diagnosis, DIAGNOSIS_OK)
        self.assertEqual(result.failure_code, "")

    def test_abstention_case_fails_when_the_agent_leaks_another_record(self) -> None:
        case = self.scenario.case("preference-missing")
        trace = parse_events(agent_stream("The office closes at 18:00.", hits="[]"))
        result = evaluate_agent_run(case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace)
        self.assertFalse(result.passed)
        self.assertIn("18:00", result.leaked)


class DirectEvaluationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.scenario = load_scenario(SCENARIO_PATH)

    def test_hit_with_the_expected_value_passes(self) -> None:
        case = self.scenario.case("category-exact")
        result = evaluate_direct_retrieval(
            case,
            CONDITION_MEMORY,
            1,
            [{"content": "For the campaign status report, use the title Juniper Harbor "
                "and the columns Task, Owner, Status in that order."}],
        )
        self.assertTrue(result.passed)

    def test_empty_result_fails(self) -> None:
        case = self.scenario.case("category-exact")
        result = evaluate_direct_retrieval(case, CONDITION_MEMORY, 1, [])
        self.assertFalse(result.passed)
        self.assertEqual(result.diagnosis, DIAGNOSIS_RETRIEVAL)

    def test_error_is_not_evaluated_as_a_retrieval_failure(self) -> None:
        case = self.scenario.case("category-incompatible")
        result = evaluate_direct_retrieval(
            case,
            CONDITION_MEMORY,
            1,
            [],
            error="the MCP server closed the connection",
        )
        self.assertEqual(result.failure_code, "MCP_ERROR")
        self.assertEqual(result.diagnosis, DIAGNOSIS_INCOMPLETE)
        self.assertFalse(result.evaluated)
        self.assertFalse(result.passed)

    def test_abstention_case_that_passed_is_not_reported_as_a_failure(self) -> None:
        case = self.scenario.case("preference-missing")
        result = evaluate_direct_retrieval(case, CONDITION_MEMORY, 1, [])
        self.assertTrue(result.passed)
        self.assertEqual(result.diagnosis, DIAGNOSIS_OK)
        self.assertEqual(result.failure_code, "")


class DiagnosisTests(unittest.TestCase):
    def result(self, mode: str, passed: bool) -> RunResult:
        return RunResult(
            case_id="c",
            mode=mode,
            condition=CONDITION_MEMORY,
            repetition=1,
            answer_correct=passed,
        )

    def test_direct_failure_points_at_retrieval(self) -> None:
        diagnosis = diagnose_case(
            [self.result(MODE_DIRECT, False), self.result(MODE_FORCED, True)]
        )
        self.assertEqual(diagnosis, DIAGNOSIS_RETRIEVAL)

    def test_forced_failure_points_at_tool_use(self) -> None:
        diagnosis = diagnose_case(
            [self.result(MODE_DIRECT, True), self.result(MODE_FORCED, False)]
        )
        self.assertEqual(diagnosis, DIAGNOSIS_TOOL_USE)

    def test_autonomous_failure_points_at_the_model(self) -> None:
        diagnosis = diagnose_case(
            [
                self.result(MODE_DIRECT, True),
                self.result(MODE_FORCED, True),
                self.result(MODE_AUTONOMOUS, False),
                self.result(MODE_ORACLE, False),
            ]
        )
        self.assertEqual(diagnosis, DIAGNOSIS_MODEL)

    def test_autonomous_failure_with_a_passing_oracle_points_at_utilization(self) -> None:
        diagnosis = diagnose_case(
            [
                self.result(MODE_DIRECT, True),
                self.result(MODE_FORCED, True),
                self.result(MODE_AUTONOMOUS, False),
                self.result(MODE_ORACLE, True),
            ]
        )
        self.assertEqual(diagnosis, DIAGNOSIS_UTILIZATION)

    def test_every_mode_passing_is_ok(self) -> None:
        diagnosis = diagnose_case(
            [
                self.result(MODE_DIRECT, True),
                self.result(MODE_FORCED, True),
                self.result(MODE_AUTONOMOUS, True),
                self.result(MODE_ORACLE, True),
            ]
        )
        self.assertEqual(diagnosis, DIAGNOSIS_OK)

    def test_missing_modes_are_reported_as_incomplete(self) -> None:
        diagnosis = diagnose_case([self.result(MODE_DIRECT, True)])
        self.assertEqual(diagnosis, DIAGNOSIS_INCOMPLETE)


class FailureClassificationTests(unittest.TestCase):
    """A failed run is classified before it is attributed to the model."""

    def setUp(self) -> None:
        self.scenario = load_scenario(SCENARIO_PATH)
        self.case = self.scenario.case("category-exact")

    def test_provider_quota_error_is_an_infrastructure_error(self) -> None:
        trace = parse_events(
            error_event(
                "Rate limit reached for groq. Limit 1000 tokens per day (TPD). "
                "Please try again in 8h2m."
            )
        )
        result = evaluate_agent_run(
            self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace
        )
        self.assertEqual(result.failure_code, "INFRASTRUCTURE_ERROR")
        self.assertEqual(result.diagnosis, DIAGNOSIS_INCOMPLETE)
        self.assertFalse(result.evaluated)
        self.assertFalse(result.passed)

    def test_tool_schema_violation_is_a_tool_use_failure(self) -> None:
        trace = parse_events(
            error_event(
                "parameters for tool muninn_muninn_search_preferences did not match "
                "schema: errors: [/limit: expected integer, but got string]"
            )
        )
        result = evaluate_agent_run(
            self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace
        )
        self.assertEqual(result.failure_code, "TOOL_USE_ERROR")
        self.assertEqual(result.diagnosis, DIAGNOSIS_TOOL_USE)
        self.assertTrue(result.evaluated)

    def test_unknown_crash_is_not_evaluated(self) -> None:
        trace = parse_events(error_event("opencode exited with status 1"))
        result = evaluate_agent_run(
            self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace
        )
        self.assertEqual(result.failure_code, "RUN_ERROR")
        self.assertEqual(result.diagnosis, DIAGNOSIS_INCOMPLETE)
        self.assertFalse(result.evaluated)

    def test_manifest_reports_the_evaluated_flag(self) -> None:
        trace = parse_events(error_event("opencode exited with status 1"))
        result = evaluate_agent_run(
            self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace
        )
        self.assertIn("evaluated", result.to_manifest())
        self.assertFalse(result.to_manifest()["evaluated"])


class EvaluatedSeparationTests(unittest.TestCase):
    """A run without an answer is not evidence about the system under test."""

    def not_evaluated(self, mode: str) -> RunResult:
        return RunResult(
            case_id="c",
            mode=mode,
            condition=CONDITION_MEMORY,
            repetition=1,
            evaluated=False,
            answer_correct=False,
            failure_code="INFRASTRUCTURE_ERROR",
            diagnosis=DIAGNOSIS_INCOMPLETE,
        )

    def test_case_without_any_evaluated_run_has_no_signal(self) -> None:
        diagnosis = diagnose_case(
            [
                self.not_evaluated(MODE_DIRECT),
                self.not_evaluated(MODE_FORCED),
                self.not_evaluated(MODE_AUTONOMOUS),
            ]
        )
        self.assertEqual(diagnosis, DIAGNOSIS_INCOMPLETE)

    def test_summary_separates_evaluated_and_not_evaluated_runs(self) -> None:
        results = [
            self.not_evaluated(MODE_AUTONOMOUS),
            RunResult(
                case_id="c",
                mode=MODE_AUTONOMOUS,
                condition=CONDITION_MEMORY,
                repetition=2,
                answer_correct=True,
            ),
        ]
        summary = summarize(results)
        self.assertEqual(summary["runs"], 2)
        self.assertEqual(summary["evaluated"], 1)
        self.assertEqual(summary["not_evaluated"], 1)
        self.assertEqual(summary["passed"], 1)
        self.assertIn("note", summary)
        bucket = summary["by_mode"][MODE_AUTONOMOUS]
        self.assertEqual(bucket["runs"], 2)
        self.assertEqual(bucket["evaluated"], 1)
        self.assertEqual(bucket["not_evaluated"], 1)
        self.assertEqual(bucket["passed"], 1)

    def test_report_lists_the_runs_that_were_not_evaluated(self) -> None:
        report = render_report(
            {"run_id": "run-1"},
            [
                {
                    "case_id": "c",
                    "mode": MODE_AUTONOMOUS,
                    "condition": CONDITION_MEMORY,
                    "repetition": 1,
                    "evaluated": False,
                    "failure_code": "INFRASTRUCTURE_ERROR",
                    "diagnosis": DIAGNOSIS_INCOMPLETE,
                }
            ],
        )
        self.assertIn("## Not evaluated", report)
        self.assertIn("Failure", report)
        self.assertIn("INFRASTRUCTURE_ERROR", report)


class ConditionSemanticsUnchangedTests(unittest.TestCase):
    """The condition semantics are out of scope for this change.

    The protocol intent of the empty scope (C0) is not confirmed yet, so the
    scoring of C0 stays exactly as it was. These tests guard the memory
    condition (C1), which keeps its current behaviour.
    """

    def setUp(self) -> None:
        self.scenario = load_scenario(SCENARIO_PATH)
        self.case = self.scenario.case("category-exact")

    def test_memory_condition_still_requires_the_expected_values(self) -> None:
        trace = parse_events(
            agent_stream(
                "The title is Juniper Harbor.", hits='[{"content":"Juniper Harbor"}]'
            )
        )
        result = evaluate_agent_run(
            self.case, MODE_AUTONOMOUS, CONDITION_MEMORY, 1, trace
        )
        self.assertFalse(result.passed)
        self.assertIn("Task", result.missing)

    def test_memory_condition_direct_retrieval_still_requires_the_values(self) -> None:
        result = evaluate_direct_retrieval(
            self.case, CONDITION_MEMORY, 1, [{"content": "Juniper Harbor"}]
        )
        self.assertFalse(result.passed)
        self.assertIn("Task", result.missing)


if __name__ == "__main__":
    unittest.main()
