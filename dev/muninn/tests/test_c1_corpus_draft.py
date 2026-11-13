"""Validación offline del borrador C1; no tokeniza ni ejecuta modelos."""
import copy
import json
import unittest
from pathlib import Path

import c1_corpus_draft as draft
from export_minilm_c1 import read_corpus

ROOT = Path(__file__).resolve().parents[3]


class C1DraftTests(unittest.TestCase):
    def test_draft_is_exporter_compatible_and_disjoint_except_empty(self):
        report = draft.validate()
        self.assertEqual(report["shared_exact"], [""])
        self.assertEqual(report["shared_nfc"], [""])
        self.assertFalse(report["acceptance_ready"])
        with (ROOT / draft.CORPUS).open('rb') as stream:
            corpus, _ = read_corpus(stream)
        self.assertLessEqual(len(corpus["batches"]), 32)

    def test_nonempty_overlap_is_rejected(self):
        corpus = json.loads((ROOT / draft.CORPUS).read_text())
        baseline = json.loads((ROOT / draft.C0).read_text())
        nonempty = next(t for b in baseline["batches"] for t in b["texts"] if t)
        corpus["batches"][0]["texts"][0] = nonempty
        with self.assertRaisesRegex(ValueError, "solapamiento"):
            draft.validate(corpus=corpus)

    def test_nfc_overlap_is_rejected(self):
        corpus = {"corpus_id": "c1-test", "batches": [{"id": "a", "texts": ["cafe\u0301"]}]}
        baseline = {"batches": [{"texts": ["caf\u00e9"]}]}
        with self.assertRaisesRegex(ValueError, "solapamiento"):
            draft.check_disjoint(corpus, baseline)

    def test_invalid_label_reference_is_rejected(self):
        labels = json.loads((ROOT / draft.LABELS).read_text())
        labels["triplets"][0]["negative"] = 9999
        with self.assertRaisesRegex(ValueError, "índice"):
            draft.validate(labels=labels)

    def test_duplicate_batch_reference_is_not_silently_accepted(self):
        corpus = json.loads((ROOT / draft.CORPUS).read_text())
        corpus["batches"].append(copy.deepcopy(corpus["batches"][0]))
        with self.assertRaisesRegex(ValueError, "duplicado"):
            draft.validate(corpus=corpus)

    def test_supplied_objects_do_not_claim_fixture_hashes(self):
        corpus = json.loads((ROOT / draft.CORPUS).read_text())
        corpus['batches'][0]['texts'][0] += ' nueva variante'
        report = draft.validate(corpus=corpus)
        self.assertIsNone(report['corpus_sha256'])

    def test_missing_boundary_batch_is_rejected(self):
        labels = json.loads((ROOT / draft.LABELS).read_text())
        labels['boundary_batch'] = 'absent'
        with self.assertRaisesRegex(ValueError, 'frontera'):
            draft.validate(labels=labels)

    def test_agent_labels_and_pending_token_lengths_remain_explicit(self):
        labels = json.loads((ROOT / draft.LABELS).read_text())
        self.assertEqual(labels["label_origin"], "agent_generated_hypotheses_not_human_annotations")
        self.assertEqual(labels["token_lengths_status"], "not_verified")
        self.assertEqual(labels["lifecycle"], "draft_not_frozen")
        self.assertEqual(labels["boundary_targets"], [127, 128, 129, 255, 256, 257])

class SidecarStructureTests(unittest.TestCase):
    def setUp(self):
        self.corpus = json.loads((draft.ROOT / draft.CORPUS).read_text())
        self.labels = json.loads((draft.ROOT / draft.LABELS).read_text())

    def test_rejects_invalid_sidecar_schema_and_state(self):
        for key in self.labels:
            with self.subTest(missing=key):
                labels = copy.deepcopy(self.labels)
                del labels[key]
                with self.assertRaises(ValueError):
                    draft.validate(self.corpus, labels)
        for key, value in [('extra', 1), ('schema_version', True), ('schema_version', 2),
                           ('schema_version', '1'), ('corpus_id', []), ('lifecycle', None),
                           ('label_origin', 'verified'),
                           ('coverage', None), ('triplets', None),
                           ('boundary_targets', None), ('empty_overlap_exception', []),
                           ('token_lengths_status', 'verified'), ('limits', []), ('limits', [True])]:
            with self.subTest(key=key, value=value):
                labels = copy.deepcopy(self.labels)
                labels[key] = value
                with self.assertRaises(ValueError):
                    draft.validate(self.corpus, labels)
        for labels in (None, [], 'labels', 1, True):
            if labels is not None:  # None explicitly requests the default fixture.
                with self.subTest(root=labels), self.assertRaises(ValueError):
                    draft.validate(self.corpus, labels)

    def test_rejects_missing_or_malformed_coverage_declarations(self):
        for category in self.labels['coverage']:
            labels = copy.deepcopy(self.labels)
            labels['coverage'].remove(category)
            with self.subTest(missing=category), self.assertRaises(ValueError):
                draft.validate(self.corpus, labels)
        for coverage in ([], 'empty', [True], self.labels['coverage'] * 2,
                         self.labels['coverage'] + ['unrecognized']):
            labels = copy.deepcopy(self.labels)
            labels['coverage'] = coverage
            with self.subTest(coverage=coverage), self.assertRaises(ValueError):
                draft.validate(self.corpus, labels)

    def test_rejects_invalid_triplets(self):
        original = self.labels['triplets']
        cases = [[], [original[0], original[0]], [original[0]],
                 [dict(original[0], extra=0)], [dict(original[0], batch='code')],
                 [dict(original[0], query=True)], [dict(original[0], positive=1.0)],
                 [dict(original[0], negative='2')], [None], [[]]]
        for triplets in cases:
            labels = copy.deepcopy(self.labels)
            labels['triplets'] = triplets
            with self.subTest(triplets=triplets), self.assertRaises(ValueError):
                draft.validate(self.corpus, labels)

    def test_rejects_invalid_boundary_declarations(self):
        for targets in ([], None, True, [True, 128, 129, 255, 256, 257],
                        [127, 128], [127, 128, 129, 255, 256, 258],
                        [127.0, 128, 129, 255, 256, 257]):
            labels = copy.deepcopy(self.labels)
            labels['boundary_targets'] = targets
            with self.subTest(targets=targets), self.assertRaises(ValueError):
                draft.validate(self.corpus, labels)
        for batch in ('rust', [], None):
            labels = copy.deepcopy(self.labels)
            labels['boundary_batch'] = batch
            with self.subTest(batch=batch), self.assertRaises(ValueError):
                draft.validate(self.corpus, labels)
        for count in (5, 7):
            corpus = copy.deepcopy(self.corpus)
            batch = next(b for b in corpus['batches'] if b['id'] == 'boundary_candidates')
            batch['texts'] = (batch['texts'] * 2)[:count]
            with self.subTest(count=count), self.assertRaises(ValueError):
                draft.validate(corpus, self.labels)

    def test_supplied_labels_have_no_file_hash(self):
        result = draft.validate(self.corpus, self.labels)
        self.assertIsNone(result['labels_sha256'])
        self.assertFalse(result['acceptance_ready'])
        self.assertEqual(self.labels['lifecycle'], 'draft_not_frozen')

    def test_invalid_supplied_corpus_uses_controlled_errors(self):
        from export_minilm_c1 import RunError
        cases = [[], {}, {'batches': None},
                 dict(self.corpus, batches=[None]),
                 dict(self.corpus, batches=[{'id': [], 'texts': ['new']}]),
                 dict(self.corpus, version=True)]
        for corpus in cases:
            with self.subTest(corpus=corpus), self.assertRaises((ValueError, RunError)):
                draft.validate(corpus, self.labels)


if __name__ == '__main__':
    unittest.main()
