"""Selección explícita y determinista de perfiles del runner de producto."""
import sys
from pathlib import Path
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))
from product_tck_selector import select_profiles

PROFILES = ('messages', 'chains', 'search', 'schema', 'bronze', 'silver-long-term',
            'silver-reasoning', 'gold-cross-memory')


class ProductTckProfileSelectionTests(unittest.TestCase):
    def test_defaults_to_all_profiles(self):
        self.assertEqual(select_profiles(None, PROFILES), PROFILES)

    def test_selects_one_profile(self):
        self.assertEqual(select_profiles(['silver-long-term'], PROFILES), ('silver-long-term',))

    def test_selects_only_requested_profiles_in_canonical_order(self):
        self.assertEqual(
            select_profiles(['gold-cross-memory', 'silver-long-term'], PROFILES),
            ('silver-long-term', 'gold-cross-memory'),
        )

    def test_rejects_duplicate_profiles(self):
        with self.assertRaisesRegex(ValueError, 'duplic'):
            select_profiles(['silver-long-term', 'silver-long-term'], PROFILES)

    def test_rejects_unknown_profiles(self):
        with self.assertRaisesRegex(ValueError, 'desconocido'):
            select_profiles(['silver-hito-inexistente'], PROFILES)


if __name__ == '__main__':
    unittest.main()
