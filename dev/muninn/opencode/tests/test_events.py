"""Tests for the event stream parser."""

from __future__ import annotations

import unittest

from membench.events import parse_events

STREAM = "\n".join(
    [
        '{"type":"step-start","part":{"id":"s1"}}',
        '{"type":"text","part":{"id":"p1","text":"The title is "}}',
        '{"type":"text","part":{"id":"p1","text":"Juniper Harbor."}}',
        '{"type":"tool_use","part":{"id":"t1","tool":"muninn_search_preferences",'
        '"state":{"status":"completed","input":{"query":"report"},"output":"[{\\"content\\":'
        '\\"Juniper Harbor\\"}]"}}}',
        '{"type":"step-finish","part":{"tokens":{"input":120,"output":45},"cost":0.002}}',
        "not json at all",
    ]
)


class EventTests(unittest.TestCase):
    def test_text_is_accumulated_per_part(self) -> None:
        trace = parse_events(STREAM)
        self.assertEqual(trace.text, "The title is Juniper Harbor.")

    def test_tool_calls_are_collected(self) -> None:
        trace = parse_events(STREAM)
        self.assertEqual(len(trace.tool_calls), 1)
        call = trace.tool_calls[0]
        self.assertEqual(call.tool, "muninn_search_preferences")
        self.assertEqual(call.status, "completed")
        self.assertEqual(call.input, {"query": "report"})

    def test_muninn_calls_are_flagged(self) -> None:
        trace = parse_events(STREAM)
        self.assertEqual(len(trace.muninn_tool_calls), 1)

    def test_usage_is_accumulated(self) -> None:
        trace = parse_events(STREAM)
        self.assertEqual(trace.input_tokens, 120)
        self.assertEqual(trace.output_tokens, 45)
        self.assertAlmostEqual(trace.cost, 0.002)

    def test_noise_does_not_fail_the_run(self) -> None:
        trace = parse_events(STREAM)
        self.assertEqual(trace.errors, [])
        self.assertEqual(len(trace.noise), 1)

    def test_error_events_are_recorded(self) -> None:
        trace = parse_events('{"type":"error","error":{"name":"Error","data":{"message":"boom"}}}')
        self.assertTrue(trace.failed)
        self.assertIn("boom", trace.errors[0])

    def test_empty_stream_is_not_a_failure(self) -> None:
        trace = parse_events("")
        self.assertEqual(trace.text, "")
        self.assertFalse(trace.failed)


if __name__ == "__main__":
    unittest.main()
