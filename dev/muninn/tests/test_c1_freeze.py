"""Contract tests for the pre-execution freeze manifest.

The tests use a temporary workspace with synthetic inputs. They prove the
manifest logic, not the frozen C1 artifacts themselves.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest

import c1_freeze as freeze

REQUIRED = ['corpus.json', 'labels.json', 'policy.json']


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class FreezeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for name, content in (('corpus.json', b'{"corpus_id":"x"}'),
                              ('labels.json', b'{"schema_version":1}'),
                              ('policy.json', b'{"version":"minilm-cpu-f32-v1"}')):
            (self.root / name).write_bytes(content)
        self.inputs = [self.root / name for name in REQUIRED]

    def tearDown(self):
        self.temp.cleanup()

    def test_manifest_records_relative_paths_sizes_and_hashes(self):
        manifest = freeze.build(self.root, self.inputs, contract='minilm-cpu-f32-v1')
        self.assertEqual(manifest['state'], 'frozen_pre_execution')
        self.assertEqual(manifest['numeric_contract_version'], 'minilm-cpu-f32-v1')
        self.assertEqual([entry['path'] for entry in manifest['inputs']], REQUIRED)
        for entry in manifest['inputs']:
            self.assertEqual(entry['sha256'], sha256(self.root / entry['path']))
            self.assertEqual(entry['size'], (self.root / entry['path']).stat().st_size)

    def test_verify_detects_a_changed_input(self):
        manifest = freeze.build(self.root, self.inputs, contract='minilm-cpu-f32-v1')
        (self.root / 'corpus.json').write_bytes(b'{"corpus_id":"y"}')
        with self.assertRaises(freeze.FreezeError):
            freeze.verify(self.root, manifest)

    def test_verify_detects_a_removed_input(self):
        manifest = freeze.build(self.root, self.inputs, contract='minilm-cpu-f32-v1')
        (self.root / 'labels.json').unlink()
        with self.assertRaises(freeze.FreezeError):
            freeze.verify(self.root, manifest)

    def test_manifest_must_not_follow_symlinks(self):
        target = self.root / 'corpus.json'
        link = self.root / 'link.json'
        link.symlink_to(target)
        with self.assertRaises(freeze.FreezeError):
            freeze.build(self.root, [link, self.root / 'labels.json',
                                     self.root / 'policy.json'],
                         contract='minilm-cpu-f32-v1')

    def test_output_must_not_overwrite_an_existing_manifest(self):
        output = self.root / 'manifest.json'
        output.write_bytes(b'{}')
        with self.assertRaises(freeze.FreezeError):
            freeze.write_manifest(output, freeze.build(self.root, self.inputs,
                                                       contract='minilm-cpu-f32-v1'))

    def test_contract_version_is_required_and_validated(self):
        with self.assertRaises(freeze.FreezeError):
            freeze.build(self.root, self.inputs, contract='not a valid contract')
        with self.assertRaises(freeze.FreezeError):
            freeze.build(self.root, self.inputs, contract='')

    def test_inputs_must_be_relative_to_the_workspace(self):
        outside = Path(self.temp.name).parent / 'outside-freeze-input.json'
        outside.write_bytes(b'{}')
        try:
            with self.assertRaises(freeze.FreezeError):
                freeze.build(self.root, [outside, self.root / 'labels.json',
                                         self.root / 'policy.json'],
                             contract='minilm-cpu-f32-v1')
        finally:
            outside.unlink()

    def test_manifest_is_deterministic_for_the_same_inputs(self):
        first = freeze.build(self.root, self.inputs, contract='minilm-cpu-f32-v1')
        second = freeze.build(self.root, self.inputs, contract='minilm-cpu-f32-v1')
        self.assertEqual(first, second)

    def test_verify_mode_requires_neither_contract_nor_inputs(self):
        manifest = freeze.build(self.root, self.inputs, contract='minilm-cpu-f32-v1')
        output = self.root / 'manifest.json'
        freeze.write_manifest(output, manifest)
        code = freeze.main(['--workspace', str(self.root), '--verify', str(output)])
        self.assertEqual(code, 0)

    def test_build_mode_without_contract_fails_closed(self):
        code = freeze.main(['--workspace', str(self.root),
                            '--input', str(self.inputs[0])])
        self.assertEqual(code, 1)


if __name__ == '__main__':
    unittest.main()
