"""El runner GM-05 nunca aborta sin veredicto publicado.

GM05-003 y GM05-017: `summary.json` se escribe siempre y, si falla la captura de
la referencia, queda un artefacto diagnóstico con el código de salida y la cola
del registro. Estas pruebas no usan Docker, ni red, ni el producto.
"""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
from types import SimpleNamespace


BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))


def module(name):
    import importlib.util
    spec = importlib.util.spec_from_file_location(name + "_under_test", BASE / (name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


ARGUMENTS = ('--sources', 'sources',
             '--reference-python', 'python')


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.runner = module("gm05_runner")
        self.directory = tempfile.TemporaryDirectory()
        self.output = Path(self.directory.name) / 'gm05'
        self.artifact = SimpleNamespace(path=Path('/isolated/target/release/muninn'),
                                        manifest={'binary_sha256': 'binary-hash'},
                                        verify=mock.Mock())
        self.build = mock.patch.object(self.runner, 'build_candidate')
        self.mock_build = self.build.start()
        self.addCleanup(self.build.stop)
        self.mock_build.return_value.__enter__.return_value = self.artifact

    def tearDown(self):
        self.directory.cleanup()

    def run_main(self, capture):
        import contextlib
        import io
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(self.runner, 'run_capture', capture), \
             contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = self.runner.main([*ARGUMENTS, '--output', str(self.output)])
        self.assertEqual(stderr.getvalue(), '')
        self.assertEqual(stdout.getvalue(), json.dumps({
            'verdict': 'pass' if code == 0 else 'fail',
            'errors': 0 if code == 0 else 1, 'gaps': []}) + '\n')
        return code

    def summary(self):
        return json.loads((self.output / 'summary.json').read_text(encoding='utf-8'))

    def test_run_capture_passes_exact_utc_window_to_contract(self):
        import asyncio
        import sys
        import types

        base = self.output / 'base'
        base.mkdir(parents=True)
        (base / 'source-integrity.json').write_text(json.dumps({
            'agent-memory': 'hash-a', 'agent-memory-tck': 'hash-b'}), encoding='utf-8')
        (base / 'references.json').write_text(json.dumps({
            'agent-memory': {'commit': 'ref'},
            'agent-memory-tck': {'commit': 'tck'}}), encoding='utf-8')
        args = SimpleNamespace(sources=base, reference_python='python')
        capture = {'gaps': []}
        observed = {}
        capture_instants = []

        class FakeDatabase:
            image = 'neo4j:test'
            uri = 'bolt://127.0.0.1:1'
            password = 'test-password'

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

        artifact = self.artifact
        class FakeProduct:
            def __init__(self, binary):
                self_binary = binary
                if self_binary != artifact.path:
                    raise AssertionError('fixture did not receive the built executable')

            url = 'http://127.0.0.1:1'
            binary_sha256 = 'binary-hash'
            image = 'neo4j:test'

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

            def query(self, *_args, **_kwargs):
                return []

        async def capture_muninn():
            from datetime import datetime, timezone
            capture_instants.append(datetime.now(timezone.utc))
            return capture

        def fake_subprocess_run(_command, env, **_kwargs):
            from datetime import datetime, timezone
            capture_instants.append(datetime.now(timezone.utc))
            Path(env['GM05_OUTPUT']).write_text(
                json.dumps({'capture': capture}), encoding='utf-8')
            return SimpleNamespace(returncode=0)

        fake_product_module = types.ModuleType('product_chain_fixture')
        fake_product_module.ProductFixture = FakeProduct
        with mock.patch.dict(sys.modules, {'product_chain_fixture': fake_product_module}), \
             mock.patch.object(self.runner, 'verify_source_tree'), \
             mock.patch.object(self.runner, 'Neo4jFixture', FakeDatabase), \
             mock.patch.object(self.runner.subprocess, 'run', side_effect=fake_subprocess_run), \
             mock.patch.object(self.runner, 'capture_gm05',
                               new=mock.Mock(side_effect=lambda _adapter: capture_muninn())), \
             mock.patch.object(self.runner, 'BridgeAdapter', return_value=object()), \
             mock.patch.object(self.runner, 'GraphReader', return_value=object()), \
             mock.patch.object(self.runner, '_file_hashes', return_value={}), \
             mock.patch.object(self.runner, 'compare_gm05',
                               side_effect=lambda *_args, **kwargs: observed.update(kwargs) or {
                                   'comparison': 'pass', 'deviations': [], 'errors': []}):
            self.runner.run_capture(args, base, self.output, self.artifact)
            self.artifact.manifest['binary_sha256'] = 'different-build-hash'
            with self.assertRaisesRegex(RuntimeError, 'Fixture executable hash differs'):
                self.runner.run_capture(args, base, self.output, self.artifact)
            capture_instants.pop()  # The rejected run captured only the reference.

        window = observed.get('window')
        self.assertIsNotNone(window)
        self.assertEqual(len(window), 2)
        start, end = window
        self.assertEqual(start.utcoffset().total_seconds(), 0)
        self.assertEqual(end.utcoffset().total_seconds(), 0)
        self.assertLessEqual(start, end)
        self.assertEqual(len(capture_instants), 2)
        self.assertTrue(all(start <= instant <= end for instant in capture_instants))

    def test_build_failure_publishes_summary_without_capture_or_docker(self):
        self.mock_build.return_value.__enter__.side_effect = RuntimeError('snapshot rejected')
        capture = mock.Mock()
        with mock.patch.object(self.runner, 'Neo4jFixture') as database:
            code = self.run_main(capture)
        self.assertEqual(code, 2)
        self.assertEqual(self.summary()['failed_stage'], 'build')
        self.assertEqual(self.summary()['verdict']['errors'],
                         ['error: RuntimeError: snapshot rejected'])
        capture.assert_not_called()
        database.assert_not_called()

    def test_external_binary_is_rejected_with_helpful_cli_error(self):
        import contextlib
        import io
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr), self.assertRaises(SystemExit) as caught:
            self.runner.main([*ARGUMENTS, '--binary', 'arbitrary',
                              '--output', str(self.output)])
        self.assertEqual(caught.exception.code, 2)
        self.assertIn('--binary is no longer accepted', stderr.getvalue())
        self.assertFalse(self.output.exists())
        self.mock_build.assert_not_called()

    def test_harness_manifest_includes_runner_builder_and_executed_fixture(self):
        hashes = self.runner._file_hashes(BASE)
        import hashlib
        for name in ('gm05_runner.py', 'gm05_build.py', 'product_chain_fixture.py'):
            self.assertEqual(hashes[name], hashlib.sha256((BASE / name).read_bytes()).hexdigest())

    def test_a_reference_capture_failure_still_publishes_the_summary(self):
        failure = self.runner.ReferenceCaptureFailure(
            'la captura del oráculo falló', exit_code=3,
            log=self.output / 'reference-capture.log')
        code = self.run_main(mock.Mock(side_effect=failure))
        self.assertEqual(code, 2)
        summary = self.summary()
        self.assertEqual(summary['verdict']['comparison'], 'fail')
        self.assertEqual(summary['failed_stage'], 'reference-capture')
        self.assertEqual(summary['verdict']['deviations'], [])
        self.assertEqual(summary['verdict']['errors'][0],
                         'error: ReferenceCaptureFailure: la captura del oráculo falló')
        diagnostic = json.loads((self.output / 'reference-failure.json')
                                .read_text(encoding='utf-8'))
        self.assertEqual(diagnostic['stage'], 'reference-capture')
        self.assertEqual(diagnostic['exit_code'], 3)
        self.assertEqual(diagnostic['log'], 'reference-capture.log')

    def test_the_diagnostic_keeps_the_tail_of_the_log_without_credentials(self):
        self.output.mkdir(parents=True)
        log = self.output / 'reference-capture.log'
        log.write_text('arranque\nNEO4J_AUTH=neo4j/secreto\nfinal\n', encoding='utf-8')
        failure = self.runner.ReferenceCaptureFailure('falló', exit_code=1, log=log)
        self.runner.publish_reference_failure(self.output, failure)
        diagnostic = json.loads((self.output / 'reference-failure.json')
                                .read_text(encoding='utf-8'))
        tail = '\n'.join(diagnostic['log_tail'])
        self.assertIn('arranque', tail)
        self.assertIn('final', tail)
        self.assertNotIn('secreto', tail)

    def test_an_unexpected_failure_still_publishes_the_summary(self):
        code = self.run_main(mock.Mock(side_effect=RuntimeError('base no disponible')))
        self.assertEqual(code, 2)
        summary = self.summary()
        self.assertEqual(summary['verdict']['comparison'], 'fail')
        self.assertEqual(summary['verdict']['errors'],
                         ['error: RuntimeError: base no disponible'])
        self.assertFalse((self.output / 'reference-failure.json').exists())

    def test_a_completed_capture_publishes_its_verdict(self):
        verdict = {'comparison': 'pass', 'deviations': [], 'errors': []}
        code = self.run_main(mock.Mock(return_value=({'scenario': 'gm05-v1',
                                                      'verdict': verdict}, {})))
        self.assertEqual(code, 0)
        self.assertEqual(self.summary()['verdict'], verdict)
        self.assertEqual(self.summary()['build'], self.artifact.manifest)
        self.assertEqual(self.summary()['binary_sha256'], 'binary-hash')
        self.artifact.verify.assert_called()


if __name__ == '__main__':
    unittest.main()
