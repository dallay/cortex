"""Cleanup must reach every owned resource after fixture failures."""
import importlib
from pathlib import Path
import sys
import tempfile
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import Mock, call, patch

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

# The lifecycle tests mock both adapters and do not require Docker client libraries.
httpx_stub = ModuleType('httpx')
httpx_stub.HTTPError = RuntimeError
httpx_stub.post = Mock()
neo4j_stub = ModuleType('neo4j')
neo4j_stub.GraphDatabase = SimpleNamespace(driver=Mock())
with patch.dict(sys.modules, {'httpx': httpx_stub, 'neo4j': neo4j_stub}):
    import product_chain_fixture
    importlib.reload(product_chain_fixture)


class ProductFixtureLifecycleTests(unittest.TestCase):
    def test_container_removal_is_attempted_when_driver_close_fails(self):
        with tempfile.NamedTemporaryFile() as binary:
            fixture = product_chain_fixture.ProductFixture(binary.name)
        fixture.container = 'a' * 64
        fixture.driver = Mock()
        fixture.driver.close.side_effect = RuntimeError('synthetic close failure')

        with patch.object(product_chain_fixture.subprocess, 'run') as run:
            with self.assertRaises(BaseExceptionGroup):
                fixture.__exit__(None, None, None)

        run.assert_called_once_with(
            ['docker', 'rm', '-f', 'a' * 64], capture_output=True, check=True,
        )
        self.assertIsNone(fixture.driver)
        self.assertIsNone(fixture.container)

    def test_failed_bridge_start_still_removes_fixture_container(self):
        with tempfile.NamedTemporaryFile() as binary:
            fixture = product_chain_fixture.ProductFixture(binary.name)
            fixture.password = 'synthetic-only'
            driver = Mock()
            process = Mock()
            process.poll.return_value = None
            driver.verify_connectivity.return_value = None
            product_chain_fixture.GraphDatabase.driver.return_value = driver

            def start_then_fail():
                fixture.process = process
                raise RuntimeError('bridge failed after process creation')

            run_result = SimpleNamespace(returncode=0, stdout='b' * 64 + '\n')
            with (
                patch.object(product_chain_fixture.subprocess, 'run', return_value=run_result) as run,
                patch.object(product_chain_fixture.subprocess, 'check_output',
                             side_effect=['127.0.0.1:17687\n', 'sha256:' + 'c' * 64]),
                patch.object(fixture, 'query', return_value=[{'count': 0}]),
                patch.object(fixture, 'start_bridge', side_effect=start_then_fail),
            ):
                with self.assertRaisesRegex(RuntimeError, 'bridge failed after process creation'):
                    fixture.__enter__()

        self.assertTrue(any(args[0][:4] == ['docker', 'rm', '-f', 'b' * 64]
                            for args, _ in run.call_args_list))
        process.terminate.assert_called_once_with()
        process.wait.assert_called_once_with(timeout=10)
        self.assertIsNone(fixture.process)
        driver.close.assert_called_once_with()
        self.assertIsNone(fixture.container)
        self.assertIsNone(fixture.driver)


if __name__ == '__main__':
    unittest.main()
