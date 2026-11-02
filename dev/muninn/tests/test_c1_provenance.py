"""Synthetic rejection tests, not evidence of a real GitHub attestation."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import c1_provenance as provenance


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.binary = self.root / 'export_minilm_c1'
        self.binary.write_bytes(b'not executable')
        (self.root / 'rust-toolchain.toml').write_text('[toolchain]\nchannel="1.95.0"\n')
        self.materials = [{'path': 'rust-toolchain.toml', 'size': 29,
                           'sha256': hashlib.sha256((self.root / 'rust-toolchain.toml').read_bytes()).hexdigest()}]
        self.commit = 'a' * 40
        self.record = self.root / 'build-record.json'
        self.bundle = self.root / 'bundle.jsonl'
        self.bundle.write_text('synthetic only')
        self.data = {'schema_version': 1, 'repository': provenance.REPOSITORY,
                     'workflow': provenance.WORKFLOW, 'source_ref': 'refs/heads/main',
                     'source_commit': self.commit, 'target': provenance.TARGET,
                     'package': 'muninn-c1-exporter', 'features': ['cpu-f32'],
                     'toolchain': 'rustc 1.95.0', 'materials': self.materials,
                     'binary_sha256': hashlib.sha256(self.binary.read_bytes()).hexdigest()}
        self.save()

    def save(self):
        self.record.write_text(json.dumps(self.data))

    def verify(self):
        return provenance.verify(self.binary, self.record, self.bundle, self.commit,
                                 self.materials, 'rustc 1.95.0')

    def test_missing_record(self):
        self.record.unlink()
        with self.assertRaises(provenance.ProvenanceError):
            self.verify()

    def test_unsigned_record_never_accepted(self):
        with patch('c1_provenance.subprocess.run', side_effect=OSError('no verifier')):
            with self.assertRaises(provenance.ProvenanceError):
                self.verify()

    def test_rejections(self):
        for key, value in [('source_ref', 'refs/pull/1/merge'),
                           ('source_commit', 'b' * 40),
                           ('binary_sha256', '0' * 64),
                           ('materials', []), ('workflow', 'other.yml')]:
            with self.subTest(key=key):
                original = self.data[key]
                self.data[key] = value
                self.save()
                with patch('c1_provenance.subprocess.run') as command:
                    command.return_value.returncode = 0
                    with self.assertRaises(provenance.ProvenanceError):
                        self.verify()
                self.data[key] = original

    def test_crypto_failure_for_either_subject(self):
        for results in ([1], [0, 1]):
            with self.subTest(results=results):
                from subprocess import CompletedProcess
                with patch('c1_provenance.subprocess.run', side_effect=[CompletedProcess([], x) for x in results]):
                    with self.assertRaises(provenance.ProvenanceError):
                        self.verify()

    def test_verifier_constraints_for_both_subjects(self):
        with patch('c1_provenance.subprocess.run') as command:
            command.return_value.returncode = 0
            self.verify()
        self.assertEqual(command.call_count, 2)
        for call in command.call_args_list:
            argv = call.args[0]
            self.assertIn('--deny-self-hosted-runners', argv)
            self.assertEqual(argv[argv.index('--source-ref') + 1], 'refs/heads/main')
            self.assertEqual(argv[argv.index('--source-digest') + 1], self.commit)
            self.assertEqual(argv[argv.index('--signer-workflow') + 1], provenance.WORKFLOW)
