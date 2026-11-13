"""Public capture lifecycle and input rejection; no real fixture execution."""
import asyncio
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))


class RecordingAdapter:
    def __init__(self):
        self.events = []
        self.counter = 0

    async def seed(self, fixture):
        self.events.append(('seed', deepcopy(fixture)))

    async def call(self, operation, **arguments):
        self.events.append((operation, deepcopy(arguments)))
        if operation in ('start_trace', 'add_step', 'record_tool_call'):
            self.counter += 1
            return {'id': f'00000000-0000-4000-8000-{self.counter:012d}'}
        if operation == 'get_trace_with_steps':
            return {'id': arguments['trace_id']}
        return []

    async def snapshot(self):
        self.events.append(('snapshot', {}))
        return {'nodes': [], 'edges': []}

    async def reconnect(self):
        self.events.append(('reconnect', {}))


class CaptureTests(unittest.TestCase):
    def capture(self, adapter, fixture):
        self.assertIsNotNone(importlib.util.find_spec('gm08_capture'), 'GM08 capture is required')
        from gm08_capture import capture_gm08
        return asyncio.run(capture_gm08(adapter, fixture))

    def fixture(self):
        return json.loads((BASE / 'fixtures/gm08-v1.json').read_text())

    def test_frozen_operations_raw_results_and_reconnect(self):
        adapter = RecordingAdapter()
        result = self.capture(adapter, self.fixture())
        names = [name for name, _ in adapter.events]
        self.assertEqual(names.count('start_trace'), 2)
        self.assertEqual(names.count('add_step'), 6)
        self.assertEqual(names.count('record_tool_call'), 6)
        self.assertEqual(names.count('complete_trace'), 2)
        self.assertEqual(names.count('snapshot'), 2)
        self.assertEqual(names.count('reconnect'), 1)
        self.assertEqual(names.count('get_trace_with_steps'), 4)
        calls = [args for name, args in adapter.events if name == 'record_tool_call']
        self.assertEqual([call['status'] for call in calls], ['success', 'failure', 'timeout'] * 2)
        self.assertEqual([call['duration_ms'] for call in calls], [10, 20, 30] * 2)
        self.assertTrue(all(call['message_id'] == self.fixture()['message_id'] for call in calls))
        self.assertEqual(len(result['bindings']), 14)
        self.assertEqual(len(result['api']['writes']), 16)
        # Deliberately incomplete fake responses remain incomplete, not graph-filled.
        self.assertEqual(set(result['api']['traces'][0]), {'id'})
        self.assertEqual(result['graph'], {'nodes': [], 'edges': []})
        completed = [args for name, args in adapter.events if name == 'complete_trace']
        self.assertIsInstance(completed[0]['outcome'], str)
        self.assertEqual(completed[1]['outcome']['metrics'], {'elapsed_ms': 60.0})
        self.assertEqual(completed[1]['outcome']['related_entities'], [{'id': self.fixture()['entity_id']}])

    def test_modified_fixture_rejects_before_resource_or_counter_effects(self):
        fixture = self.fixture()
        fixture['numeric_tolerance'] = 0.01
        adapter = RecordingAdapter()
        with self.assertRaisesRegex(ValueError, 'fixture differs from frozen GM08 inputs'):
            self.capture(adapter, fixture)
        self.assertEqual(adapter.events, [])
        self.assertEqual(adapter.counter, 0)


if __name__ == '__main__':
    unittest.main()
