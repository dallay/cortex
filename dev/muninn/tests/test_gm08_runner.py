"""Runner public preflight plus simulated lifecycle, ownership and peer guards.

No test here launches Docker, a database, a provider or the oracle. The
simulations drive the same production code paths the real run uses.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))


def completed(returncode=0, stdout='', stderr=''):
    return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)


class ScriptedRunner:
    """subprocess.run stand-in: records argv/env and returns scripted results."""

    def __init__(self, handler):
        self.handler = handler
        self.calls = []

    def __call__(self, args, **kwargs):
        args = list(args)
        self.calls.append({'args': args, 'env': dict(kwargs.get('env') or {})})
        return self.handler(args, kwargs)


def docker_ids(prefix='a'):
    counter = 0

    def make(args, kwargs):
        nonlocal counter
        counter += 1
        return completed(0, (prefix * 63) + format(counter, 'x') + '\n', '')

    return make


def report(args, kwargs):
    if args[1] == 'inspect':
        if args[3] == '{{json .HostConfig.PortBindings}}':
            return completed(0, 'null\n', '')
        if args[3] == '{{.Id}}':
            # Absence probe: only reached after `docker rm`, so report it gone.
            return completed(1, '', 'Error: no such object\n')
        return completed(0, '"image" "1000:1000" true ["ALL"] ["no-new-privileges"] {}\n', '')
    return completed(0, '', '')


class RunnerTests(unittest.TestCase):
    def invoke(self, *arguments):
        self.assertTrue((BASE / 'gm08_runner.py').is_file(), 'GM08 source-bound runner is required')
        return subprocess.run([sys.executable, '-B', str(BASE / 'gm08_runner.py'), *arguments],
                              text=True, capture_output=True)

    def test_help_documents_supported_lifecycle_and_no_bypass(self):
        result = self.invoke('--help')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, '')
        for word in ('--sources', '--output', 'dory', 'offline', 'zero', 'current'):
            self.assertIn(word, result.stdout)

    def test_external_binary_rejected_before_output_creation(self):
        with tempfile.TemporaryDirectory(prefix='gm08-cli-') as temp:
            output = Path(temp) / 'evidence'
            result = self.invoke('--sources', temp, '--output', str(output), '--binary', '/unverified')
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, '')
            self.assertIn('unrecognized arguments: --binary /unverified', result.stderr)
            self.assertFalse(output.exists())

    def test_offline_cache_option_is_documented_and_validated_before_resources(self):
        result = self.invoke('--help')
        self.assertEqual(result.returncode, 0)
        self.assertIn('--cargo-cache', result.stdout)
        self.assertIn('offline', result.stdout)
        with tempfile.TemporaryDirectory(prefix='gm08-cli-') as temp:
            output = Path(temp) / 'evidence'
            missing = Path(temp) / 'no-such-cache'
            result = self.invoke('--sources', temp, '--output', str(output),
                                 '--cargo-cache', str(missing))
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, '')
            self.assertEqual(json.loads(result.stdout), {'status': 'fail', 'stage': 'preflight'})
            self.assertFalse(output.exists())

    def test_unverified_sources_rejected_without_resources(self):
        with tempfile.TemporaryDirectory(prefix='gm08-cli-') as temp:
            output = Path(temp) / 'evidence'
            result = self.invoke('--sources', temp, '--output', str(output))
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, '')
            self.assertEqual(json.loads(result.stdout), {'status': 'fail', 'stage': 'preflight'})
            self.assertFalse(output.exists())


class LifecycleTests(unittest.TestCase):
    def lifecycle(self, root, handler, password=None):
        self.assertTrue((BASE / 'gm08_runner.py').is_file(), 'GM08 lifecycle is required')
        from gm08_runner import Lifecycle
        return Lifecycle(root, runner=handler, password=password)

    def output(self, temp):
        path = Path(temp) / 'evidence'
        path.mkdir()
        return path

    def test_cleanup_removes_and_reverifies_only_exact_owned_identities(self):
        runner = ScriptedRunner(report)
        runner.handler = lambda args, kwargs: (docker_ids()(args, kwargs)
                                               if args[1] == 'run' else report(args, kwargs))
        with tempfile.TemporaryDirectory(prefix='gm08-life-') as temp:
            root = self.output(temp)
            lifecycle = self.lifecycle(root, runner)
            build = lifecycle.container('build', 'image', command=['sleep', '1'])
            peer = lifecycle.container('peer', 'image')
            self.assertEqual(lifecycle.owned, [build, peer])
            self.assertTrue(lifecycle.cleanup())
            removed = [call['args'][-1] for call in runner.calls if call['args'][1] == 'rm']
            self.assertEqual(removed, [peer, build])
            self.assertEqual(set(removed), {build, peer})
            absent = [call['args'][-1] for call in runner.calls
                      if call['args'][1] == 'inspect' and call['args'][2] == '--format'
                      and call['args'][3] == '{{.Id}}']
            self.assertEqual(set(absent), {build, peer})
            cleanup = json.loads((root / 'cleanup.json').read_text())
            self.assertTrue(all(item['absent'] for item in cleanup))
            self.assertEqual(lifecycle.cleanup(), True)
            self.assertEqual(len([c for c in runner.calls if c['args'][1] == 'rm']), 2)

    def test_failed_stage_keeps_journal_for_finally_cleanup(self):
        def handler(args, kwargs):
            if args[1] == 'run':
                return docker_ids()(args, kwargs)
            if args[1] == 'exec' and 'cargo' in args:
                raise RuntimeError('simulated infrastructure failure')
            return report(args, kwargs)

        runner = ScriptedRunner(handler)
        with tempfile.TemporaryDirectory(prefix='gm08-life-') as temp:
            lifecycle = self.lifecycle(self.output(temp), runner)
            build = lifecycle.container('build', 'image')
            with self.assertRaisesRegex(RuntimeError, 'simulated infrastructure failure'):
                lifecycle.run('build', ['docker', 'exec', build, 'cargo', 'build'])
            self.assertEqual(lifecycle.owned, [build])
            self.assertTrue(lifecycle.cleanup())
            self.assertIn(build, [call['args'][-1] for call in runner.calls if call['args'][1] == 'rm'])

    def test_credential_exposure_blocks_publication_and_never_persists(self):
        secret = 'synthetic-' + 'x' * 24
        leak = ScriptedRunner(lambda args, kwargs: completed(0, secret, ''))
        with tempfile.TemporaryDirectory(prefix='gm08-life-') as temp:
            root = self.output(temp)
            lifecycle = self.lifecycle(root, leak, password=secret)
            with self.assertRaisesRegex(RuntimeError, 'credential exposure'):
                lifecycle.run('leak', ['docker', 'info'])
            leaked = [path.name for path in root.iterdir()
                      if path.name.endswith(('-stdout', '-stderr'))]
            self.assertEqual(leaked, [])
            for path in root.iterdir():
                self.assertNotIn(secret, path.read_text())
        clean = ScriptedRunner(lambda args, kwargs: completed(0, 'ok', ''))
        with tempfile.TemporaryDirectory(prefix='gm08-life-') as temp:
            root = self.output(temp)
            lifecycle = self.lifecycle(root, clean, password=secret)
            lifecycle.run('echo', ['docker', 'version'])
            for path in root.iterdir():
                self.assertNotIn(secret, path.read_text())
            self.assertNotIn(secret, json.dumps(lifecycle.results))
            for path in root.glob('*-command.json'):
                self.assertNotIn('env', json.loads(path.read_text()))

    def test_invalid_container_identity_never_becomes_owned(self):
        runner = ScriptedRunner(lambda args, kwargs: completed(0, 'not-a-container-id\n', ''))
        with tempfile.TemporaryDirectory(prefix='gm08-life-') as temp:
            lifecycle = self.lifecycle(self.output(temp), runner)
            with self.assertRaisesRegex(RuntimeError, 'invalid owned container identity'):
                lifecycle.container('build', 'image')
            self.assertEqual(lifecycle.owned, [])


class PeerTests(unittest.TestCase):
    def peer(self):
        self.assertTrue((BASE / 'gm08_peer.py').is_file(), 'GM08 peer guards are required')
        import gm08_peer
        return gm08_peer

    def test_only_known_capture_sides_are_accepted(self):
        peer = self.peer()
        self.assertEqual(peer.validate_side('reference'), 'reference')
        self.assertEqual(peer.validate_side('product'), 'product')
        for value in ('oracle', '', 'product '):
            with self.assertRaisesRegex(ValueError, 'unknown capture side'):
                peer.validate_side(value)

    def test_consumed_binary_must_match_the_bound_hash(self):
        peer = self.peer()
        with tempfile.TemporaryDirectory(prefix='gm08-peer-') as temp:
            source, target = Path(temp) / 'built', Path(temp) / 'target'
            source.write_bytes(b'current-source-bytes')
            digest = peer.hashlib.sha256(source.read_bytes()).hexdigest()
            self.assertEqual(peer.consume_binary(source, target, digest), digest)
            self.assertEqual(target.read_bytes(), source.read_bytes())
            with self.assertRaisesRegex(ValueError, 'consumed binary hash mismatch'):
                peer.consume_binary(source, target, '0' * 64)
            self.assertFalse(target.exists())

    def test_fresh_database_guard_rejects_existing_data(self):
        peer = self.peer()
        peer.require_empty([{'count': 0}])
        with self.assertRaisesRegex(ValueError, 'fresh database is not empty'):
            peer.require_empty([{'count': 1}])
        with self.assertRaisesRegex(ValueError, 'fresh database is not empty'):
            peer.require_empty([])

    def test_reference_results_project_to_the_bridge_wire_shape(self):
        peer = self.peer()
        call = {'id': 'c', 'step_id': 's1', 'tool_name': 'x', 'arguments': {'p': 1},
                'result': None, 'status': 'success', 'duration_ms': 10, 'error': None,
                'created_at': 'created'}
        step = {'id': 's1', 'trace_id': 't', 'step_number': 1, 'thought': None,
                'action': 'a', 'observation': 'o', 'embedding': [0.2], 'metadata': {},
                'created_at': 'created', 'tool_calls': [call]}
        trace = {'id': 't', 'session_id': 's', 'task': 'k', 'task_embedding': [0.1],
                 'steps': [step], 'outcome': None, 'success': True, 'started_at': 'x',
                 'completed_at': 'y', 'metadata': {'similarity': 0.9}, 'created_at': 'c'}
        full = peer.project_wire('get_trace_with_steps', trace)
        self.assertEqual(set(full), {'id', 'session_id', 'task', 'steps', 'outcome',
                                     'success', 'started_at', 'completed_at'})
        self.assertEqual(set(full['steps'][0]), {'id', 'trace_id', 'step_number', 'thought',
                                                 'action', 'observation', 'tool_calls'})
        self.assertEqual(set(full['steps'][0]['tool_calls'][0]),
                         {'id', 'tool_name', 'arguments', 'result', 'status',
                          'duration_ms', 'error'})
        self.assertEqual(full['steps'][0]['tool_calls'][0]['status'], 'success')
        # list/get_similar traces carry no steps in the bridge wire shape.
        listed = peer.project_wire('list_traces', [trace])
        self.assertEqual(listed[0]['steps'], [])
        self.assertEqual(set(peer.project_wire('record_tool_call', call)),
                         {'id', 'tool_name', 'arguments', 'result', 'status',
                          'duration_ms', 'error'})
        stats = peer.project_wire('get_tool_stats', [{'name': 'x', 'total_calls': 2,
                                                      'successful_calls': 1, 'failed_calls': 1,
                                                      'success_rate': 0.5,
                                                      'avg_duration_ms': 10.0,
                                                      'description': None,
                                                      'last_used_at': 'z'}])
        self.assertEqual(set(stats[0]), {'name', 'total_calls', 'successful_calls',
                                         'failed_calls', 'success_rate', 'avg_duration_ms'})

    def test_graph_projection_keeps_ids_tool_keys_and_schema_exclusion(self):
        peer = self.peer()
        statements = []
        nodes = [{'labels': ['Tool'], 'properties': {'name': 'gm08_inspect', 'total_calls': 6}},
                 {'labels': ['ReasoningStep'], 'properties': {'id': 'step', 'step_number': 2}}]
        edges = [{'source': 'step', 'target': 'Tool:gm08_inspect', 'type': 'INSTANCE_OF',
                  'properties': {}}]

        def query(cypher, parameters):
            statements.append(cypher)
            return nodes if cypher.startswith('MATCH (n)') else edges

        snapshot = peer.ExecutionGraph(query).snapshot()
        self.assertEqual([node['key'] for node in snapshot['nodes']],
                         ['Tool:gm08_inspect', 'step'])
        self.assertEqual(snapshot['edges'][0]['type'], 'INSTANCE_OF')
        # The measured surface is the execution-memory label allowlist, applied to both
        # servers symmetrically, so schema/ontology bookkeeping never enters the graph.
        for text in statements:
            self.assertIn(':ReasoningTrace', text)
            self.assertIn(':ToolCall', text)
            self.assertNotIn('MuninnSchema', text)
            self.assertNotIn('Ontology', text)


if __name__ == '__main__':
    unittest.main()
