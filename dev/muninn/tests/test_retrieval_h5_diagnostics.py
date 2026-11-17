"""Offline diagnostics contracts; synthetic vectors only."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import retrieval_h5_diagnostics as diagnostics


def corpus():
    return {"schema_version": "1.0", "corpus_id": "test", "queries": [
        {"id": "q", "text": "PRIVATE QUERY", "candidates": [
            {"id": "a", "text": "PRIVATE A"},
            {"id": "b", "text": "PRIVATE B"},
            {"id": "c", "text": "PRIVATE C"}]}]}


class PrepareTests(unittest.TestCase):
    def test_current_pilot_fits_existing_exporter_contract(self):
        import io
        from export_minilm_c1 import read_corpus
        path = Path(__file__).resolve().parents[1] / 'fixtures/retrieval-pilot-v1-corpus.json'
        pilot, raw = diagnostics.read_json(path)
        derived, manifest = diagnostics.derive(pilot, raw)
        parsed, sha = read_corpus(io.BytesIO(diagnostics.encode(derived)))
        self.assertEqual(parsed, derived)
        self.assertEqual(sha, manifest['derived_sha256'])
        self.assertEqual(len(derived['batches']), 18)

    def test_rejects_duplicate_ids_and_oversize_text_before_output(self):
        bad = corpus()
        bad['queries'][0]['candidates'].append(bad['queries'][0]['candidates'][0])
        with self.assertRaises(ValueError):
            diagnostics.derive(bad, json.dumps(bad).encode())
        bad = corpus()
        bad['queries'][0]['text'] = 'x' * 4097
        with self.assertRaises(ValueError):
            diagnostics.derive(bad, json.dumps(bad).encode())

    def test_prepare_singletons_hashes_and_immutable_original(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source.json'
            raw = json.dumps(corpus()).encode()
            source.write_bytes(raw)
            derived, manifest = root / 'derived.json', root / 'manifest.json'
            diagnostics.prepare(source, derived, manifest)
            obj = json.loads(derived.read_bytes())
            mapping = json.loads(manifest.read_bytes())
            self.assertTrue(obj['corpus_id'].startswith('c1_h5_'))
            self.assertEqual([b['texts'] for b in obj['batches']],
                             [['PRIVATE QUERY'], ['PRIVATE A'], ['PRIVATE B'], ['PRIVATE C']])
            self.assertEqual(mapping['derived_sha256'], hashlib.sha256(derived.read_bytes()).hexdigest())
            self.assertEqual(len(mapping['items']), 4)
            self.assertNotIn('PRIVATE', manifest.read_text())
            self.assertEqual(source.read_bytes(), raw)
            with self.assertRaises(FileExistsError):
                diagnostics.prepare(source, derived, manifest)
            self.assertEqual(source.read_bytes(), raw)


def vector(x, y=0):
    return [x, y] + [0.0] * 382


class CompareTests(unittest.TestCase):
    def setUp(self):
        from c1_identity import declare
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.put('source.json', corpus())
        self.derived, self.manifest = self.root / 'derived.json', self.root / 'manifest.json'
        diagnostics.prepare(self.source, self.derived, self.manifest)
        obj = json.loads(self.derived.read_bytes())
        self.export = {'schema_version': 2, 'identity': declare('test-unapproved-v1'),
                       'corpus_id': obj['corpus_id'],
                       'input_sha256': hashlib.sha256(self.derived.read_bytes()).hexdigest(),
                       'artifact_bundle_id': declare('test-unapproved-v1')['artifact_bundle_id'],
                       'execution': 'candidate_cpu_f32', 'token_source': 'supplied',
                       'batches': [{'id': b['id'], 'input_ids': [[101, 102]],
                                    'attention_mask': [[1, 1]], 'token_type_ids': [[0, 0]],
                                    'vectors': [v]} for b, v in zip(obj['batches'],
                                     [vector(1), vector(1), vector(0, 1), vector(-1)])]}

    def put(self, name, obj):
        path = self.root / name
        path.write_text(json.dumps(obj))
        return path

    def compare(self, historical=None, python_mutate=None):
        rust = self.put('rust.json', self.export)
        python = json.loads(json.dumps(self.export))
        python['execution'] = 'reference_cpu_f32'
        python['actual_runtime'] = {'private_payload': 'PRIVATE RUNTIME'}
        if python_mutate:
            python_mutate(python)
        py = self.put('python.json', python)
        paths = [self.source, self.derived, self.manifest, rust, py]
        before = [p.read_bytes() for p in paths]
        result = diagnostics.compare(*paths, historical=historical)
        self.assertEqual(before, [p.read_bytes() for p in paths])
        return result

    def test_known_vectors_ranks_and_no_payload_leakage(self):
        report = self.compare()
        self.assertEqual(report['embedding_differences'][0]['max_abs_error'], 0)
        self.assertEqual(report['embedding_differences'][0]['cosine'], 1)
        self.assertEqual(report['embedding_differences'][0]['norm_difference'], 0)
        ranks = report['raw_cosine']['rust'][0]
        self.assertEqual(ranks['ordered_ids'], ['a', 'b', 'c'])
        self.assertEqual([x['score'] for x in ranks['scores']], [1, 0, -1])
        self.assertEqual(ranks['adjacent_margins'], [1, 1])
        self.assertEqual(ranks['exact_tie_groups'], [])
        serialized = json.dumps(report)
        for forbidden in ['PRIVATE', 'vectors', 'input_ids', 'actual_runtime', 'PASS']:
            self.assertNotIn(forbidden, serialized)
        self.assertEqual(report['score_comparison'], 'blocked/unverified-scale')

    def test_identical_subnormal_exports_preserve_unit_cosine(self):
        for batch in self.export['batches']:
            batch['vectors'] = [[5e-324] * 384]
        report = self.compare()
        for difference in report['embedding_differences']:
            self.assertAlmostEqual(difference['cosine'], 1.0, places=14)
            self.assertEqual(difference['max_abs_error'], 0)
            self.assertEqual(difference['norm_difference'], 0)
        for runtime in ('rust', 'python'):
            ranking = report['raw_cosine'][runtime][0]
            for score in ranking['scores']:
                self.assertAlmostEqual(score['score'], 1.0, places=14)
            self.assertEqual(ranking['adjacent_margins'], [0, 0])
            self.assertEqual(ranking['exact_tie_groups'], [['a', 'b', 'c']])

    def test_cross_runtime_differences_and_incompatible_identity(self):
        def changed_vectors(obj):
            obj['batches'][0]['vectors'] = [vector(0, 2)]
        report = self.compare(python_mutate=changed_vectors)
        difference = report['embedding_differences'][0]
        self.assertEqual(difference['max_abs_error'], 2)
        self.assertEqual(difference['cosine'], 0)
        self.assertEqual(difference['norm_difference'], 1)
        self.assertEqual(report['raw_cosine']['python'][0]['ordered_ids'], ['b', 'a', 'c'])
        from c1_identity import declare
        with self.assertRaises(ValueError):
            self.compare(python_mutate=lambda obj: obj.update(identity=declare('other-v1')))

    def test_cli_preserves_existing_output_and_sanitizes_errors(self):
        from contextlib import redirect_stderr
        import io
        self.compare()
        output = self.put('report.json', {'original': True})
        args = ['compare', '--corpus', str(self.source), '--derived', str(self.derived),
                '--manifest', str(self.manifest), '--rust', str(self.root / 'rust.json'),
                '--python', str(self.root / 'python.json'), '--output', str(output)]
        with redirect_stderr(io.StringIO()) as stderr:
            self.assertEqual(diagnostics.main(args), 1)
        self.assertNotIn(str(self.root), stderr.getvalue())
        self.assertEqual(json.loads(output.read_bytes()), {'original': True})
        args[-1] = str(self.root / 'new-report.json')
        self.assertEqual(diagnostics.main(args), 0)
        self.assertNotIn('PRIVATE', (self.root / 'new-report.json').read_text())

    def test_difference_and_exact_ties(self):
        self.export['batches'][2]['vectors'] = [vector(1)]
        report = self.compare()
        self.assertEqual(report['raw_cosine']['rust'][0]['exact_tie_groups'], [['a', 'b']])
        self.assertEqual(report['raw_cosine']['rust'][0]['adjacent_margins'], [0, 2])

    def test_historical_orders_not_score_deltas(self):
        from retrieval_quality import corpus_hash
        history = {'provenance': {'corpus_id': 'test', 'corpus_hash': corpus_hash(corpus())},
                   'per_query': [{'query_id': 'q', 'metrics': {
                       'ordered_ids': ['c', 'a', 'b'], 'ordered_scores': [0.9, 0.8, 0.7]}}]}
        path = self.put('history.json', history)
        report = self.compare(path)
        item = report['historical_orders'][0]
        self.assertEqual(item['ordered_ids'], ['c', 'a', 'b'])
        self.assertFalse(item['matches_rust_replay'])
        self.assertEqual(item['score_observations'], [0.9, 0.8, 0.7])
        history['per_query'][0]['metrics']['ordered_ids'] = ['a', 'a', 'c']
        self.put('history.json', history)
        with self.assertRaises(ValueError):
            self.compare(path)

    def test_rejects_invalid_exports(self):
        import copy
        original = copy.deepcopy(self.export)
        changes = [
            lambda e: e.update(input_sha256='0' * 64),
            lambda e: e.update(corpus_id='other'),
            lambda e: e.update(identity={}),
            lambda e: e.update(artifact_bundle_id='wrong'),
            lambda e: e.update(batches=e['batches'][:-1]),
            lambda e: e['batches'].append(e['batches'][0]),
            lambda e: e['batches'][0].update(id='unknown'),
            lambda e: e['batches'][0].update(vectors=[vector(0)]),
            lambda e: e['batches'][0].update(vectors=[[1] * 383]),
            lambda e: e['batches'][0].update(vectors=[[True] * 384]),
            lambda e: e['batches'][0]['vectors'][0].__setitem__(0, float('nan')),
            lambda e: e['batches'][0]['vectors'][0].__setitem__(0, float('inf')),
            lambda e: e['batches'][0].update(attention_mask=[[2, 1]]),
        ]
        for change in changes:
            self.export = copy.deepcopy(original)
            change(self.export)
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.compare()

    def test_rejects_changed_text_manifest_and_duplicate_json_keys(self):
        derived = json.loads(self.derived.read_bytes())
        derived['batches'][0]['texts'] = ['tampered']
        self.derived.write_text(json.dumps(derived))
        with self.assertRaises(ValueError):
            self.compare()
        diagnostics.prepare(self.source, self.root / 'new-derived', self.root / 'new-manifest')
        self.derived = self.root / 'new-derived'
        self.manifest.write_text('{}')
        with self.assertRaises(ValueError):
            self.compare()
        self.source.write_text('{"a":1,"a":2}')
        with self.assertRaises(ValueError):
            diagnostics.read_json(self.source)


if __name__ == '__main__':
    unittest.main()
