"""Pruebas estáticas sintéticas; no ejecutan TCK ni modelos."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import inventory_test_traceability as trace


class TraceabilityTests(unittest.TestCase):
    def test_extracts_only_function_docstrings_and_qualified_test_names(self):
        raw = b'''"""SPEC-9.9.9"""
class TestExample:
    async def test_one(self):
        """SPEC-1.1.1: observable."""
        text = "SPEC-8.8.8"
'''
        self.assertEqual(trace.extract(raw), {'SPEC-1.1.1': 'TestExample::test_one'})

    def test_source_hash_is_checked_before_parsing(self):
        with self.assertRaisesRegex(ValueError, 'integridad'):
            trace.checked_extract(b'invalid python', '0' * 64)

    def test_duplicate_declarations_fail(self):
        raw = b'def test_a():\n """SPEC-1.1.1"""\ndef test_b():\n """SPEC-1.1.1"""\n'
        with self.assertRaises(ValueError):
            trace.extract(raw)

    def test_malformed_identifier_does_not_match_a_prefix(self):
        for value in ['SPEC-1.1.1oops', 'SPEC-1.1', 'SPEC-1.1.1.2', 'SPEC-01.1.1']:
            with self.subTest(value=value), self.assertRaises(ValueError):
                trace.extract(f'def test_a():\n """{value}"""\n'.encode())

    def test_duplicate_test_nodes_fail(self):
        raw = b'def test_a():\n """SPEC-1.1.1"""\ndef test_a():\n """SPEC-1.1.2"""\n'
        with self.assertRaises(ValueError):
            trace.extract(raw)

    def test_duplicate_classes_fail_even_with_distinct_methods(self):
        raw = b'class TestA:\n def test_a(self):\n  """SPEC-1.1.1"""\nclass TestA:\n def test_b(self):\n  """SPEC-1.1.2"""\n'
        with self.assertRaises(ValueError):
            trace.extract(raw)

    def test_duplicate_unmapped_declarations_across_files_fail(self):
        commit = 'e60b410f1046dc21db50f9aba48e171dc8a9b5a1'
        duplicate = b'def test_unmapped():\n """SPEC-9.9.9"""\n'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source_root = root / 'dev/muninn/.cache/sources/agent-memory-tck'
            integrity_files = {}
            for index, name in enumerate(trace.FILES):
                path = 'tck/tests/v1/' + name
                raw = duplicate if index < 2 else b''
                target = source_root / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(raw)
                integrity_files[path] = hashlib.sha256(raw).hexdigest()
            (root / 'dev/muninn').mkdir(parents=True, exist_ok=True)
            integrity = {'agent-memory-tck': {'commit': commit, 'files': integrity_files}}
            references = {'agent-memory-tck': {'commit': commit}}
            (root / 'dev/muninn/source-integrity.json').write_text(json.dumps(integrity))
            (root / 'dev/muninn/references.json').write_text(json.dumps(references))
            matrix = root / 'docs/muninn/specs'
            matrix.mkdir(parents=True)
            (matrix / 'parity-matrix.csv').write_text('id,source_ref\napi:example,\n')
            with self.assertRaisesRegex(ValueError, 'duplicada'):
                trace.build(root)

    def test_non_test_helpers_do_not_supply_links(self):
        self.assertEqual(trace.extract(b'def helper():\n """SPEC-1.1.1"""\n'), {})

    def test_report_is_deterministic_and_does_not_claim_execution(self):
        report = trace.build()
        self.assertEqual(trace.serialized(report), trace.serialized(trace.build()))
        self.assertEqual(report['summary'], {'declared_links': 145, 'inventory_rows': 1335,
                                             'unresolved_rows': 1190})
        self.assertEqual(report['execution'], 'not_run')
        self.assertFalse(report['parity_claim'])
        self.assertTrue(report['unmapped_declarations'])
        for entry in report['entries']:
            if entry['tests']:
                self.assertEqual(entry['status'], 'upstream_declared_not_run')
                self.assertTrue(entry['id'].startswith('SPEC-'))
            else:
                self.assertEqual(entry['status'], 'unresolved')
                self.assertTrue(entry['gap_reason'])

    def test_checked_extract_accepts_verified_bytes(self):
        raw = b'def test_a():\n """SPEC-1.1.1"""\n'
        self.assertEqual(trace.checked_extract(raw, hashlib.sha256(raw).hexdigest()),
                         {'SPEC-1.1.1': 'test_a'})


if __name__ == '__main__':
    unittest.main()
