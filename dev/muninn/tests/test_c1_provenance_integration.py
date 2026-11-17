"""Verify native rejection happens before any exporter subprocess."""
import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import c1_attempt_run
import c1_runner


class NativeRejectionTests(unittest.TestCase):
    def test_missing_build_record_never_launches_exporter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'export_minilm_c1'
            binary.write_text('#!/bin/sh\nexit 99\n')
            binary.chmod(0o700)
            binding = c1_runner.BuildBinding(str(binary), hashlib.sha256(binary.read_bytes()).hexdigest(), ())
            with patch('c1_runner.subprocess.Popen') as exporter:
                with self.assertRaisesRegex(c1_runner.RunnerError, 'missing_build_provenance'):
                    c1_runner._verify_build(binding, None)
                exporter.assert_not_called()
            with patch('c1_runner.subprocess.Popen') as exporter:
                with self.assertRaisesRegex(c1_attempt_run.PlanError, 'missing_build_provenance'):
                    c1_attempt_run.build_stage(root, {}, {'name': 'native', 'binary': binary.name,
                                                        'files': [], 'source_trees': []})
                exporter.assert_not_called()

    def test_crypto_rejection_never_launches_exporter(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'export_minilm_c1'
            binary.write_bytes(b'not a model')
            binary.chmod(0o700)
            binding = c1_runner.BuildBinding(str(binary), hashlib.sha256(binary.read_bytes()).hexdigest(), (),
                                            provenance=(binary.parent / 'missing-record', binary.parent / 'missing-bundle',
                                                        'a' * 40, [], 'rustc'))
            with patch('c1_runner.subprocess.Popen') as exporter:
                with self.assertRaises(c1_runner.RunnerError):
                    c1_runner._verify_build(binding, type('Admission', (), {'root': binary.parent})())
                exporter.assert_not_called()
