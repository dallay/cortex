"""El recibo Bronze exige cardinalidad e IDs exactos, no solo exit code."""
import unittest


class BronzeReceiptTests(unittest.TestCase):
    def test_rejects_partial_bronze_collection(self):
        from bronze_receipt import validate_bronze
        with self.assertRaises(ValueError):
            validate_bronze([{'nodeid': 'v1/one', 'markers': ['bronze']}],
                            {'tests': [{'nodeid': 'v1/one', 'outcome': 'passed'}]}, 0)

    def test_accepts_only_complete_exact_passing_ids(self):
        from bronze_receipt import validate_bronze
        items = [{'nodeid': str(i), 'markers': ['bronze'] if i < 93 else []} for i in range(189)]
        report = {'tests': [{'nodeid': str(i), 'outcome': 'passed'} for i in range(93)]}
        validate_bronze(items, report, 0)
        report['tests'][0]['outcome'] = 'skipped'
        with self.assertRaises(ValueError):
            validate_bronze(items, report, 0)
