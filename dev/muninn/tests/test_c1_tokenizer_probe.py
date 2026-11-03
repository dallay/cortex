"""Algorithm tests only: no tokenizer runtime, cache, network, or weights."""
import hashlib
import json
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import c1_tokenizer_probe as probe
from export_minilm_c1 import RunError

ROOT = Path(__file__).resolve().parents[3]


class FakeTokenizer:
    truncation_side = 'left'
    padding_side = 'left'

    def __init__(self):
        self.calls = []

    def __call__(self, texts, **kwargs):
        self.calls.append(kwargs)
        rows = []
        for text in texts:
            ids = [101] + [int(word) for word in text.split()] + [102]
            if kwargs['truncation']:
                ids = ids[:255] + [102] if len(ids) > 256 else ids
            rows.append(ids)
        width = max(map(len, rows))
        return {
            'input_ids': [r + [0] * (width - len(r)) if kwargs['padding'] else r for r in rows],
            'attention_mask': [[1] * len(r) + ([0] * (width - len(r)) if kwargs['padding'] else []) for r in rows],
            'token_type_ids': [[0] * (width if kwargs['padding'] else len(r)) for r in rows],
        }


class ProbeTests(unittest.TestCase):
    def test_full_and_truncated_positions_and_prepadding_lengths(self):
        tokenizer = FakeTokenizer()
        corpus = {'corpus_id': 'draft', 'batches': [
            {'id': 'boundary_candidates', 'texts': ['', ' '.join(['7'] * 255), '8 9']}]}
        labels = {'boundary_batch': 'boundary_candidates', 'boundary_targets': [2, 257, 4]}
        result = probe.probe_tokens(tokenizer, corpus, labels)
        batch = result['batches'][0]
        self.assertEqual(batch['full_lengths_including_specials'], [2, 257, 4])
        self.assertEqual(batch['truncated_lengths_including_specials'], [2, 256, 4])
        self.assertEqual(batch['was_truncated'], [False, True, False])
        self.assertEqual(batch['full']['input_ids'][2], [101, 8, 9, 102])
        self.assertEqual(batch['truncated']['input_ids'][1][-1], 102)
        self.assertEqual(batch['padded']['attention_mask'][0], [1, 1] + [0] * 254)
        self.assertEqual(len(batch['padded']['token_type_ids'][0]), 256)
        self.assertEqual(result['boundary_targets_from_labels'], [2, 257, 4])
        self.assertFalse(result['acceptance_ready'])
        self.assertEqual(tokenizer.truncation_side, 'right')
        self.assertEqual(tokenizer.padding_side, 'right')
        self.assertEqual([c['padding'] for c in tokenizer.calls], [False, False, True])
        self.assertEqual([c['truncation'] for c in tokenizer.calls], [False, 'longest_first', 'longest_first'])
        self.assertTrue(all(c['add_special_tokens'] for c in tokenizer.calls))
        self.assertEqual(tokenizer.calls[1]['max_length'], 256)

    def test_generate_uses_local_auto_tokenizer_and_checked_files_twice(self):
        from minilm_cpu_oracle import load_pinned_lock
        lock = load_pinned_lock((ROOT / 'dev/muninn/fixtures/artifact-lock.json').read_bytes())
        versions = {}
        for line in (ROOT / 'dev/muninn/requirements-minilm-tokenizer.txt').read_text().splitlines():
            if line and not line.startswith('#'):
                name, version = line.split(' --hash=sha256:', 1)[0].split('==', 1)
                versions[name] = version
        calls = []

        def load(path, **kwargs):
            calls.append((path, kwargs))
            self.assertEqual(os.environ['USE_TORCH'], '0')
            self.assertEqual(os.environ['HF_HUB_OFFLINE'], '1')
            return FakeTokenizer()

        # The complete draft includes non-numeric text; use a numeric-only fake
        # while retaining the actual reader and sidecar validation.
        class DraftFake(FakeTokenizer):
            def __call__(self, texts, **kwargs):
                return super().__call__(['' for _ in texts], **kwargs)

        def load_draft(path, **kwargs):
            load(path, **kwargs)
            return DraftFake()

        corpus = ROOT / 'dev/muninn/fixtures/minilm-c1-v1-draft.json'
        labels = ROOT / 'dev/muninn/fixtures/minilm-c1-v1-draft-labels.json'
        with patch.dict(os.environ), patch.dict(sys.modules, {
                'transformers': SimpleNamespace(AutoTokenizer=SimpleNamespace(from_pretrained=load_draft))}), \
                patch.object(probe.importlib.metadata, 'version', side_effect=lambda p: versions[p.replace('-', '_')] if p.replace('-', '_') in versions else versions[p]), \
                patch('minilm_tokenizer_reference._checked_files', return_value={'checked': 'fake'}) as checked:
            result = probe.generate(Path('/not-opened'), corpus, labels)
        self.assertEqual(checked.call_count, 2)
        self.assertEqual(calls, [('/not-opened', {'local_files_only': True, 'trust_remote_code': False})])
        self.assertEqual(result['corpus_sha256'], hashlib.sha256(corpus.read_bytes()).hexdigest())
        self.assertEqual(result['boundary_batch'], 'boundary_candidates')

    def test_cli_errors_are_bounded_and_stdout_stays_empty(self):
        import io
        out, err = io.StringIO(), io.StringIO()
        with patch.object(probe, 'generate', side_effect=RunError('private ' * 1000)), \
                patch('sys.stdout', out), patch('sys.stderr', err):
            status = probe.main(['--bundle', '/fake', '--corpus', '/fake', '--labels', '/fake'])
        self.assertEqual(status, 1)
        self.assertEqual(out.getvalue(), '')
        self.assertEqual(err.getvalue(), 'C1 tokenizer probe failed: RunError\n')


if __name__ == '__main__':
    unittest.main()
