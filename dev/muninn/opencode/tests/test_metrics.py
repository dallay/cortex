"""Tests for the assertions and the diagnosis rules."""

from __future__ import annotations

import unittest

from membench.events import parse_events
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

    def test_error_is_reported(self) -> None:
        case = self.scenario.case("category-exact")
        result = evaluate_direct_retrieval(case, CONDITION_MEMORY, 1, [], error="boom")
        self.assertEqual(result.failure_code, "MCP_ERROR")


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


if __name__ == "__main__":
    unittest.main()
