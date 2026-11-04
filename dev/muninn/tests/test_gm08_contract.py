"""Frozen comparator rules; no Docker, history, provider or archived oracle."""
from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from uuid import UUID

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))


def document():
    """Synthetic contract unit data, explicitly NOT reference measurement."""
    fixture = json.loads((BASE / 'fixtures/gm08-v1.json').read_text())
    bindings = {}
    counter = 100
    def identifier(key):
        nonlocal counter
        counter += 1
        bindings[key] = str(UUID(int=(4 << 76) | (2 << 62) | counter))
        return bindings[key]
    traces = []
    nodes, edges = [], []
    for task in fixture['tasks']:
        tid = identifier(task['key'])
        steps = []
        for number, step in enumerate(task['steps'], 1):
            sid = identifier(f"{task['key']}.step{number}")
            cid = identifier(f"{task['key']}.call{number}")
            call = {key: deepcopy(step[key]) for key in
                    ('arguments', 'result', 'status', 'duration_ms', 'error')}
            call.update(id=cid, tool_name=fixture['tool_name'])
            steps.append(dict(id=sid, trace_id=tid, step_number=number,
                              thought=None, action=step['action'],
                              observation=step['observation'], tool_calls=[call]))
            nodes.extend([
                dict(key=sid, labels=['ReasoningStep'], properties=dict(
                    id=sid, step_number=number, action=step['action'],
                    observation=step['observation'], timestamp='2026-01-01T00:00:02Z')),
                dict(key=cid, labels=['ToolCall'], properties=dict(
                    call, timestamp='2026-01-01T00:00:03Z'))])
            edges.extend([
                dict(source=tid, target=sid, type='HAS_STEP', properties={'order': number}),
                dict(source=sid, target=cid, type='USES_TOOL', properties={}),
                dict(source=cid, target='Tool:gm08_inspect', type='INSTANCE_OF', properties={}),
                dict(source=sid, target=fixture['entity_id'], type='TOUCHED', properties={}),
                dict(source=cid, target=fixture['message_id'], type='TRIGGERED_BY', properties={})])
        outcome = task['outcome']
        summary = outcome if isinstance(outcome, str) else outcome['summary']
        traces.append(dict(id=tid, session_id=fixture['session_id'], task=task['task'],
                           steps=steps, outcome=summary, success=task['success'],
                           started_at='2026-01-01T00:00:01Z', completed_at='2026-01-01T00:00:04Z'))
        nodes.append(dict(key=tid, labels=['ReasoningTrace'], properties=dict(
            id=tid, task=task['task'], session_id=fixture['session_id'], success=task['success'],
            outcome=summary, started_at='2026-01-01T00:00:01Z', completed_at='2026-01-01T00:00:04Z')))
        edges.append(dict(source=tid, target=fixture['message_id'], type='INITIATED_BY', properties={}))
    for label, key, properties in [
        ('Entity', fixture['entity_id'], {'id': fixture['entity_id'], 'name': fixture['entity_name'], 'type': 'OBJECT'}),
        ('Message', fixture['message_id'], {'id': fixture['message_id'], 'timestamp': fixture['timestamp']}),
        ('Tool', 'Tool:gm08_inspect', {'name': fixture['tool_name'], 'total_calls': 6,
                                    'successful_calls': 2, 'failed_calls': 2,
                                    'total_duration_ms': 120,
                                    'created_at': '2026-01-01T00:00:01Z',
                                    'last_used_at': '2026-01-01T00:00:03Z'})]:
        nodes.append(dict(key=key, labels=[label], properties=properties))
    stats = [dict(name=fixture['tool_name'], total_calls=6, successful_calls=2, failed_calls=4,
                  success_rate=2 / 6, avg_duration_ms=20.0)]
    writes = []
    for trace in traces:
        writes.append(dict(operation='start_trace', arguments={}, result=deepcopy(trace)))
        for step in trace['steps']:
            writes.append(dict(operation='add_step', arguments={}, result=deepcopy(step)))
            writes.append(dict(operation='record_tool_call', arguments={}, result=deepcopy(step['tool_calls'][0])))
        writes.append(dict(operation='complete_trace', arguments={}, result=deepcopy(trace)))
    lists = {name: deepcopy(traces) for name in
             ('ordered', 'completed', 'successful', 'failed', 'page', 'supplied_dates')}
    searches = {entry['key']: deepcopy(traces) for entry in fixture['searches']}
    api = dict(traces=traces, stats=stats, lists=lists, searches=searches, writes=writes)
    graph = dict(nodes=nodes, edges=edges)
    return dict(schema='gm08-capture-v1', window=['2026-01-01T00:00:00Z', '2026-01-01T00:00:10Z'],
                bindings=bindings, api=api, graph=graph, reconnect=dict(api=deepcopy(api), graph=deepcopy(graph)))


class ContractTests(unittest.TestCase):
    def compare(self, left, right, **kwargs):
        # Assertion, rather than import error, records the intentionally missing feature in RED.
        self.assertIsNotNone(importlib.util.find_spec('gm08_contract'), 'GM08 comparator is required')
        from gm08_contract import compare_gm08
        return compare_gm08(left, right, **kwargs)

    def test_identity_time_and_json_representation_only(self):
        left, right = document(), document()
        mapping = {value: str(UUID(int=UUID(value).int + 1000)) for value in right['bindings'].values()}
        right = json.loads(json.dumps(right))
        def replace(value):
            if isinstance(value, dict):
                return {key: replace(item) for key, item in reversed(list(value.items()))}
            if isinstance(value, list):
                return [replace(item) for item in value]
            if isinstance(value, str):
                return mapping.get(value, value.replace('00:00:01Z', '00:00:01+00:00'))
            return value
        right = replace(right)
        right['graph']['nodes'].reverse()
        right['graph']['edges'].reverse()
        self.assertEqual(self.compare(left, right)['comparison'], 'pass')

    def test_adversarial_rules_fail_closed(self):
        def mutate(case):
            value = document()
            if case == 'edge': value['graph']['edges'].pop()
            if case == 'steps': value['api']['traces'][0]['steps'].reverse()
            if case == 'ranking': value['api']['searches']['all'].reverse()
            if case == 'id': value['api']['traces'][0]['id'] = value['api']['traces'][1]['id']
            if case == 'status': value['api']['traces'][0]['steps'][0]['tool_calls'][0]['status'] = 'error'
            if case == 'count': value['api']['stats'][0]['failed_calls'] = 3
            if case == 'error': value['api']['traces'][0]['steps'][1]['tool_calls'][0]['error'] = None
            if case == 'unknown': value['api']['traces'][0]['secret_field'] = 1
            if case == 'omitted': del value['api']['traces'][0]['success']
            if case == 'null': value['api']['traces'][0]['outcome'] = None
            if case == 'type': value['api']['stats'][0]['total_calls'] = True
            if case == 'time': value['api']['traces'][0]['completed_at'] = '2025-01-01T00:00:00Z'
            if case == 'order-time': value['api']['traces'][0]['completed_at'] = '2026-01-01T00:00:00Z'
            if case == 'derived-id': value['bindings']['success'] = 'gm08:OBJECT'
            if case == 'unknown-graph': value['graph']['nodes'][0]['properties']['injected'] = 1
            return value
        for case in ('edge', 'steps', 'ranking', 'id', 'status', 'count', 'error', 'unknown',
                     'omitted', 'null', 'type', 'time', 'order-time', 'derived-id', 'unknown-graph'):
            with self.subTest(case=case):
                result = self.compare(document(), mutate(case))
                self.assertEqual(result['comparison'], 'fail')
                self.assertTrue(result['errors'])

    def test_invalid_equal_captures_do_not_pass(self):
        for mutation in ('missing', 'unknown', 'status', 'edge'):
            value = document()
            if mutation == 'missing': del value['api']
            if mutation == 'unknown': value['unexpected'] = None
            if mutation == 'status': value['api']['traces'][0]['steps'][0]['tool_calls'][0]['status'] = 'invalid'
            if mutation == 'edge': value['graph']['edges'].pop()
            self.assertEqual(self.compare(value, deepcopy(value))['comparison'], 'fail', mutation)

    def test_missing_operation_and_forged_supplied_time_fail_even_when_equal(self):
        for mutation in ('operation', 'time'):
            value = document()
            if mutation == 'operation':
                del value['api']['searches']['threshold']
            else:
                value['graph']['nodes'][0]['properties']['timestamp'] = '2026-01-01T00:00:00+00:00'
                value['window'] = ['2026-01-02T00:00:00Z', '2026-01-02T00:00:10Z']
            self.assertEqual(self.compare(value, deepcopy(value))['comparison'], 'fail')

    def test_equal_but_corrupt_captures_fail_closed(self):
        """Invariants must hold per side; equality alone never proves a capture."""
        def node(value, label, key):
            return next(item for item in value['graph']['nodes']
                        if item['labels'] == [label] and item['key'] == key)

        def drop_edge(value, kind, source):
            value['graph']['edges'] = [edge for edge in value['graph']['edges']
                                       if not (edge['type'] == kind and edge['source'] == source)]

        def drop_incoming(value, kind, target):
            value['graph']['edges'] = [edge for edge in value['graph']['edges']
                                       if not (edge['type'] == kind and edge['target'] == target)]

        def cases():
            def dangling_call(value):
                drop_incoming(value, 'USES_TOOL', value['bindings']['failure.call1'])

            def step_numbers(value):
                node(value, 'ReasoningStep', value['bindings']['success.step2'])['properties']['step_number'] = 1

            def edge_order(value):
                order = next(edge for edge in value['graph']['edges']
                             if edge['type'] == 'HAS_STEP' and edge['target'] == value['bindings']['success.step2'])
                order['properties']['order'] = 9

            def call_tool(value):
                node(value, 'ToolCall', value['bindings']['success.call1'])['properties']['tool_name'] = 'other'

            def duplicates(value):
                value['graph']['edges'].append(deepcopy(value['graph']['edges'][0]))

            def reconnect(value):
                node(value['reconnect'], 'Tool', 'Tool:gm08_inspect')['properties']['failed_calls'] = 3

            return {
                'step-count': lambda value: value['api']['traces'][0]['steps'].pop(),
                'call-count': lambda value: value['api']['traces'][0]['steps'][0]['tool_calls'].clear(),
                'status-sequence': lambda value: value['api']['traces'][0]['steps'][0]['tool_calls'][0].update(status='timeout'),
                'duration': lambda value: value['api']['traces'][0]['steps'][0]['tool_calls'][0].update(duration_ms=11),
                'dangling-call': dangling_call,
                'step-numbers': step_numbers,
                'edge-order': edge_order,
                'call-tool': call_tool,
                'duplicate-edge': duplicates,
                'graph-failed-counter': lambda value: node(value, 'Tool', 'Tool:gm08_inspect')['properties'].update(failed_calls=3),
                'graph-total-counter': lambda value: node(value, 'Tool', 'Tool:gm08_inspect')['properties'].update(total_calls=5),
                'api-failed-counter': lambda value: value['api']['stats'][0].update(failed_calls=3),
                'api-successful-counter': lambda value: value['api']['stats'][0].update(successful_calls=3),
                'success-rate': lambda value: value['api']['stats'][0].update(success_rate=0.5),
                'avg-duration': lambda value: value['api']['stats'][0].update(avg_duration_ms=21.0),
                'total-duration': lambda value: node(value, 'Tool', 'Tool:gm08_inspect')['properties'].update(total_duration_ms=130),
                'graph-step-time': lambda value: node(value, 'ReasoningStep', value['bindings']['success.step1'])['properties'].update(timestamp='2025-01-01T00:00:00Z'),
                'graph-call-time': lambda value: node(value, 'ToolCall', value['bindings']['success.call3'])['properties'].update(timestamp='2026-01-01T00:00:01Z'),
                'graph-last-used': lambda value: node(value, 'Tool', 'Tool:gm08_inspect')['properties'].update(last_used_at='2025-01-01T00:00:00Z'),
                'tool-created-after-use': lambda value: node(value, 'Tool', 'Tool:gm08_inspect')['properties'].update(created_at='2026-01-01T00:00:05Z'),
                'reconnect-desync': reconnect,
            }

        for case, mutate in sorted(cases().items()):
            with self.subTest(case=case):
                value = document()
                mutate(value)
                result = self.compare(value, deepcopy(value))
                self.assertEqual(result['comparison'], 'fail', case)
                self.assertTrue(result['errors'], case)

    def test_nonzero_tolerance_rejected(self):
        result = self.compare(document(), document(), numeric_tolerance=0.01)
        self.assertEqual(result['comparison'], 'fail')
        self.assertIn('numeric_tolerance: only zero is authorized', result['errors'])

    def test_cli_retains_exact_field_diff_without_mutating_inputs(self):
        self.assertTrue((BASE / 'gm08_contract.py').is_file(), 'GM08 comparison CLI is required')
        with tempfile.TemporaryDirectory(prefix='gm08-contract-') as temp:
            root = Path(temp)
            left, right = root / 'expected.json', root / 'actual.json'
            left.write_text(json.dumps(document()))
            actual = document()
            actual['api']['stats'][0]['failed_calls'] = 3
            right.write_text(json.dumps(actual))
            before = left.read_bytes(), right.read_bytes()
            result = subprocess.run([sys.executable, '-B', str(BASE / 'gm08_contract.py'),
                                     '--expected', str(left), '--actual', str(right)],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, '')
            verdict = json.loads(result.stdout)
            self.assertEqual(verdict['comparison'], 'fail')
            self.assertTrue(any('failed_calls' in str(diff) for diff in verdict['diffs']))
            self.assertEqual(before, (left.read_bytes(), right.read_bytes()))

    def test_equal_window_and_undeclared_identity_fail_closed(self):
        # Independent verifier probes: per-side invariants must reject a capture that
        # equals its pair but is internally corrupt.
        for case in ('future-window', 'undeclared-identity'):
            value = document()
            if case == 'future-window':
                value['window'][0] = '2030-01-01T00:00:00+00:00'
            else:
                value['api']['traces'][0]['id'] = '11111111-1111-4111-8111-111111111111'
            with self.subTest(case=case):
                result = self.compare(value, deepcopy(value))
                self.assertEqual(result['comparison'], 'fail', case)
                self.assertTrue(result['errors'], case)

    def test_cli_passes_a_valid_equal_pair_and_rejects_unreadable_input(self):
        with tempfile.TemporaryDirectory(prefix='gm08-contract-') as temp:
            root = Path(temp)
            left, right = root / 'expected.json', root / 'actual.json'
            left.write_text(json.dumps(document()))
            right.write_text(json.dumps(document()))
            ok = subprocess.run([sys.executable, '-B', str(BASE / 'gm08_contract.py'),
                                 '--expected', str(left), '--actual', str(right)],
                                capture_output=True, text=True)
            self.assertEqual(ok.returncode, 0)
            self.assertEqual(json.loads(ok.stdout)['comparison'], 'pass')
            missing = subprocess.run([sys.executable, '-B', str(BASE / 'gm08_contract.py'),
                                      '--expected', str(root / 'absent.json'),
                                      '--actual', str(right)],
                                     capture_output=True, text=True)
            self.assertEqual(missing.returncode, 2)
            self.assertIn('unreadable', json.loads(missing.stdout)['errors'][0])


if __name__ == '__main__':
    unittest.main()
