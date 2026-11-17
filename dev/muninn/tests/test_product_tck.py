"""El runner TCK rechaza informes incompletos o inconsistentes."""
import sys
from pathlib import Path
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))
import product_tck_report


def report(nodeids, *, outcomes=None, summary=None, exitcode=0):
    outcomes = outcomes or ['passed'] * len(nodeids)
    tests = []
    for nodeid, outcome in zip(nodeids, outcomes, strict=True):
        tests.append({
            'nodeid': nodeid,
            'outcome': outcome,
            'setup': {'outcome': 'passed'},
            'call': {'outcome': 'passed' if outcome == 'passed' else outcome},
            'teardown': {'outcome': 'passed'},
        })
    return {
        'exitcode': exitcode,
        'tests': tests,
        'summary': summary or {'passed': len(nodeids), 'total': len(nodeids)},
    }


class ProductTckValidationTests(unittest.TestCase):
    def assert_rejected(self, expected, result, code=0):
        with self.assertRaises(ValueError):
            product_tck_report.validate_profile_result(expected, code, result)

    def test_profile_receipt_partitions_collection_and_preserves_selected_outcomes(self):
        collected = [
            {'nodeid': 'case-a', 'markers': ['bronze']},
            {'nodeid': 'case-b', 'markers': []},
            {'nodeid': 'case-c', 'markers': ['silver']},
        ]
        pytest_tests = report(['case-a', 'case-c'], outcomes=['passed', 'failed'])['tests']

        receipt = product_tck_report.build_profile_receipt(
            collected, {'case-a', 'case-c'}, pytest_tests,
        )

        self.assertEqual(receipt['collected_ids'], ['case-a', 'case-b', 'case-c'])
        self.assertEqual(receipt['selected_ids'], ['case-a', 'case-c'])
        self.assertEqual(receipt['excluded'], [
            {'nodeid': 'case-b', 'reason': 'not_selected_by_profile'},
        ])
        self.assertEqual(receipt['outcomes'], [
            {'nodeid': 'case-a', 'outcome': 'passed'},
            {'nodeid': 'case-c', 'outcome': 'failed'},
        ])
        selected_ids = set(receipt['selected_ids'])
        excluded_ids = {item['nodeid'] for item in receipt['excluded']}
        collected_ids = set(receipt['collected_ids'])
        self.assertFalse(selected_ids & excluded_ids)
        self.assertEqual(selected_ids | excluded_ids, collected_ids)
        self.assertEqual(selected_ids, {item['nodeid'] for item in receipt['outcomes']})

    def test_profile_receipt_rejects_duplicate_collection_and_uncollected_selection(self):
        item = {'nodeid': 'case-a', 'markers': []}
        with self.assertRaisesRegex(ValueError, 'duplicate_collected_nodeid'):
            product_tck_report.build_profile_receipt([item, item], {'case-a'}, [])
        with self.assertRaisesRegex(ValueError, 'selected_nodeid_not_collected'):
            product_tck_report.build_profile_receipt([item], {'case-b'}, [])

    def test_accepts_exact_complete_success_report(self):
        expected = {'case-a', 'case-b'}
        product_tck_report.validate_profile_result(expected, 0, report(sorted(expected)))

    def test_rejects_empty_expected_profile(self):
        self.assert_rejected(set(), report([]))

    def test_rejects_partial_report(self):
        self.assert_rejected({'case-a', 'case-b'}, report(['case-a']))

    def test_rejects_nonzero_process_or_report_exit(self):
        expected = {'case-a'}
        self.assert_rejected(expected, report(['case-a']), code=1)
        self.assert_rejected(expected, report(['case-a'], exitcode=1))

    def test_rejects_duplicate_and_unexpected_node_ids(self):
        expected = {'case-a', 'case-b'}
        self.assert_rejected(expected, report(['case-a', 'case-a']))
        self.assert_rejected(expected, report(['case-a', 'case-c']))

    def test_rejects_nonpassing_test_or_phase(self):
        self.assert_rejected({'case-a'}, report(['case-a'], outcomes=['failed']))
        broken = report(['case-a'])
        broken['tests'][0]['teardown']['outcome'] = 'failed'
        self.assert_rejected({'case-a'}, broken)

    def test_rejects_summary_counts_that_disagree_with_records(self):
        self.assert_rejected({'case-a'}, report(['case-a'], summary={'passed': 1, 'total': 2}))
        self.assert_rejected({'case-a'}, report(['case-a'], summary={
            'passed': 1, 'total': 1, 'failed': 1,
        }))


if __name__ == '__main__':
    unittest.main()
