"""The isolated Compose stack does not create host database bindings."""

from pathlib import Path
import tempfile
import unittest

import gm01_runner as runner


class DatabaseBindingTests(unittest.TestCase):
    def compose_document(self):
        root = Path('/private/gm01')
        return runner.compose_document(root / 'context', root / 'evidence',
                                       root / 'reference.auth', root / 'rust.auth',
                                       'muninn-gm01-a1', 501, 20)

    def test_database_services_have_no_published_ports(self):
        document = self.compose_document()
        self.assertNotIn('\n    ports:', document)
        self.assertNotIn('published:', document)
        self.assertIn('image: "' + runner.NEO4J_IMAGE + '"', document)
        self.assertEqual(document.count('image: "' + runner.NEO4J_IMAGE + '"'), 2)

    def test_bridge_and_databases_use_only_internal_dns(self):
        document = self.compose_document()
        self.assertIn('internal: true', document)
        self.assertIn('reference-db:', document)
        self.assertIn('rust-db:', document)
        self.assertIn('bolt://rust-db:7687', (Path(runner.__file__).parent / 'gm01_runner.py').read_text())
        self.assertIn('bolt://reference-db:7687', (Path(runner.__file__).parent / 'gm01_runner.py').read_text())

    def test_external_project_name_is_rejected(self):
        with self.assertRaises(ValueError):
            runner.compose_document(Path('/context'), Path('/evidence'), Path('/ref'),
                                    Path('/rust'), 'unrelated-project', 501, 20)

    def test_no_docker_socket_mount_is_generated(self):
        document = self.compose_document()
        self.assertNotIn('/var/run/docker.sock', document)
        self.assertNotIn('docker.sock', document)


if __name__ == '__main__':
    unittest.main()
