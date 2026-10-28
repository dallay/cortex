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


if __name__ == "__main__":
    unittest.main()
