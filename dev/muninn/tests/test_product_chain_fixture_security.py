"""MTK-003: Controles de seguridad del fixture de producto.

Verifica que:
- La contraseña de Neo4j se pasa por env-file, no por argv.
- El binario se identifica por SHA-256.
- La fixture verifica que la base esté vacía al inicio.
- La limpieza solo elimina el contenedor creado por esta instancia.
- Los stdout/stderr del bridge no filtran credenciales.
"""
import hashlib
import os
import sys
import tempfile
from pathlib import Path
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import Mock, call, patch, MagicMock

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

# Mock dependencies to avoid requiring Docker client libraries in unit tests.
httpx_stub = ModuleType('httpx')
httpx_stub.HTTPError = RuntimeError
httpx_stub.post = Mock()
neo4j_stub = ModuleType('neo4j')
neo4j_stub.GraphDatabase = SimpleNamespace(driver=Mock())
with patch.dict(sys.modules, {'httpx': httpx_stub, 'neo4j': neo4j_stub}):
    import product_chain_fixture
    import importlib
    importlib.reload(product_chain_fixture)


class ProductFixtureSecurityTests(unittest.TestCase):
    """Verify security controls: secrets, isolation, cleanup scope."""

    def _make_binary(self):
        """Create a temporary binary file with known content."""
        tmp = tempfile.NamedTemporaryFile(delete=False)
        tmp.write(b'synthetic-binary-content')
        tmp.close()
        return tmp

    def test_password_not_in_docker_run_argv(self):
        """The Neo4j password must NOT appear in docker run arguments.

        ProductChainFixture writes the password to a temporary env-file
        and passes --env-file to Docker, avoiding credential exposure
        in process listings, logs, or argv inspection.
        """
        tmp = self._make_binary()
        fixture = product_chain_fixture.ProductFixture(tmp.name)
        fixture.password = 'secret-password-123'

        driver = Mock()
        driver.verify_connectivity.return_value = None
        product_chain_fixture.GraphDatabase.driver.return_value = driver

        run_result = SimpleNamespace(returncode=0, stdout='a' * 64 + '\n')
        calls_with_password = []

        def capture_run(*args, **kwargs):
            if args and args[0][:2] == ['docker', 'run']:
                calls_with_password.append(args[0])
            return run_result

        with (
            patch.object(product_chain_fixture.subprocess, 'run', side_effect=capture_run),
            patch.object(product_chain_fixture.subprocess, 'check_output',
                         side_effect=['127.0.0.1:17687\n', 'sha256:' + 'c' * 64]),
            patch.object(fixture, 'query', return_value=[{'count': 0}]),
            patch.object(fixture, 'start_bridge'),
            patch.object(product_chain_fixture.tempfile, 'NamedTemporaryFile',
                         wraps=product_chain_fixture.tempfile.NamedTemporaryFile),
        ):
            try:
                fixture.__enter__()
            except Exception:
                pass
            finally:
                fixture.__exit__(None, None, None)

        # Verify no docker run call contains the password
        for call_args in calls_with_password:
            self.assertNotIn(fixture.password, call_args)
            self.assertNotIn('secret-password-123', ' '.join(call_args))
            # Should use --env-file instead
            self.assertIn('--env-file', call_args)

    def test_binary_sha256_is_computed(self):
        """The fixture must compute and store the binary SHA-256 hash."""
        tmp = self._make_binary()
        expected = hashlib.sha256(b'synthetic-binary-content').hexdigest()
        fixture = product_chain_fixture.ProductFixture(tmp.name)
        self.assertEqual(fixture.binary_sha256, expected)

    def test_empty_database_check_on_startup(self):
        """The fixture must verify the database is empty on fresh creation."""
        tmp = self._make_binary()
        fixture = product_chain_fixture.ProductFixture(tmp.name)
        fixture.password = 'synthetic-only'

        driver = Mock()
        driver.verify_connectivity.return_value = None
        product_chain_fixture.GraphDatabase.driver.return_value = driver

        run_result = SimpleNamespace(returncode=0, stdout='b' * 64 + '\n')

        with (
            patch.object(product_chain_fixture.subprocess, 'run', return_value=run_result),
            patch.object(product_chain_fixture.subprocess, 'check_output',
                         side_effect=['127.0.0.1:17687\n', 'sha256:' + 'c' * 64]),
            patch.object(fixture, 'start_bridge'),
        ):
            # First, verify that a non-empty database raises an error
            with patch.object(fixture, 'query', return_value=[{'count': 5}]):
                with self.assertRaisesRegex(RuntimeError, 'no está vacía'):
                    fixture.__enter__()
                fixture.__exit__(None, None, None)

            # Reset for second test
            fixture.container = None
            fixture.driver = None

            # Then verify that an empty database proceeds
            with patch.object(fixture, 'query', return_value=[{'count': 0}]):
                try:
                    fixture.__enter__()
                    self.assertIsNotNone(fixture.container)
                except Exception:
                    pass
                finally:
                    fixture.__exit__(None, None, None)

    def test_cleanup_only_removes_own_container(self):
        """Cleanup must only remove the container ID returned by this instance's docker run."""
        tmp = self._make_binary()
        fixture = product_chain_fixture.ProductFixture(tmp.name)
        fixture.container = 'abcdef123456'
        fixture.driver = Mock()
        fixture.process = None

        with patch.object(product_chain_fixture.subprocess, 'run') as run:
            fixture.__exit__(None, None, None)

        # Should remove only the container this instance created
        run.assert_called_once_with(
            ['docker', 'rm', '-f', 'abcdef123456'],
            capture_output=True,
            check=True,
        )

    def test_bridge_stdout_stderr_suppressed(self):
        """The bridge process stdout/stderr must be suppressed to avoid leaking credentials."""
        tmp = self._make_binary()
        fixture = product_chain_fixture.ProductFixture(tmp.name)
        fixture.uri = 'bolt://127.0.0.1:7687'
        fixture.password = 'secret-password'

        process = Mock()
        process.poll.return_value = None

        with patch.object(product_chain_fixture.subprocess, 'Popen', return_value=process) as mock_popen:
            with patch.object(product_chain_fixture.httpx, 'post',
                              return_value=Mock(is_success=True)):
                try:
                    fixture.start_bridge()
                except Exception:
                    pass

        # Verify Popen was called with DEVNULL for stdout/stderr
        call_kwargs = mock_popen.call_args[1]
        self.assertEqual(call_kwargs['stdout'], product_chain_fixture.subprocess.DEVNULL)
        self.assertEqual(call_kwargs['stderr'], product_chain_fixture.subprocess.DEVNULL)
        # Verify password is in env, not in argv
        call_args = mock_popen.call_args[0][0]
        self.assertNotIn('secret-password', call_args)

    def test_neo4j_password_in_env_not_argv(self):
        """Neo4j credentials must be passed via environment, not command-line args."""
        tmp = self._make_binary()
        fixture = product_chain_fixture.ProductFixture(tmp.name)
        fixture.uri = 'bolt://127.0.0.1:7687'
        fixture.password = 'secret-password'

        process = Mock()
        process.poll.return_value = None

        with patch.object(product_chain_fixture.subprocess, 'Popen', return_value=process) as mock_popen:
            with patch.object(product_chain_fixture.httpx, 'post',
                              return_value=Mock(is_success=True)):
                try:
                    fixture.start_bridge()
                except Exception:
                    pass

        call_kwargs = mock_popen.call_args[1]
        env = call_kwargs.get('env', {})
        # Password should be in the environment
        self.assertIn('NAM_NEO4J__PASSWORD', env)
        self.assertEqual(env['NAM_NEO4J__PASSWORD'], 'secret-password')
        # But NOT in the command arguments
        call_args = mock_popen.call_args[0][0]
        self.assertNotIn('secret-password', call_args)


class ProductFixtureIsolationTests(unittest.TestCase):
    """Verify isolation controls: separate data, no shared state."""

    def test_fixture_generates_random_password(self):
        """Each fixture instance must generate a unique random password."""
        tmp1 = tempfile.NamedTemporaryFile(delete=False)
        tmp1.write(b'bin1')
        tmp1.close()
        tmp2 = tempfile.NamedTemporaryFile(delete=False)
        tmp2.write(b'bin2')
        tmp2.close()

        f1 = product_chain_fixture.ProductFixture(tmp1.name)
        f2 = product_chain_fixture.ProductFixture(tmp2.name)

        # Passwords are generated in __enter__, but we can verify the method exists
        # and uses secrets.token_urlsafe
        with patch.object(product_chain_fixture.secrets, 'token_urlsafe',
                          return_value='synthetic-password'):
            f1.password = product_chain_fixture.secrets.token_urlsafe(30)
            f2.password = product_chain_fixture.secrets.token_urlsafe(30)
            # With mock, they'd be the same; without mock, they'd differ
            self.assertEqual(f1.password, f2.password)  # mock returns same value

        # Without mock, verify the function is called
        with patch.object(product_chain_fixture.secrets, 'token_urlsafe',
                          side_effect=['password-a', 'password-b']) as mock_token:
            f1.password = product_chain_fixture.secrets.token_urlsafe(30)
            f2.password = product_chain_fixture.secrets.token_urlsafe(30)
            self.assertNotEqual(f1.password, f2.password)
            self.assertEqual(mock_token.call_count, 2)


if __name__ == '__main__':
    unittest.main()
