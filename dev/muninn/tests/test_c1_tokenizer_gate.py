"""Contract tests for the tokenizer receipt gate; no tokenizer runtime or cache.

These tests use small synthetic receipts. They prove the gate logic, not the
pinned runtime: only a receipt produced by the real probe binds runtime facts.
"""
import copy
import hashlib
import json
from pathlib import Path
import unittest

import c1_tokenizer_gate as gate

ROOT = Path(__file__).resolve().parents[3]


def corpus_bytes():
    return (ROOT / 'dev/muninn/fixtures/minilm-c1-v1-draft.json').read_bytes()


def labels_bytes():
    return (ROOT / 'dev/muninn/fixtures/minilm-c1-v1-draft-labels.json').read_bytes()


def row(ids):
    return {'input_ids': list(ids), 'attention_mask': [1] * len(ids),
            'token_type_ids': [0] * len(ids)}


def synthetic_receipt(corpus, labels):
    """Receipt-shaped fixture derived from corpus text identity only."""
    targets = labels['boundary_targets']
    batches = []
    for batch in corpus['batches']:
        rows = []
        for index, text in enumerate(batch['texts']):
            if batch['id'] == 'boundary_candidates':
                length = targets[index]
            else:
                length = max(len(text) % 23 + 2, 3)
            marker = 1000 + (int(hashlib.sha256(text.encode()).hexdigest()[:8], 16) % 97)
            rows.append([101] + [marker] * (length - 2) + [102])
        truncated = []
        for ids in rows:
            truncated.append(list(ids[:255]) + [102] if len(ids) > 256 else list(ids))
        width = max(len(ids) for ids in truncated)
        batches.append({
            'id': batch['id'],
            'full': {'input_ids': rows,
                     'attention_mask': [[1] * len(ids) for ids in rows],
                     'token_type_ids': [[0] * len(ids) for ids in rows]},
            'truncated': {'input_ids': truncated,
                          'attention_mask': [[1] * len(ids) for ids in truncated],
                          'token_type_ids': [[0] * len(ids) for ids in truncated]},
            'padded': {'input_ids': [ids + [0] * (width - len(ids)) for ids in truncated],
                       'attention_mask': [[1] * len(ids) + [0] * (width - len(ids))
                                          for ids in truncated],
                       'token_type_ids': [[0] * width for ids in truncated]},
            'full_lengths_including_specials': [len(ids) for ids in rows],
            'truncated_lengths_including_specials': [len(ids) for ids in truncated],
            'was_truncated': [len(a) > len(b) for a, b in zip(rows, truncated)],
        })
    return {
        'schema_version': 1,
        'scope': 'tokenizer_only_not_encoder_evidence',
        'acceptance_ready': False,
        'corpus_id': corpus['corpus_id'],
        'corpus_sha256': gate.sha256_bytes(corpus_bytes()),
        'labels_sha256': gate.sha256_bytes(labels_bytes()),
        'model_id': 'sentence-transformers/all-MiniLM-L6-v2',
        'revision': '1110a243fdf4706b3f48f1d95db1a4f5529b4d41',
        'max_length': 256,
        'padding_side': 'right',
        'truncation_side': 'right',
        'truncation_strategy': 'longest_first',
        'boundary_batch': 'boundary_candidates',
        'boundary_targets_from_labels': [127, 128, 129, 255, 256, 257],
        'tokenizer_packages': {},
        'artifact_files': {},
        'batches': batches,
    }


class TokenizerGateTests(unittest.TestCase):
    def setUp(self):
        self.corpus = json.loads(corpus_bytes())
        self.labels = json.loads(labels_bytes())
        self.receipt = synthetic_receipt(self.corpus, self.labels)

    def test_synthetic_receipt_shape_is_accepted_but_not_runtime_proof(self):
        report = gate.validate(self.receipt, self.corpus, self.labels,
                               corpus_bytes(), labels_bytes())
        self.assertEqual(report['state'], 'tokenizer_gate_pass_not_encoder_acceptance')
        self.assertFalse(report['acceptance_ready'])
        self.assertEqual(report['acceptance_ready'], False)

    def test_receipt_must_bind_the_exact_corpus_and_labels_bytes(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['corpus_sha256'] = '0' * 64
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_receipt_must_not_claim_encoder_acceptance(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['acceptance_ready'] = True
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_truncation_must_keep_the_final_separator(self):
        receipt = copy.deepcopy(self.receipt)
        batch = receipt['batches'][0]
        ids = batch['truncated']['input_ids'][0]
        ids[-1] = 999
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_padding_mask_must_mark_exactly_the_padded_positions(self):
        receipt = copy.deepcopy(self.receipt)
        batch = next(b for b in receipt['batches'] if b['id'] == 'semantic_es')
        lengths = [len(ids) for ids in batch['truncated']['input_ids']]
        shortest = lengths.index(min(lengths))
        width = len(batch['padded']['input_ids'][0])
        if shortest == lengths.index(max(lengths)):
            self.skipTest('batch rows share one length')
        batch['padded']['attention_mask'][shortest][-1] = 1
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_declared_lengths_must_match_the_observed_rows(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['batches'][0]['full_lengths_including_specials'][0] += 1
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_every_corpus_batch_must_be_present_in_the_receipt(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['batches'] = receipt['batches'][:-1]
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_same_text_must_tokenize_identically_across_batches(self):
        receipt = copy.deepcopy(self.receipt)
        mixed = next(b for b in receipt['batches'] if b['id'] == 'mixed')
        single = next(b for b in receipt['batches'] if b['id'] == 'single')
        mixed['full']['input_ids'][0] = list(mixed['full']['input_ids'][0])
        mixed['full']['input_ids'][0][1] = 4242
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())
        self.assertTrue(single['full']['input_ids'][0])

    def test_boundary_targets_must_match_the_labels_declaration(self):
        receipt = copy.deepcopy(self.receipt)
        receipt['boundary_targets_from_labels'] = [127, 128, 129, 255, 256, 256]
        with self.assertRaises(gate.GateError):
            gate.validate(receipt, self.corpus, self.labels, corpus_bytes(), labels_bytes())

    def test_report_lists_observed_boundary_lengths(self):
        report = gate.validate(self.receipt, self.corpus, self.labels,
                               corpus_bytes(), labels_bytes())
        boundary = next(b for b in self.receipt['batches']
                        if b['id'] == self.receipt['boundary_batch'])
        self.assertEqual(report['boundary_observed_lengths'],
                         boundary['full_lengths_including_specials'])


if __name__ == '__main__':
    unittest.main()
