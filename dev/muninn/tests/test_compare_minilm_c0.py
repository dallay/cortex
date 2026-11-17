"""Mathematical basis-vector records, NOT inference or parity evidence."""
import copy
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import compare_minilm_c0 as compare


def records():
    corpus = {'corpus_id': 'c0-test', 'batches': [{'id': 'a', 'texts': ['q', 'p', 'n']}]}
    raw = json.dumps(corpus).encode()
    basis = lambda i: [float(j == i) for j in range(384)]
    batch = {'id': 'a', 'input_ids': [[101, 102]] * 3, 'attention_mask': [[1, 1]] * 3,
             'token_type_ids': [[0, 0]] * 3, 'vectors': [basis(0), basis(0), basis(1)]}
    rust = {'schema_version': 1, 'corpus_id': 'c0-test', 'input_sha256': hashlib.sha256(raw).hexdigest(),
            'artifact_bundle_id': compare.BUNDLE_ID, 'execution': 'candidate_cpu_f32',
            'token_source': 'independently_recomputed_same_verified_cache', 'batches': [batch]}
    python = copy.deepcopy(rust)
    python.update(execution='reference_cpu_f32', token_source='independent_recomputed_same_verified_cache',
                  actual_runtime={'packages': {k: 'test' for k in compare.PACKAGES}, 'python': 'test',
                                  'platform': 'test', 'device': 'cpu', 'dtype': 'float32',
                                  'attention_backend': 'test', 'model_eval': True, 'max_seq_length': 256})
    labels = {'schema_version': 1, 'corpus_id': 'c0-test', 'status': 'human_hypotheses_not_results',
              'label_origin_es': 'test', 'reference_format': 'batch_id/zero_based_index',
              'coverage': {'test': ['a/0']}, 'triplets': [{'id': 't', 'query': 'a/0', 'positive': 'a/1',
              'negative': 'a/2', 'rationale_es': 'test'}], 'length_hypotheses': [],
              'length_note_es': 'test', 'equal_text_groups': []}
    return raw, rust, python, labels


class ComparatorTests(unittest.TestCase):
    def test_basis_metrics_and_descriptive_scores(self):
        raw, rust, python, labels = records()
        rust['batches'][0]['vectors'][1] = [0.0, 1.0] + [0.0] * 382
        report = compare.compare(raw, rust, python, labels)
        self.assertEqual(report['state'], 'measured_not_accepted')
        self.assertEqual(report['metrics']['max_abs_error'], 1.0)
        self.assertEqual(report['metrics']['min_cosine'], 0.0)
        self.assertEqual(report['metrics']['max_within_batch_score_error'], 1.0)
        self.assertEqual(report['labeled_scores'][0]['python']['gap'], 1.0)
        self.assertTrue(report['labeled_scores'][0]['rust']['tie'])
        self.assertEqual(report['runtime']['python'], python['actual_runtime'])
        self.assertEqual(report['runtime']['rust']['state'], 'unknown')

    def test_token_mismatch_never_computes_metrics(self):
        raw, rust, python, labels = records()
        rust['batches'][0]['input_ids'][0][0] = 7
        with patch.object(compare, 'measure', side_effect=AssertionError('must not measure')):
            report = compare.compare(raw, rust, python, labels)
        self.assertEqual(report['state'], 'token_mismatch')
        self.assertNotIn('metrics', report)
        self.assertEqual(report['token_mismatches'][0]['field'], 'input_ids')

    def test_strict_export_validation(self):
        mutations = [lambda r: r.update(schema_version=True), lambda r: r.update(extra=1),
                     lambda r: r.update(input_sha256='0'*64), lambda r: r.update(artifact_bundle_id='0'*64),
                     lambda r: r.update(execution='reference_cpu_f32'), lambda r: r.update(token_source='captured'),
                     lambda r: r['batches'][0].update(id='wrong'),
                     lambda r: r['batches'][0]['input_ids'][0].__setitem__(0, True),
                     lambda r: r['batches'][0]['attention_mask'][0].__setitem__(0, 2),
                     lambda r: r['batches'][0]['token_type_ids'][0].__setitem__(0, 1),
                     lambda r: r['batches'][0]['vectors'][0].pop(),
                     lambda r: r['batches'][0]['vectors'][0].__setitem__(0, float('inf')),
                     lambda r: r['batches'][0]['vectors'].__setitem__(0, [0.0]*384)]
        for mutate in mutations:
            raw, rust, python, labels = records()
            mutate(rust)
            with self.subTest(mutate=mutate), self.assertRaises(compare.ValidationError):
                compare.compare(raw, rust, python, labels)

    def test_duplicate_nonfinite_and_bounded_reader(self):
        for raw in [b'{"x":1,"x":2}', b'NaN', b'1e999', b'\xff']:
            with self.assertRaises(compare.ValidationError):
                compare.read_json(io.BytesIO(raw))
        with patch.object(compare, 'EXPORT_LIMIT', 8):
            with self.assertRaises(compare.ValidationError):
                compare.read_json(io.BytesIO(b' '*9))

    def test_labels_validate_all_references(self):
        raw, rust, python, labels = records()
        for section in ('coverage', 'triplets', 'length_hypotheses', 'equal_text_groups'):
            bad = copy.deepcopy(labels)
            if section == 'coverage': bad[section]['test'] = ['a/03']
            elif section == 'triplets': bad[section][0]['positive'] = 'missing/0'
            elif section == 'length_hypotheses': bad[section] = [{'ref': 'a/8', 'hello_repetitions': 1,
                'expected_encoded_length_before_truncation': 3, 'tokenizer_verified': False}]
            else: bad[section] = [['a/0', 'a/9']]
            with self.subTest(section=section), self.assertRaises(compare.ValidationError):
                compare.compare(raw, rust, python, bad)

    def test_actual_fixture_schema_without_inference(self):
        fixtures = Path(__file__).resolve().parents[1] / 'fixtures'
        corpus = compare.validate_corpus((fixtures / 'minilm-c0-v1.json').read_bytes())
        labels = compare.decode((fixtures / 'minilm-c0-v1-labels.json').read_bytes())
        compare.validate_labels(labels, corpus)
        self.assertEqual(len(labels['triplets']), 13)

    def test_cli_exit_states(self):
        raw, rust, python, labels = records()
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / name for name in ('corpus', 'rust', 'python', 'labels')]
            paths[0].write_bytes(raw)
            for path, obj in zip(paths[1:], (rust, python, labels)):
                path.write_text(json.dumps(obj))
            args = [arg for name, path in zip(('corpus', 'rust', 'python', 'labels'), paths)
                    for arg in ('--' + name, str(path))]
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(compare.main(args), 0)
            self.assertEqual(json.loads(output.getvalue())['state'], 'measured_not_accepted')
            rust['batches'][0]['attention_mask'][0][0] = 0
            paths[1].write_text(json.dumps(rust))
            output = io.StringIO()
            with redirect_stdout(output):
                self.assertEqual(compare.main(args), 1)
            self.assertNotIn('metrics', json.loads(output.getvalue()))
            paths[1].write_text('{"x":1,"x":2}')
            with redirect_stdout(io.StringIO()):
                self.assertEqual(compare.main(args), 1)

    def test_stable_extreme_norm_and_cosine(self):
        self.assertAlmostEqual(compare.cosine([1e300, 1e300], [1e300, 1e300]), 1.0)
        self.assertAlmostEqual(compare.cosine([1e-300, 0.0], [1e-300, 0.0]), 1.0)
        with self.assertRaises(compare.ValidationError):
            compare.cosine([0.0], [1.0])


if __name__ == '__main__':
    unittest.main()
