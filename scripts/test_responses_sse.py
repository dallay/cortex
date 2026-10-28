import json
import unittest

from responses_sse import parse_sse_frames


class ParseSseFramesTests(unittest.TestCase):
    def test_matches_each_event_header_to_its_own_data(self):
        body = (
            'event: response.output_text.delta\n'
            'data: {"type":"response.output_text.delta","sequence_number":0}\n\n'
            'event: response.completed\n'
            'data: {"type":"response.completed","sequence_number":1}\n\n'
        )
        self.assertEqual(
            [event for _, event in parse_sse_frames(body)],
            [
                {"type": "response.output_text.delta", "sequence_number": 0},
                {"type": "response.completed", "sequence_number": 1},
            ],
        )

    def test_rejects_mismatched_header_even_when_type_occurs_elsewhere(self):
        body = (
            'event: response.output_text.delta\n'
            'data: {"type":"response.output_text.delta","sequence_number":0}\n\n'
            'event: response.output_text.delta\n'
            'data: {"type":"response.completed","sequence_number":1}\n\n'
        )
        with self.assertRaisesRegex(AssertionError, "does not match its data type"):
            parse_sse_frames(body)

    def test_empty_input_and_comment_only_frames_have_no_events(self):
        for body in ["", "\n\n", ": heartbeat\n\n", "event: response.created\n\n"]:
            with self.subTest(body=body):
                self.assertEqual(parse_sse_frames(body), [])

    def test_multiline_data_ignores_comments_id_and_retry_fields(self):
        body = (
            ': heartbeat\n\n'
            'id: 42\nretry: 1000\nevent: response.output_text.delta\n'
            'data: {"type":"response.output_text.delta",\n'
            ': a comment inside the frame\n'
            'data: "delta":"héllo 世界"}\n\n'
        )
        self.assertEqual(parse_sse_frames(body), [
            ("response.output_text.delta", {
                "type": "response.output_text.delta", "delta": "héllo 世界",
            }),
        ])

    def test_data_without_an_event_header_is_rejected(self):
        with self.assertRaisesRegex(AssertionError, "no event header"):
            parse_sse_frames('data: {"type":"response.completed"}\n\n')

    def test_header_from_an_earlier_frame_does_not_apply_to_later_data(self):
        body = (
            'event: response.completed\ndata: {"type":"response.completed"}\n\n'
            'data: {"type":"response.completed"}\n\n'
        )
        with self.assertRaisesRegex(AssertionError, "no event header"):
            parse_sse_frames(body)

    def test_data_must_be_an_object_with_a_string_type(self):
        for payload in [None, [], "response.completed", 1, {}, {"type": None}, {"type": 1}]:
            with self.subTest(payload=payload):
                body = f'event: response.completed\ndata: {json.dumps(payload)}\n\n'
                with self.assertRaisesRegex(AssertionError, "must contain a string type"):
                    parse_sse_frames(body)

    def test_malformed_json_and_chat_done_marker_are_rejected(self):
        for payload in ['{"type":', '[DONE]']:
            with self.subTest(payload=payload):
                with self.assertRaises(json.JSONDecodeError):
                    parse_sse_frames(f'event: response.completed\ndata: {payload}\n\n')


if __name__ == "__main__":
    unittest.main()
