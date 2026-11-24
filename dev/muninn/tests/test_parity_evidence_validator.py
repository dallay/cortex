"""TDD tests for Muninn parity evidence & closure validator (SDD-1B: F-19, F-20)."""
from pathlib import Path
import subprocess
import sys
import unittest

import parity_evidence_validator as validator

ROOT = Path(__file__).resolve().parents[3]
VALIDATOR = ROOT / "dev/muninn/parity_evidence_validator.py"
CLOSURE_DOC = ROOT / "docs/muninn/specs/bolt-parity-final-closure.md"


def sample_receipt(**overrides):
    receipt = {
        "capability_id": "BOLT-08-05",
        "candidate_commit": "4d3835bedf83d65da6db71c0752eda49e380929d",
        "reference_commit": "9d45af5529ecb5f2fe352013468a11087f3163d4",
        "execution_status": "PASSED",
        "implementation_status": "IMPLEMENTED",
        "critical": True,
        "reference_result": "passed",
        "candidate_result": "passed",
        "deviations": [],
        "evidence": ["crates/muninn/neo4j/tests/consolidation.rs"],
        "accepted": True,
    }
    receipt.update(overrides)
    return receipt


class ParityEvidenceValidatorTests(unittest.TestCase):
    def test_loads_all_92_official_bolt_capability_ids(self):
        ids = validator.load_capability_ids(ROOT)
        self.assertEqual(len(ids), 92)
        self.assertIn("BOLT-01-01", ids)
        self.assertIn("BOLT-08-04", ids)
        self.assertIn("BOLT-08-06", ids)
        self.assertIn("BOLT-08-15", ids)

    def test_closure_document_has_no_missing_test_file_references(self):
        missing = validator.find_missing_repo_references(CLOSURE_DOC, ROOT)
        self.assertEqual(missing, [], f"Closure doc cites non-existent files: {missing}")

    def test_detects_missing_test_file_reference_in_markdown(self):
        markdown = "25. `BOLT-08-04`: Verificado en `crates/muninn/bridge/tests/buffered.rs`.\n"
        missing = validator.find_missing_references_in_text(markdown, ROOT)
        self.assertEqual(missing, ["crates/muninn/bridge/tests/buffered.rs"])

    def test_rejects_skipped_ignored_or_not_run_receipt_marked_accepted(self):
        cap_ids = validator.load_capability_ids(ROOT)
        expected_commit = "4d3835bedf83d65da6db71c0752eda49e380929d"
        for status in ("NOT_RUN", "SKIPPED", "IGNORED", "FAILED"):
            with self.assertRaisesRegex(validator.EvidenceError, "execution_status"):
                validator.validate_receipt(
                    sample_receipt(execution_status=status, accepted=True),
                    cap_ids,
                    expected_commit,
                    ROOT,
                )

    def test_rejects_unknown_capability_id(self):
        cap_ids = validator.load_capability_ids(ROOT)
        with self.assertRaisesRegex(validator.EvidenceError, "capability_id"):
            validator.validate_receipt(
                sample_receipt(capability_id="BOLT-99-99"),
                cap_ids,
                "4d3835bedf83d65da6db71c0752eda49e380929d",
                ROOT,
            )

    def test_rejects_stale_candidate_commit(self):
        cap_ids = validator.load_capability_ids(ROOT)
        with self.assertRaisesRegex(validator.EvidenceError, "candidate_commit"):
            validator.validate_receipt(
                sample_receipt(candidate_commit="6be52cb0cf4c0000000000000000000000000000"),
                cap_ids,
                "4d3835bedf83d65da6db71c0752eda49e380929d",
                ROOT,
            )

    def test_rejects_accepted_receipt_with_empty_or_missing_evidence(self):
        cap_ids = validator.load_capability_ids(ROOT)
        commit = "4d3835bedf83d65da6db71c0752eda49e380929d"
        with self.assertRaisesRegex(validator.EvidenceError, "evidence"):
            validator.validate_receipt(sample_receipt(evidence=[]), cap_ids, commit, ROOT)
        with self.assertRaisesRegex(validator.EvidenceError, "evidence"):
            validator.validate_receipt(
                sample_receipt(evidence=["crates/muninn/bridge/tests/buffered.rs"]),
                cap_ids,
                commit,
                ROOT,
            )

    def test_rejects_duplicate_capability_receipts(self):
        cap_ids = validator.load_capability_ids(ROOT)
        commit = "4d3835bedf83d65da6db71c0752eda49e380929d"
        with self.assertRaisesRegex(validator.EvidenceError, "duplicate"):
            validator.validate_receipts(
                [sample_receipt(), sample_receipt()],
                cap_ids,
                commit,
                ROOT,
            )

    def test_multi_gate_acceptance_requires_all_four_gates_without_averaging(self):
        passed = validator.evaluate_release_gates(
            verified_api_ops=132,
            verified_capabilities=88,
            verified_critical=27,
            passed_tck=178,
        )
        self.assertTrue(passed["accepted"])

        # Fails if API ops below 132 even when other gates pass
        self.assertFalse(
            validator.evaluate_release_gates(131, 92, 27, 178)["accepted"]
        )
        # Fails if independent capabilities below 88 even when API ops >= 132
        self.assertFalse(
            validator.evaluate_release_gates(138, 87, 27, 178)["accepted"]
        )
        # Fails if any critical criterion is missing
        self.assertFalse(
            validator.evaluate_release_gates(138, 92, 26, 178)["accepted"]
        )
        # Fails if selected TCK is incomplete
        self.assertFalse(
            validator.evaluate_release_gates(138, 92, 27, 177)["accepted"]
        )

    def test_cli_check_passes_for_reconciled_repository(self):
        result = subprocess.run(
            [sys.executable, str(VALIDATOR), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
