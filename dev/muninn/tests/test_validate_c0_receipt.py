"""Synthetic local receipts only; no inference or fixture changes."""
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import validate_c0_receipt as receipt


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        before = self.file('fixtures/minilm-c0-v1.json', b'{}')
        self.report = {'state': 'measured_not_accepted', 'corpus_id': 'c0-v1',
                       'input_sha256': before['sha256']}
        outputs = [dict(self.file(role + '.json', json.dumps(self.report).encode()
                                  if role == 'comparison' else b'{}'), role=role)
                   for role in ('rust_export', 'python_export', 'comparison', 'bundle_verify')]
        self.data = dict(schema_version=1, run_id='run40',
                         scope='c0_local_provenance_not_authentication', numerical_acceptance=False,
                         files_before=[before], files_after=[copy.deepcopy(before)], outputs=outputs,
                         steps=[dict(id=s, argv=['not-executed', '; touch sentinel'], exit_code=0)
                                for s in ('build', 'rust_export', 'bundle_verify', 'python_export', 'compare')],
                         runtime=dict(rustc='claimed', cargo='claimed', host='claimed'), bundle_id='a' * 64)

    def file(self, name, raw):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(raw)
        return dict(path=name, size=len(raw), sha256=hashlib.sha256(raw).hexdigest())

    def validate(self, data=None):
        self.file('receipt.json', json.dumps(self.data if data is None else data).encode())
        return receipt.validate(self.root, 'receipt.json')

    def test_valid_and_no_command_execution(self):
        self.assertEqual(self.validate()['state'], 'valid_provenance_not_acceptance')
        self.assertFalse((self.root / 'sentinel').exists())

    def test_schema_mutations(self):
        mutations = [dict(schema_version=True), dict(numerical_acceptance=0), dict(scope='accepted'),
                     dict(run_id=''), dict(bundle_id='g' * 64), dict(runtime={}), dict(extra=1)]
        for mutation in mutations:
            with self.subTest(mutation=mutation), self.assertRaises(receipt.ValidationError):
                self.validate({**self.data, **mutation})

    def test_manifest_mutations(self):
        for field in ('files_before', 'files_after', 'outputs'):
            for value in ([], self.data[field] * 2):
                with self.subTest(field=field), self.assertRaises(receipt.ValidationError):
                    self.validate({**self.data, field: value})
        for key, value in [('size', True), ('size', -1), ('size', receipt.FILE_LIMIT + 1),
                           ('sha256', 'x' * 64), ('sha256', '0'), ('path', '../escape'),
                           ('path', '/absolute'), ('path', './comparison.json'),
                           ('path', 'a//b'), ('path', 'a\\b'), ('path', '\x00')]:
            data = copy.deepcopy(self.data)
            data['outputs'][0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(receipt.ValidationError):
                self.validate(data)

    def test_changed_missing_and_directory_files(self):
        for name in ('rust_export.json', 'fixtures/minilm-c0-v1.json'):
            path = self.root / name
            original = path.read_bytes()
            path.write_bytes(b'changed')
            with self.assertRaises(receipt.ValidationError):
                self.validate()
            path.unlink()
            with self.assertRaises(receipt.ValidationError):
                self.validate()
            path.mkdir()
            with self.assertRaises(receipt.ValidationError):
                self.validate()
            path.rmdir()
            path.write_bytes(original)

    def test_symlink_components_and_receipt(self):
        (self.root / 'alias').symlink_to('fixtures', target_is_directory=True)
        data = copy.deepcopy(self.data)
        for field in ('files_before', 'files_after'):
            data[field][0]['path'] = 'alias/minilm-c0-v1.json'
        with self.assertRaises(receipt.ValidationError):
            self.validate(data)
        self.validate()
        (self.root / 'link.json').symlink_to('receipt.json')
        with self.assertRaises(receipt.ValidationError):
            receipt.validate(self.root, 'link.json')

    def test_before_after_must_match(self):
        data = copy.deepcopy(self.data)
        data['files_before'][0]['sha256'] = 'b' * 64
        with self.assertRaises(receipt.ValidationError):
            self.validate(data)

    def test_required_steps_and_roles(self):
        for field in ('steps', 'outputs'):
            data = copy.deepcopy(self.data)
            data[field].pop()
            with self.assertRaises(receipt.ValidationError):
                self.validate(data)
        for value in (True, 1, -1, 0.0):
            data = copy.deepcopy(self.data)
            data['steps'][0]['exit_code'] = value
            with self.assertRaises(receipt.ValidationError):
                self.validate(data)
        data = copy.deepcopy(self.data)
        data['steps'].append(dict(id='tests', argv=['untrusted'], exit_code=0))
        self.validate(data)
        data['steps'].append(data['steps'][0])
        with self.assertRaises(receipt.ValidationError):
            self.validate(data)
        data = copy.deepcopy(self.data)
        data['outputs'][0]['role'] = 'comparison'
        with self.assertRaises(receipt.ValidationError):
            self.validate(data)

    def test_comparison_contract(self):
        for key, value in [('state', 'accepted'), ('corpus_id', 'wrong'), ('input_sha256', 'f' * 64)]:
            report = {**self.report, key: value}
            self.data['outputs'][2].update(self.file('comparison.json', json.dumps(report).encode()))
            with self.subTest(key=key), self.assertRaises(receipt.ValidationError):
                self.validate()

    def test_json_limits_duplicates_and_nonfinite(self):
        for raw in (b'{"x":1,"x":2}', b'{"x":NaN}', b'{"x":Infinity}', b'{"x":1e999}',
                    b'[' * 2000, b'\xff', b' ' * (receipt.JSON_LIMIT + 1)):
            self.file('receipt.json', raw)
            with self.subTest(raw=raw[:30]), self.assertRaises(receipt.ValidationError):
                receipt.validate(self.root, 'receipt.json')

    def test_resource_limits(self):
        with patch.object(receipt, 'MAX_FILES', 5), self.assertRaises(receipt.ValidationError):
            self.validate()
        with patch.object(receipt, 'FILE_LIMIT', 1), self.assertRaises(receipt.ValidationError):
            self.validate()

    def test_cli_fixed_error(self):
        out = io.StringIO()
        with redirect_stdout(out):
            code = receipt.main(['--workspace', str(self.root), '--receipt', 'missing.json'])
        self.assertEqual(code, 1)
        self.assertEqual(json.loads(out.getvalue()), {'state': 'invalid_receipt', 'error': 'file_access'})


if __name__ == '__main__':
    unittest.main()
