"""Bridge startup diagnostics with stdlib doubles only."""

import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import gm01_runner as runner


class StartupTests(unittest.TestCase):
    def fixture(self, private):
        return runner.OwnedFixture(
            Mock(), private, runner.Database('bolt://rust-db:7687', 'secret-marker'))

    def test_early_exit_is_categorized_without_log_or_password(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(Path(temporary))
            process = Mock()
            process.poll.return_value = 7

            def spawn(*args, **kwargs):
                kwargs['stdout'].write(b'failure secret-marker\n')
                kwargs['stdout'].flush()
                return process

            with patch.object(runner.subprocess, 'Popen', side_effect=spawn):
                with self.assertRaises(runner.StartupFailure) as raised:
                    fixture.start()
            self.assertEqual(raised.exception.diagnostic, {
                'stage': 'bridge_readiness', 'cause': 'early_exit',
                'exit_code': 7, 'last_http_status': None,
                'last_probe': 'not_attempted', 'budget_seconds': 180})
            self.assertNotIn('secret-marker', str(raised.exception))
            self.assertNotIn('secret-marker', fixture.process_log.read_text())
            self.assertEqual(fixture.process_log.stat().st_mode & 0o777, 0o600)
            fixture.close()

    def test_timeout_polls_process_and_records_http_status(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = self.fixture(Path(temporary))
            process = Mock()
            process.poll.return_value = None
            with patch.object(runner.subprocess, 'Popen', return_value=process), \
                 patch.object(fixture, '_request', return_value=(503, {})), \
                 patch.object(runner.time, 'monotonic', side_effect=[0, 0, 1, 181]), \
                 patch.object(runner.time, 'sleep'):
                with self.assertRaises(runner.StartupFailure) as raised:
                    fixture.start()
            self.assertEqual(raised.exception.diagnostic['cause'], 'timeout')
            self.assertEqual(raised.exception.diagnostic['last_http_status'], 503)
            self.assertGreaterEqual(process.poll.call_count, 2)
            fixture.close()

    def test_database_and_binary_failures_have_distinct_safe_stages(self):
        for stage in ('database_connection', 'bridge_process'):
            with self.subTest(stage=stage), tempfile.TemporaryDirectory() as temporary:
                fixture = self.fixture(Path(temporary))
                if stage == 'database_connection':
                    fixture.database.active = False
                else:
                    fixture.artifact.verify.side_effect = RuntimeError('secret-marker')
                with self.assertRaises(runner.StartupFailure) as raised:
                    fixture.start()
                self.assertEqual(raised.exception.diagnostic,
                                 {'stage': stage, 'cause': 'operation_failed'})
                self.assertNotIn('secret-marker', str(raised.exception))
                fixture.close()

    def test_preparation_failure_manifest_omits_exception_text_and_never_accepts_h1(self):
        with tempfile.TemporaryDirectory() as temporary:
            evidence = Path(temporary) / 'new-evidence'
            with patch.object(runner, 'prepare_reference',
                              side_effect=RuntimeError('secret-marker')):
                with self.assertRaises(RuntimeError):
                    asyncio.run(runner.run(Path('/reference'), evidence))
            manifest = json.loads((evidence / 'manifest.json').read_text())
            self.assertEqual(manifest['stage'], 'prepare_reference')
            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['error_type'], 'RuntimeError')
            self.assertEqual(manifest['h1_acceptance'], 'not_established')
            self.assertNotIn('secret-marker', json.dumps(manifest))


if __name__ == '__main__':
    unittest.main()
