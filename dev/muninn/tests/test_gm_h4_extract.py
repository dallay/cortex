"""GM-03 spaCy stage projection rules; no model, image or network is executed."""
import json
from pathlib import Path
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

FIXTURE = json.loads((BASE / 'fixtures/gm_h4_v1.json').read_text())


def reference_payload(**values):
    """The pinned ExtractedEntity surface, projected by model_dump(mode='json')."""
    payload = {'name': 'Paris', 'type': 'LOCATION', 'subtype': 'GEOPOLITICAL',
               'start_pos': 30, 'end_pos': 35, 'confidence': 0.85, 'extractor': 'spacy',
               'context': 'Alice met Acme Corporation in Paris on 4 May 2024.',
               'attributes': {'spacy_label': 'GPE', 'spacy_label_description': 'GPE'},
               'id': None}
    payload.update(values)
    return payload


def product_payload(**values):
    """The Muninn sidecar extraction item surface."""
    payload = {'name': 'Paris', 'type': 'LOCATION', 'subtype': 'GEOPOLITICAL',
               'start_pos': 30, 'end_pos': 35, 'confidence': 0.85, 'extractor': 'spacy'}
    payload.update(values)
    return payload


class ProjectionTests(unittest.TestCase):
    def setUp(self):
        from gm_h4_extract import capture_gm03
        from gm_h4_contract import ContractError
        self.capture = capture_gm03
        self.error = ContractError

    def texts(self):
        return [dict(item) for item in FIXTURE['gm03']['texts']]

    def test_reference_payload_projects_only_the_compared_fields(self):
        captured = self.capture(self.texts(), lambda text: [reference_payload()],
                                'reference')
        self.assertEqual(set(captured['entities'][0]),
                         {'text', 'name', 'type', 'subtype', 'start_pos', 'end_pos',
                          'confidence', 'extractor'})
        self.assertEqual(captured['entities'][0]['text'], 'auto')
        self.assertEqual(captured['entities'][0]['name'], 'Paris')
        self.assertEqual(captured['entities'][0]['start_pos'], 30)

    def test_product_payload_projects_only_the_compared_fields(self):
        captured = self.capture(self.texts(), lambda text: [product_payload()], 'product')
        self.assertEqual(set(captured['entities'][0]),
                         {'text', 'name', 'type', 'subtype', 'start_pos', 'end_pos',
                          'confidence', 'extractor'})

    def test_unknown_fields_fail_closed(self):
        with self.assertRaisesRegex(self.error, 'unknown'):
            self.capture(self.texts(), lambda text: [reference_payload(label='GPE')],
                         'reference')
        with self.assertRaisesRegex(self.error, 'unknown'):
            self.capture(self.texts(), lambda text: [product_payload(context='x')], 'product')

    def test_missing_compared_fields_fail_closed(self):
        for payload, side in ((reference_payload(), 'reference'),
                              (product_payload(), 'product')):
            for name in ('name', 'type', 'start_pos', 'end_pos', 'confidence', 'extractor'):
                broken = dict(payload)
                del broken[name]
                with self.assertRaisesRegex(self.error, 'missing', msg=name):
                    self.capture(self.texts(), lambda text, value=broken: [value], side)
        for payload, side in ((reference_payload(), 'reference'),
                              (product_payload(), 'product')):
            for name in ('subtype',):
                broken = dict(payload)
                del broken[name]
                with self.assertRaisesRegex(self.error, 'missing', msg=name):
                    self.capture(self.texts(), lambda text, value=broken: [value], side)

    def test_pinned_offsets_and_names_are_not_normalized(self):
        padded = {'name': 'Paris', 'type': 'LOCATION', 'subtype': 'GEOPOLITICAL',
                  'start_pos': 5, 'end_pos': 12, 'confidence': 0.85, 'extractor': 'spacy'}
        captured = self.capture(self.texts(), lambda text: [dict(padded)], 'product')
        self.assertEqual((captured['entities'][0]['start_pos'],
                          captured['entities'][0]['end_pos']), (5, 12))
        self.assertEqual(captured['entities'][0]['name'], 'Paris')

    def test_document_order_is_preserved_across_texts(self):
        def extract(text):
            if text.strip():
                return [product_payload(), product_payload(name='Alice', type='PERSON',
                                                           subtype=None, start_pos=0,
                                                           end_pos=5)]
            return []
        captured = self.capture(self.texts(), extract, 'product')
        self.assertEqual([item['text'] for item in captured['entities']],
                         ['auto', 'auto', 'unicode', 'unicode', 'mixed', 'mixed'])
        self.assertEqual([item['name'] for item in captured['entities']],
                         ['Paris', 'Alice'] * 3)

    def test_empty_stage_output_is_an_empty_list_not_a_default(self):
        captured = self.capture(self.texts(), lambda text: [], 'reference')
        self.assertEqual(captured['entities'], [])
        self.assertEqual([item['key'] for item in captured['texts']],
                         [item['key'] for item in FIXTURE['gm03']['texts']])

    def test_stage_output_must_be_a_list(self):
        with self.assertRaisesRegex(self.error, 'list required'):
            self.capture(self.texts(), lambda text: None, 'reference')
        with self.assertRaisesRegex(self.error, 'list required'):
            self.capture(self.texts(), lambda text: {'entities': []}, 'product')

    def test_only_declared_sides_are_accepted(self):
        for value in ('oracle', '', 'product '):
            with self.assertRaisesRegex(ValueError, 'unknown capture side'):
                self.capture(self.texts(), lambda text: [], value)

    def test_module_needs_no_model_runtime_at_import(self):
        source = (BASE / 'gm_h4_extract.py').read_text()
        self.assertNotIn('import spacy', source)
        self.assertNotIn('spacy.load', source)
        self.assertNotIn('urllib', source)
        self.assertNotIn('http', source)

    def test_captured_texts_are_the_frozen_inputs_verbatim(self):
        captured = self.capture(self.texts(), lambda text: [], 'reference')
        self.assertEqual(captured['texts'], FIXTURE['gm03']['texts'])


if __name__ == '__main__':
    unittest.main()
