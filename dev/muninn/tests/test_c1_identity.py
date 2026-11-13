"""Pinned declaration tests. No inference or cache access."""
import copy
import json
from pathlib import Path
import unittest
import c1_identity as identity

class IdentityTests(unittest.TestCase):
    def test_cross_language_golden(self):
        declaration = identity.declare('test-unapproved-v1')
        self.assertEqual(declaration['space_id'], '7b39cda61fd78b7fcf492989102d58151950b3cdb1451b89c1a107d3a109ddea')
        self.assertEqual(declaration['artifact_bundle_id'], 'ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1')
        identity.validate(declaration, 'test-unapproved-v1')

    def test_declaration_matches_the_shared_rust_golden(self):
        golden = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/minilm-c1-identity-golden.json').read_text())
        self.assertEqual(identity.declare('test-unapproved-v1'), golden)

    def test_contract_grammar_matches_shared_cases(self):
        cases = json.loads((Path(__file__).resolve().parents[1] / 'fixtures/minilm-c1-contract-cases.json').read_text())
        for contract in cases['valid']:
            with self.subTest(contract=contract):
                declaration = identity.declare(contract)
                self.assertEqual(declaration['compatibility']['numeric_contract_version'], contract)
        for contract in cases['invalid']:
            with self.subTest(contract=contract), self.assertRaises(ValueError):
                identity.declare(contract)

    def test_invalid_contract(self):
        for contract in [None, '', 'X', 'bad contract', 'a'*65, 1]:
            with self.subTest(contract=contract), self.assertRaises(ValueError):
                identity.declare(contract)

    def test_full_identity_not_just_id(self):
        for change in ('config', 'digest', 'id', 'contract', 'extra', 'bundle'):
            value = copy.deepcopy(identity.declare('test-unapproved-v1'))
            if change == 'config': value['compatibility']['normalization'] = 'none'
            if change == 'digest': value['compatibility']['model_files'][0]['sha256'] = '0'*64
            if change == 'id': value['space_id'] = '0'*64
            if change == 'contract': value['compatibility']['numeric_contract_version'] = 'test-unapproved-v2'
            if change == 'extra': value['extra'] = True
            if change == 'bundle': value['artifact_bundle_id'] = '0'*64
            with self.subTest(change=change), self.assertRaises(ValueError):
                identity.validate(value, 'test-unapproved-v1')
