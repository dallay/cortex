"""Contract tests only: no model construction or real embeddings."""
import hashlib
import io
import json
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

import export_minilm_c1 as export


def corpus():
    return {"corpus_id": "c1-test", "batches": [{"id": "mixed", "texts": ["a", "longer text"]}]}


class ExportTests(unittest.TestCase):
    def test_accepts_only_the_c1_corpus_family(self):
        for value in ['c1', 'c1-test', 'c1_test', 'c10']:
            obj = corpus(); obj['corpus_id'] = value
            raw = json.dumps(obj).encode()
            parsed, digest = export.read_corpus(io.BytesIO(raw))
            self.assertEqual(parsed['corpus_id'], value)
            self.assertEqual(digest, hashlib.sha256(raw).hexdigest())
        for value in ['c0', 'c0-test', 'c0_test', 'c2-test', 'C1-test', 'x-c1', 'c1/test', 'c1é', '']:
            obj = corpus(); obj['corpus_id'] = value
            with self.subTest(corpus_id=value), self.assertRaises(export.RunError):
                export.read_corpus(io.BytesIO(json.dumps(obj).encode()))

    def test_hash_exact_bytes_and_limits(self):
        raw = json.dumps(corpus()).encode()
        parsed, digest = export.read_corpus(io.BytesIO(raw))
        self.assertEqual(parsed, corpus())
        self.assertEqual(digest, hashlib.sha256(raw).hexdigest())
        export.read_corpus(io.BytesIO(raw + b" " * (export.INPUT_LIMIT - len(raw))))
        with self.assertRaises(export.RunError):
            export.read_corpus(io.BytesIO(raw + b" " * (export.INPUT_LIMIT + 1 - len(raw))))

    def test_strict_input(self):
        bad = [b'{"corpus_id":"c1","corpus_id":"c1","batches":[]}', b'NaN', b'Infinity', b'[]', b'{}', b'\xff']
        for value in [True, 1, None, {}, []]:
            obj = corpus(); obj['batches'][0]['texts'] = [value]
            bad.append(json.dumps(obj).encode())
        for mutate in [lambda c: c.update(extra=1), lambda c: c.update(corpus_id='C1'), lambda c: c.update(corpus_id='c1'+'x'*63), lambda c: c.update(batches=[]), lambda c: c.update(batches=c['batches']*2), lambda c: c['batches'][0].update(extra=0), lambda c: c['batches'][0].update(texts=['é'*2049]), lambda c: c['batches'][0].update(texts=['']*33)]:
            obj = corpus(); mutate(obj); bad.append(json.dumps(obj).encode())
        for raw in bad:
            with self.subTest(raw=raw[:80]), self.assertRaises(export.RunError):
                export.read_corpus(io.BytesIO(raw))

    def test_utf8_boundary_and_empty_text(self):
        obj = corpus(); obj['batches'][0]['texts'] = ['', 'é'*2048]
        export.read_corpus(io.BytesIO(json.dumps(obj).encode()))

    def test_invalid_input_precedes_construction(self):
        with patch.object(export, 'construct_cpu_reference') as build, patch('builtins.open', return_value=io.BytesIO(b'{}')):
            with self.assertRaises(export.RunError):
                export.run('/bundle', '/log', '/input', 'test-unapproved-v1')
            build.assert_not_called()

    def test_tokens_original_order_and_serialization(self):
        model = Mock()
        tensors = {key: Mock(tolist=Mock(return_value=value)) for key, value in {
            'input_ids': [[101, 1, 0], [101, 2, 3]], 'attention_mask': [[1, 1, 0], [1, 1, 1]], 'token_type_ids': [[0, 0, 0], [0, 0, 0]]}.items()}
        model.tokenize.return_value = tensors
        # Shape/serialization values, not model embeddings or numerical evidence.
        vectors = Mock(shape=(2, 384), tolist=Mock(return_value=[[0.0]*384, [1.0]*384]))
        with patch.object(export, 'encode_cpu_reference', return_value=vectors) as encode:
            batch = export.export_batch(model, corpus()['batches'][0])
        model.tokenize.assert_called_once_with(['a', 'longer text'])
        encode.assert_called_once_with(model, ['a', 'longer text'])
        self.assertEqual(batch['input_ids'], [[101, 1, 0], [101, 2, 3]])
        self.assertEqual(batch['vectors'][1], [1.0]*384)
        self.assertEqual(set(batch), {'id', 'input_ids', 'attention_mask', 'token_type_ids', 'vectors'})
        json.dumps(batch, allow_nan=False)

    def test_reject_invalid_vectors_and_tokens(self):
        model = Mock()
        model.tokenize.return_value = {k: Mock(tolist=Mock(return_value=[[1], [1]])) for k in ('input_ids', 'attention_mask', 'token_type_ids')}
        for rows in [[[0.0]*383]*2, [[float('nan')]*384]*2, [[True]*384]*2, [[float('inf')]*384]*2]:
            with patch.object(export, 'encode_cpu_reference', return_value=Mock(shape=(2, 384), tolist=Mock(return_value=rows))):
                with self.assertRaises(export.RunError): export.export_batch(model, corpus()['batches'][0])
        model.tokenize.return_value['input_ids'].tolist.return_value = [[True], [1]]
        with patch.object(export, 'encode_cpu_reference') as encode:
            with self.assertRaises(export.RunError): export.export_batch(model, corpus()['batches'][0])
            encode.assert_not_called()

    def test_receipt_separate_actual_versions(self):
        model = Mock(); model.max_seq_length = 256; model.training = False
        model.__getitem__ = Mock(return_value=SimpleNamespace(auto_model=SimpleNamespace(config=SimpleNamespace(_attn_implementation='eager'))))
        with patch.object(export.metadata, 'version', side_effect=lambda name: 'actual-'+name):
            receipt = export.runtime_receipt(model)
        self.assertEqual(receipt['packages']['torch'], 'actual-torch')
        self.assertEqual(receipt['device'], 'cpu')
        self.assertNotIn('rust', json.dumps(receipt).lower())

    def test_run_envelope_and_log_validation_before_build(self):
        raw = json.dumps(corpus()).encode()
        event = {'event': 'bundle_verified', 'bundle_id': export.BUNDLE_ID,
                 'files': 11, 'artifact_bytes': 91578415,
                 'revision': '1110a243fdf4706b3f48f1d95db1a4f5529b4d41',
                 'inference_executed': False}
        for log in [b'{}', b'{"event":1,"event":2}', b'NaN', b'x'*(export.INPUT_LIMIT+1)]:
            with patch('builtins.open', side_effect=[io.BytesIO(raw), io.BytesIO(log)]), patch.object(export, 'construct_cpu_reference') as build:
                with self.assertRaises(export.RunError): export.run('/bundle', '/log', '/input', 'test-unapproved-v1')
                build.assert_not_called()
        with patch('builtins.open', side_effect=[io.BytesIO(raw), io.BytesIO(json.dumps(event).encode())]), patch.object(export, 'construct_cpu_reference') as build, patch.object(export, 'runtime_receipt', return_value={'actual': True}), patch.object(export, 'export_batch', return_value={'id': 'mixed'}):
            result = export.run('/bundle', '/log', '/input', 'test-unapproved-v1')
        build.assert_called_once()
        self.assertEqual(result['execution'], 'reference_cpu_f32')
        self.assertEqual(result['token_source'], 'independent_recomputed_same_verified_cache')
        self.assertEqual(result['input_sha256'], hashlib.sha256(raw).hexdigest())
        self.assertEqual(result['actual_runtime'], {'actual': True})
        self.assertEqual(set(result), {'identity', 'schema_version', 'corpus_id', 'input_sha256', 'artifact_bundle_id', 'execution', 'token_source', 'actual_runtime', 'batches'})

    def test_real_structural_ndjson_passes_raw_to_constructor(self):
        log = (Path(__file__).resolve().parents[1] / 'fixtures' /
               'minilm-c0-verification.ndjson').read_bytes()
        lines = log.decode('utf-8').splitlines()
        self.assertEqual(len(lines), 12)
        self.assertTrue(all(json.loads(line)['event'] == 'cache_artifact_verified'
                            for line in lines[:-1]))
        self.assertEqual(json.loads(lines[-1])['event'], 'bundle_verified')
        with patch('builtins.open', side_effect=[io.BytesIO(json.dumps(corpus()).encode()), io.BytesIO(log)]), patch.object(export, 'construct_cpu_reference') as build, patch.object(export, 'runtime_receipt', return_value={}), patch.object(export, 'export_batch', return_value={}):
            result = export.run('/bundle', '/log', '/input', 'test-unapproved-v1')
        build.assert_called_once_with('/bundle', 0, log.decode('utf-8'))
        self.assertEqual(result['execution'], 'reference_cpu_f32')

    def test_verification_bounded_utf8_read_before_constructor(self):
        class BoundedReader(io.BytesIO):
            def read(self, size=-1):
                if size != 65537:
                    raise AssertionError(f'Unexpected verification read: {size}')
                return super().read(size)

        valid = (Path(__file__).resolve().parents[1] / 'fixtures' /
                 'minilm-c0-verification.ndjson').read_bytes().rstrip(b'\n')
        cases = [valid + b' ' * (65536 - len(valid)),
                 valid + b' ' * (65537 - len(valid)), valid + b'\xff',
                 valid + b'\n{}',
                 valid.replace(b'"files":11', b'"files":11,"files":11'),
                 valid.replace(b'"artifact_bytes":91578415', b'"artifact_bytes":NaN')]
        for index, log in enumerate(cases):
            with self.subTest(index=index), patch('builtins.open', side_effect=[io.BytesIO(json.dumps(corpus()).encode()), BoundedReader(log)]), patch.object(export, 'construct_cpu_reference') as build, patch.object(export, 'runtime_receipt', return_value={}), patch.object(export, 'export_batch', return_value={}):
                if index == 0:
                    export.run('/bundle', '/log', '/input', 'test-unapproved-v1')
                    build.assert_called_once_with('/bundle', 0, log.decode('utf-8'))
                else:
                    with self.assertRaisesRegex(export.RunError, '^artifact$'):
                        export.run('/bundle', '/log', '/input', 'test-unapproved-v1')
                    build.assert_not_called()

    def test_contract_rejected_before_input_or_model(self):
        for contract in (None, '', 'X', 'bad contract', 'a'*65):
            with self.subTest(contract=contract), patch('builtins.open') as opened, patch.object(export, 'construct_cpu_reference') as build:
                with self.assertRaises(export.RunError):
                    export.run('/bundle', '/log', '/input', contract)
                opened.assert_not_called()
                build.assert_not_called()

    def test_cli_errors_are_fixed(self):
        for args, category in [([], 'usage'), (['--numeric-contract-version','test-unapproved-v1','--bundle','x','--verification-log','y','--input','z'], 'input')]:
            out, err = io.StringIO(), io.StringIO()
            with patch.object(export, 'run', side_effect=export.RunError('input')), patch('sys.stderr', err), patch('sys.stdout', out):
                self.assertEqual(export.main(args), 1)
            self.assertEqual(out.getvalue(), '')
            self.assertEqual(err.getvalue(), 'C1 reference export failed: '+category+'\n')


if __name__ == '__main__': unittest.main()
