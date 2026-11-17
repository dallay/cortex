"""Los dos backends no pueden aprobar omitiendo los mismos campos."""
import unittest
from copy import deepcopy

from gm01_contract import compare_append_read, canonical_conversation, validate_complete_message


class PublicCompleteTests(unittest.TestCase):
    def test_both_incomplete_messages_are_rejected(self):
        errors = validate_complete_message({'id': 'x', 'role': 'user'}, 'conversation')
        self.assertIn('fields', errors)

    def test_append_timestamp_must_match_read_in_same_backend(self):
        left = {'id': 'm', 'created_at': '2026-11-03T00:00:00Z'}
        right = {'id': 'm', 'created_at': '2026-11-04T00:00:00Z'}
        self.assertIn('created_at', compare_append_read(left, right))
        right['created_at'] = '2026-11-03T01:00:00+01:00'
        self.assertEqual(compare_append_read(left, right), [])

    def test_generated_window_inclusive_and_symmetric_for_time_fields_only(self):
        for field in ('created_at', 'updated_at', 'timestamp'):
            for delta in ('00:00:00', '00:00:00.999999', '00:00:01'):
                with self.subTest(field=field, delta=delta):
                    left = {field: '2026-11-03T00:00:00Z', 'id': 'm'}
                    right = {field: f'2026-11-03T{delta}Z', 'id': 'm'}
                    for a, b in ((left, right), (right, left)):
                        self.assertEqual(compare_append_read(
                            a, b, generated_time_window_seconds=1.0), [])
                    if delta != '00:00:00':
                        # Supplied timestamps remain exact by default.
                        self.assertTrue(compare_append_read(left, right))
                    right[field] = '2026-11-03T00:00:01.000001Z'
                    self.assertTrue(compare_append_read(
                        left, right, generated_time_window_seconds=1.0))

    def test_window_keeps_null_missing_and_other_payload_strict(self):
        left = {'created_at': '2026-11-03T00:00:00Z', 'updated_at': None,
                'id': 'm', 'content': 'same', 'metadata': {'timestamp': 'original'}}
        for field in left:
            for replacement in (None, 'different'):
                if field in ('created_at', 'updated_at') and replacement == 'different':
                    continue  # Invalid time strings are tested separately.
                right = deepcopy(left)
                right[field] = replacement
                if right == left:
                    continue
                with self.subTest(field=field, replacement=replacement):
                    self.assertIn(field, compare_append_read(
                        left, right, generated_time_window_seconds=1.0))
            right = deepcopy(left)
            del right[field]
            self.assertIn(field, compare_append_read(
                left, right, generated_time_window_seconds=1.0))
        self.assertEqual(compare_append_read(
            left, deepcopy(left), generated_time_window_seconds=1.0), [])
        right = deepcopy(left)
        right['timestamp'] = right.pop('created_at')
        self.assertEqual(compare_append_read(
            left, right, generated_time_window_seconds=1.0), [])
        for invalid in ('not-a-time', '2026-11-03T00:00:00'):
            right = dict(left, created_at=invalid)
            with self.assertRaises(ValueError):
                compare_append_read(left, right, generated_time_window_seconds=1.0)

    def test_window_does_not_mutate_or_reorder_payload(self):
        left = {'timestamp': '2026-11-03T00:00:00Z', 'content': ['a', 'b']}
        original = deepcopy(left)
        right = {'created_at': '2026-11-03T00:00:00.1Z', 'content': ['b', 'a']}
        self.assertEqual(compare_append_read(
            left, right, generated_time_window_seconds=1.0), ['content'])
        self.assertEqual(left, original)

    def test_message_and_conversation_validators_preserve_time_order(self):
        message = self.message()
        message['updated_at'] = '2026-11-02T23:59:59.999999Z'
        self.assertIn('time_order', validate_complete_message(message, message['conversation_id']))
        messages = []
        for index, time in enumerate(('00:00:00.5', '00:00:00')):
            item = self.message()
            item.pop('updated_at')
            item.pop('tool_calls')
            item['id'] = f'm{index}'
            item['timestamp'] = f'2026-11-03T{time}Z'
            messages.append(item)
        conversation = dict(id=messages[0]['conversation_id'], session_id='s', title=None,
                            created_at='2026-11-03T00:00:00Z', updated_at=None,
                            metadata={}, messages=messages)
        with self.assertRaisesRegex(ValueError, 'Orden temporal'):
            canonical_conversation(conversation)

    @staticmethod
    def message():
        return {'id': '00000000-0000-0000-0000-000000000001',
                'conversation_id': '00000000-0000-0000-0000-000000000002',
                'content': 'hello', 'role': 'user', 'metadata': {}, 'embedding': None,
                'timestamp': '2026-11-03T00:00:00Z', 'updated_at': None, 'tool_calls': None}

    def test_valid_rust_and_python_messages(self):
        message = self.message()
        self.assertEqual(validate_complete_message(message, message['conversation_id']), [])
        message['created_at'] = message.pop('timestamp')
        self.assertEqual(validate_complete_message(message, message['conversation_id']), [])
