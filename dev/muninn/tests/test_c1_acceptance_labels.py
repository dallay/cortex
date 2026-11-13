"""Conversión offline: hipótesis del agente, nunca resultados ni aceptación."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import c1_corpus_draft as draft
import compare_minilm_c1 as compare
import c1_acceptance_labels as labels


class AcceptanceLabelsTests(unittest.TestCase):
    def setUp(self):
        self.corpus = json.loads((draft.ROOT / draft.CORPUS).read_text())
        self.draft = json.loads((draft.ROOT / draft.LABELS).read_text())

    def convert(self):
        return labels.convert(self.corpus, self.draft)

    def test_exact_comparator_contract_and_agent_provenance(self):
        result = self.convert()
        compare.validate_labels(result, self.corpus)
        self.assertEqual(result['status'], 'agent_hypotheses_not_results')
        self.assertIn('no humanas', result['label_origin_es'])
        self.assertEqual(len(result['triplets']), 6)
        for triplet in result['triplets']:
            self.assertIn('hipótesis', triplet['rationale_es'])
            self.assertIn('no resultados', triplet['rationale_es'])

    def test_refs_preserve_all_draft_triplets(self):
        result = self.convert()
        for source, target in zip(self.draft['triplets'], result['triplets']):
            for role in ('query', 'positive', 'negative'):
                self.assertEqual(target[role], f"{source['batch']}/{source[role]}")

    def test_coverage_has_concrete_valid_refs_for_every_declared_category(self):
        result = self.convert()
        self.assertTrue(draft.COVERAGE <= result['coverage'].keys())
        batches = {b['id']: b['texts'] for b in self.corpus['batches']}
        for refs in result['coverage'].values():
            self.assertTrue(refs)
            for ref in refs:
                batch, index = ref.split('/')
                self.assertIsInstance(batches[batch][int(index)], str)
        self.assertEqual(result['coverage']['paraphrases_no_shared_words'], [
            'lexical_es/0', 'lexical_es/1'])

    def test_equal_groups_are_all_and_only_exact_duplicates(self):
        expected = {}
        for batch in self.corpus['batches']:
            for index, text in enumerate(batch['texts']):
                expected.setdefault(text, []).append(f"{batch['id']}/{index}")
        self.assertEqual(self.convert()['equal_text_groups'], [v for v in expected.values() if len(v) > 1])
        groups = self.convert()['equal_text_groups']
        self.assertFalse(any('unicode/1' in g and
                             'unicode/2' in g for g in groups))

    def test_equal_groups_follow_changed_corpus_not_invented_nfc_groups(self):
        self.corpus['batches'][6]['texts'][0] = self.corpus['batches'][6]['texts'][1]
        result = self.convert()
        self.assertTrue(any('unicode/0' in group and
                            'unicode/1' in group
                            for group in result['equal_text_groups']))

    def test_length_hypotheses_are_empty_and_receipt_is_separate(self):
        result = self.convert()
        self.assertEqual(result['length_hypotheses'], [])
        self.assertIn('receipt', result['length_note_es'])
        self.assertIn('hello_repetitions', result['length_note_es'])

    def test_common_unicode_word_rejects_lexical_support(self):
        self.corpus['batches'][2]['texts'][0] += ' ÁRBOL'
        self.corpus['batches'][2]['texts'][1] += ' árbol'
        with self.assertRaisesRegex(ValueError, 'palabras comunes'):
            self.convert()

    def test_inconsistent_inputs_rejected(self):
        for mutate in (lambda: self.draft.update(corpus_id='other'),
                       lambda: self.draft['triplets'][0].update(query=999),
                       lambda: self.corpus['batches'].pop()):
            with self.subTest(mutate=mutate):
                self.setUp()
                mutate()
                with self.assertRaises(ValueError):
                    self.convert()

    def test_conversion_does_not_mutate_sources(self):
        original = copy.deepcopy((self.corpus, self.draft))
        self.convert()
        self.assertEqual((self.corpus, self.draft), original)

    def test_cli_stdout_and_exclusive_output(self):
        command = [sys.executable, str(Path(labels.__file__)), '--corpus',
                   str(draft.ROOT / draft.CORPUS), '--draft-labels', str(draft.ROOT / draft.LABELS)]
        proc = subprocess.run(command, capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(json.loads(proc.stdout), self.convert())
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'labels.json'
            proc = subprocess.run(command + ['--output', str(output)], capture_output=True, text=True)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            raw = output.read_bytes()
            proc = subprocess.run(command + ['--output', str(output)], capture_output=True, text=True)
            self.assertNotEqual(proc.returncode, 0)
            self.assertEqual(output.read_bytes(), raw)


if __name__ == '__main__':
    unittest.main()
