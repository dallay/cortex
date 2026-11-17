"""El runner Bronze no puede interpolar rutas ambiguas en `--mount` de Docker.

Docker separa el valor de `--mount` por comas. Una ruta de operador con coma
alteraría el parseo de opciones de Docker, así que el runner debe fallar antes
de invocar `docker run`.
"""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

import bronze_isolated


def comma_path(root: Path, name: str) -> Path:
    """Construye una ruta que contiene una coma de forma determinista."""
    path = root / name
    path.mkdir(parents=True)
    return path


class BronzeMountPathTests(unittest.TestCase):
    """El armado de `docker run` rechaza rutas con coma."""

    def test_source_path_with_comma_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sources = comma_path(root, 'sources,with,commas')
            with self.assertRaises(ValueError) as raised:
                bronze_isolated.build_run_command(
                    image='image:tag', network='net',
                    base=root, sources=sources,
                    auth=root / 'neo4j_auth', evidence=root / 'evidence',
                )
        self.assertIn('coma', str(raised.exception))
        self.assertIn(str(sources), str(raised.exception))

    def test_evidence_path_with_comma_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = comma_path(root, 'evidence,with,commas')
            with self.assertRaises(ValueError) as raised:
                bronze_isolated.build_run_command(
                    image='image:tag', network='net',
                    base=root, sources=root,
                    auth=root / 'neo4j_auth', evidence=evidence,
                )
        self.assertIn('coma', str(raised.exception))

    def test_command_keeps_one_mount_option_per_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            command = bronze_isolated.build_run_command(
                image='image:tag', network='net',
                base=root, sources=root,
                auth=root / 'neo4j_auth', evidence=root / 'evidence',
            )
        self.assertIsInstance(command, list)
        self.assertEqual(command[0], 'docker')
        mounts = [command[index + 1] for index, value in enumerate(command)
                  if value == '--mount']
        self.assertEqual(len(mounts), 4)
        for mount in mounts:
            self.assertTrue(mount.startswith('type=bind,source='))
            self.assertNotIn(',,', mount)


class BronzeMainGuardTests(unittest.TestCase):
    """`main` falla antes de invocar Docker cuando una ruta es ambigua."""

    def _argv(self, root: Path, sources: Path) -> list[str]:
        gm01 = root / 'gm01'
        gm01.mkdir(parents=True, exist_ok=True)
        (gm01 / 'manifest.json').write_text('{"backend": {}}')
        return [
            'bronze_isolated.py', '--image', 'image:tag',
            '--sources', str(sources), '--gm01-evidence', str(gm01),
            '--evidence', str(root / 'evidence'),
        ]

    def test_source_path_with_comma_fails_before_any_docker_call(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sources = comma_path(root, 'sources,with,commas')
            argv = self._argv(root, sources)
            docker = Mock(return_value=Mock(returncode=0))
            with (
                patch.object(bronze_isolated.fixtures, 'docker', docker),
                patch.object(bronze_isolated.subprocess, 'run') as run,
                patch.object(sys, 'argv', argv),
            ):
                with self.assertRaises(ValueError) as raised:
                    bronze_isolated.main()
            docker.assert_not_called()
            run.assert_not_called()
        self.assertIn('coma', str(raised.exception))

    def test_auth_path_with_comma_fails_before_the_fixture_docker_run(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sources = root / 'sources'
            sources.mkdir()
            argv = self._argv(root, sources)
            docker = Mock(return_value=Mock(returncode=0))
            with (
                patch.object(bronze_isolated.fixtures, 'docker', docker),
                patch.object(bronze_isolated.fixtures, 'CACHE', root / 'cache'),
                patch.object(bronze_isolated.fixtures, 'write_auth_file',
                             return_value=root / 'cache,odd' / 'neo4j_auth-bronze'),
                patch.object(bronze_isolated.fixtures, 'start_fixture') as start_fixture,
                patch.object(bronze_isolated.subprocess, 'run') as run,
                patch.object(sys, 'argv', argv),
            ):
                (root / 'cache').mkdir()
                with self.assertRaises(ValueError) as raised:
                    bronze_isolated.main()
            start_fixture.assert_not_called()
            run.assert_not_called()
        self.assertIn('coma', str(raised.exception))


if __name__ == '__main__':
    unittest.main()
