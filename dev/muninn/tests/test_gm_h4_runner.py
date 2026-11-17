"""H4 runner preflight, ownership and peer guards; no Docker or network runs.

The simulations drive the same production code paths the real U6 run uses.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from uuid import uuid4

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

EVIDENCE = (BASE.parents[1] / '.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory'
            / 'evidence')


def completed(returncode=0, stdout='', stderr=''):
    return SimpleNamespace(returncode=returncode, stdout=stdout, stderr=stderr)


class ScriptedRunner:
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
            return completed(1, '', 'Error: no such object\n')
        return completed(0, '"image" "7474:7474" true ["ALL"] ["no-new-privileges"] {}\n', '')
    return completed(0, '', '')


class CommandTests(unittest.TestCase):
    def invoke(self, *arguments):
        return subprocess.run([sys.executable, '-B', str(BASE / 'gm_h4_runner.py'),
                               *arguments], text=True, capture_output=True)

    def test_help_documents_the_source_bound_dory_lifecycle(self):
        result = self.invoke('--help')
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stderr, '')
        for word in ('--sources', '--output', '--cargo-cache', 'dory', 'offline'):
            self.assertIn(word, result.stdout)
        for forbidden in ('--binary', '--tolerance', '--epsilon', '--deviation'):
            self.assertNotIn(forbidden, result.stdout)

    def test_external_binary_is_rejected_before_any_resource(self):
        with tempfile.TemporaryDirectory(prefix='gm-h4-cli-') as temp:
            result = self.invoke('--sources', temp, '--output', str(Path(temp) / 'out'),
                                 '--binary', '/unverified')
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, '')
            self.assertIn('unrecognized arguments: --binary /unverified', result.stderr)
            self.assertFalse((Path(temp) / 'out').exists())

    def test_no_tolerance_or_deviation_bypass_exists(self):
        for flag in ('--epsilon', '--tolerance', '--deviation'):
            with tempfile.TemporaryDirectory(prefix='gm-h4-cli-') as temp:
                result = self.invoke('--sources', temp, '--output', str(Path(temp) / 'out'),
                                     flag, '1e-03')
                self.assertEqual(result.returncode, 2, flag)
                self.assertEqual(result.stdout, '')
                self.assertIn(f'unrecognized arguments: {flag} 1e-03', result.stderr)

    def test_output_outside_the_task_evidence_root_is_refused(self):
        with tempfile.TemporaryDirectory(prefix='gm-h4-cli-') as temp:
            output = Path(temp) / 'evidence'
            result = self.invoke('--sources', temp, '--output', str(output))
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, '')
            self.assertEqual(json.loads(result.stdout), {'status': 'fail', 'stage': 'output'})
            self.assertFalse(output.exists())

    def test_unverified_sources_fail_before_any_resource(self):
        output = EVIDENCE / ('absent-' + uuid4().hex)
        with tempfile.TemporaryDirectory(prefix='gm-h4-cli-') as temp:
            result = self.invoke('--sources', temp, '--output', str(output))
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stderr, '')
            self.assertEqual(json.loads(result.stdout), {'status': 'fail', 'stage': 'preflight'})
            self.assertFalse(output.exists())


class PreflightTests(unittest.TestCase):
    def sources(self, tck_name):
        import gm_h4_runner
        temp = tempfile.TemporaryDirectory(prefix='gm-h4-preflight-')
        root = Path(temp.name)
        (root / 'agent-memory').symlink_to(BASE / '.cache/sources/agent-memory')
        (root / 'agent-memory-tck').symlink_to(BASE / '.cache/sources' / tck_name)
        self.addCleanup(temp.cleanup)
        return gm_h4_runner, root

    def test_h4_accepts_only_normative_b6c7_tck_tree(self):
        runner, sources = self.sources('tck-b6c7')
        integrity = runner.preflight(sources)
        self.assertEqual(integrity['agent-memory-tck']['commit'],
                         'b6c72c36bb7486881b87053ee1610f2caf972fb6')

    def test_h4_rejects_shared_consumer_e60_tck_tree(self):
        runner, sources = self.sources('agent-memory-tck')
        with self.assertRaises(ValueError):
            runner.preflight(sources)

    def test_gm05_and_gm08_keep_the_shared_e60_preflight_pin(self):
        from gm08_runner import preflight
        self.assertIn('e60b410f1046dc21db50f9aba48e171dc8a9b5a1',
                      preflight.__code__.co_consts)
        shared = json.loads((BASE / 'source-integrity.json').read_text())
        self.assertEqual(shared['agent-memory-tck']['commit'],
                         'e60b410f1046dc21db50f9aba48e171dc8a9b5a1')

    def test_peer_loads_the_h4_integrity_manifest_not_the_shared_one(self):
        import gm_h4_peer
        integrity = gm_h4_peer.load_integrity_manifest()
        self.assertEqual(integrity['agent-memory-tck']['commit'],
                         'b6c72c36bb7486881b87053ee1610f2caf972fb6')
        shared = json.loads((BASE / 'source-integrity.json').read_text())
        self.assertEqual(shared['agent-memory-tck']['commit'],
                         'e60b410f1046dc21db50f9aba48e171dc8a9b5a1')
        self.assertIn('gm_h4_source_integrity.json', __import__('gm_h4_runner').HELPERS)

class SummaryTests(unittest.TestCase):
    def skeleton(self):
        import gm_h4_runner
        return gm_h4_runner.summary_skeleton(reference_commit='a' * 40,
                                             tck_commit='b' * 40, images={})

    def test_summary_declares_the_epsilon_and_discloses_hashes(self):
        skeleton = self.skeleton()
        self.assertEqual(skeleton['numeric_epsilon'], 1e-9)
        self.assertEqual(skeleton['deviations'], [])
        self.assertRegex(skeleton['declarations_sha256'], r'^[0-9a-f]{64}$')
        self.assertEqual(set(skeleton['harness_sha256']), set())
        self.assertEqual(skeleton['status'], 'fail')
        self.assertEqual(skeleton['stage'], 'prepare')

    def test_summary_discloses_every_declared_rule_and_divergence(self):
        from gm_h4_contract import (DECLARATIONS, GM06_DIVERGENCES, GM07_DIVERGENCES,
                                    declarations_sha256)
        skeleton = self.skeleton()
        self.assertEqual(skeleton['declarations_rule_count'], len(DECLARATIONS))
        self.assertEqual(skeleton['declarations_sha256'], declarations_sha256())
        self.assertEqual(skeleton['declared_divergences'],
                         {'gm06': sorted(GM06_DIVERGENCES['product']),
                          'gm07': sorted(GM07_DIVERGENCES['reference'])})
        for unit in ('GM06', 'GM07'):
            self.assertTrue(any(key.startswith(unit) for key in DECLARATIONS))

    def test_every_measured_helper_exists_and_is_hashed(self):
        import gm_h4_runner
        from gm_h4_contract import declarations_sha256
        for name in gm_h4_runner.HELPERS:
            self.assertTrue((BASE / name).is_file(), name)
        self.assertEqual(gm_h4_runner.declared_epsilon(),
                         {'epsilon': 1e-9, 'declarations_sha256': declarations_sha256()})


class LifecycleTests(unittest.TestCase):
    def lifecycle(self, root, handler, password=None):
        import gm_h4_runner
        return gm_h4_runner.Lifecycle(root, runner=handler, password=password)

    def output(self, temp):
        path = Path(temp) / 'evidence'
        path.mkdir()
        return path

    def test_cleanup_removes_and_reverifies_only_exact_owned_identities(self):
        runner = ScriptedRunner(lambda args, kwargs: (docker_ids()(args, kwargs)
                                                     if args[1] == 'run' else report(args, kwargs)))
        with tempfile.TemporaryDirectory(prefix='gm-h4-life-') as temp:
            root = self.output(temp)
            lifecycle = self.lifecycle(root, runner)
            build = lifecycle.container('build', 'image', command=['sleep', '1'])
            peer = lifecycle.container('peer', 'image')
            self.assertEqual(lifecycle.owned, [build, peer])
            self.assertTrue(lifecycle.cleanup())
            removed = [call['args'][-1] for call in runner.calls if call['args'][1] == 'rm']
            self.assertEqual(removed, [peer, build])
            cleanup = json.loads((root / 'cleanup.json').read_text())
            self.assertTrue(all(item['absent'] for item in cleanup))
            self.assertTrue(lifecycle.cleanup())
            self.assertEqual(len([call for call in runner.calls if call['args'][1] == 'rm']), 2)

    def test_credential_exposure_blocks_publication(self):
        secret = 'synthetic-' + 'y' * 24
        leak = ScriptedRunner(lambda args, kwargs: completed(0, secret, ''))
        with tempfile.TemporaryDirectory(prefix='gm-h4-life-') as temp:
            root = self.output(temp)
            lifecycle = self.lifecycle(root, leak, password=secret)
            with self.assertRaisesRegex(RuntimeError, 'credential exposure'):
                lifecycle.run('leak', ['docker', 'info'])
            self.assertEqual([path.name for path in root.iterdir()
                              if path.name.endswith(('-stdout', '-stderr'))], [])
            for path in root.iterdir():
                self.assertNotIn(secret, path.read_text())

    def test_invalid_container_identity_never_becomes_owned(self):
        runner = ScriptedRunner(lambda args, kwargs: completed(0, 'not-a-container-id\n', ''))
        with tempfile.TemporaryDirectory(prefix='gm-h4-life-') as temp:
            lifecycle = self.lifecycle(self.output(temp), runner)
            with self.assertRaisesRegex(RuntimeError, 'invalid owned container identity'):
                lifecycle.container('build', 'image')
            self.assertEqual(lifecycle.owned, [])

    def test_published_ports_are_refused(self):
        def ports(args, kwargs):
            if args[1] == 'run':
                return docker_ids()(args, kwargs)
            if args[1] == 'inspect' and args[3] == '{{json .HostConfig.PortBindings}}':
                return completed(0, '{"7687/tcp":[{"HostPort":"7687"}]}\n', '')
            return report(args, kwargs)

        runner = ScriptedRunner(ports)
        with tempfile.TemporaryDirectory(prefix='gm-h4-life-') as temp:
            lifecycle = self.lifecycle(self.output(temp), runner)
            with self.assertRaisesRegex(RuntimeError, 'published fixture ports prohibited'):
                lifecycle.container('reference-db', 'image')
            self.assertEqual(len(lifecycle.owned), 1)
            self.assertTrue(lifecycle.cleanup())


class PeerTests(unittest.TestCase):
    def test_only_known_capture_sides_are_accepted(self):
        import gm_h4_peer
        self.assertEqual(gm_h4_peer.validate_side('reference'), 'reference')
        self.assertEqual(gm_h4_peer.validate_side('product'), 'product')
        for value in ('oracle', '', 'product '):
            with self.assertRaisesRegex(ValueError, 'unknown capture side'):
                gm_h4_peer.validate_side(value)

    def test_capture_document_is_schema_bound(self):
        import gm_h4_peer
        document = gm_h4_peer.capture_document('product', {'messages': []}, {'entities': []},
                                               {'attempts': []}, {'attempts': []},
                                               {'entities': []})
        self.assertEqual(set(document), {'schema', 'side', 'gm02', 'gm03', 'gm04', 'gm06',
                                        'gm07'})
        self.assertEqual(document['schema'], 'gm_h4-capture-v1')
        self.assertEqual(document['side'], 'product')
        with self.assertRaisesRegex(ValueError, 'unknown capture side'):
            gm_h4_peer.capture_document('oracle', {}, {}, {}, {}, {})

    def test_recorded_providers_serve_only_frozen_answers(self):
        import gm_h4_peer
        records = json.loads((BASE / 'fixtures/gm_h4_v1.json').read_text())['gm06']['providers']
        clock = iter([0.0, 0.5, 1.1, 1.7]).__next__
        providers = gm_h4_peer.RecordedProviders(records, clock=clock)
        status, body = providers.answer('/api/rest_v1/page/summary/Gm_H6_Atlas_Institute')
        self.assertEqual(status, 200)
        self.assertEqual(body['wikibase_item'], 'Q6106')
        status, body = providers.answer('/search')
        self.assertEqual(status, 200)
        self.assertEqual(body[0]['type'], 'harbour')
        self.assertEqual([item['provider'] for item in providers.requests()],
                         ['wikimedia', 'nominatim'])
        with self.assertRaisesRegex(RuntimeError, 'recorded provider fixture is absent'):
            providers.answer('/live/provider')
        self.assertEqual(json.loads(json.dumps(providers.requests()))[0],
                         {'provider': 'wikimedia', 'fixture': 'wikimedia-success', 'at': 0.0})

    def test_peer_reuses_the_proven_database_and_binary_guards(self):
        source = (BASE / 'gm_h4_peer.py').read_text()
        self.assertIn('from gm08_peer import', source)
        for name in ('consume_binary', 'require_empty', 'start_bridge', 'stop_bridge'):
            self.assertIn(name, source)

    def test_absent_capture_is_a_failure_not_an_empty_document(self):
        import gm_h4_runner
        with tempfile.TemporaryDirectory(prefix='gm-h4-absent-') as temp:
            with self.assertRaisesRegex(ValueError, 'capture absent'):
                gm_h4_runner.require_capture(Path(temp), 'reference')
            (Path(temp) / 'reference.json').write_text('{}')
            self.assertEqual(gm_h4_runner.require_capture(Path(temp), 'reference'),
                             Path(temp) / 'reference.json')


if __name__ == '__main__':
    unittest.main()
