"""Product TCK rows do not rewrite historical reference-oracle outcomes."""
import sys
from pathlib import Path
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))
import api_test_traceability


class ReferenceBaselineProfileTests(unittest.TestCase):
    def test_only_historical_reference_profiles_are_compared(self):
        candidate = "v1/test_short_term.py::TestDeleteMessage::test_delete_message_returns_true"
        rows = [
            {"profile": "adapted", "test_id": candidate, "outcome": "passed", "gap_id": ""},
            {"profile": "native-delete", "test_id": candidate, "outcome": "failed", "gap_id": "REF-001"},
            {"profile": "product-b6c7-chains", "test_id": candidate,
             "outcome": "passed", "gap_id": ""},
            {"profile": "product-b6c7-bronze", "test_id": candidate,
             "outcome": "passed", "gap_id": ""},
        ]
        self.assertEqual(
            api_test_traceability.reference_baseline_outcomes(rows, candidate),
            {("adapted", "passed", ""), ("native-delete", "failed", "REF-001")},
        )


if __name__ == "__main__":
    unittest.main()
