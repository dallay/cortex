"""Synthetic tests for the C1 acceptance driver; no inference, no Neo4j."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import c1_acceptance


POLICY = {"version": "minilm-cpu-f32-v1", "max_abs_error": 1e-6,
          "min_embedding_cosine": 0.99999999999, "max_score_error": 2e-6,
          "tie_epsilon": 1e-6, "max_norm_error": 1e-6}


class C1AcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "corpus.json").write_bytes(b'{"corpus_id":"c1-test","batches":[]}')
        (self.root / "labels.json").write_bytes(b'{"schema_version":1}')
        (self.root / "policy.json").write_bytes(json.dumps(POLICY).encode("utf-8"))
        self.entries = []
        for relative in ("corpus.json", "labels.json", "policy.json"):
            data = (self.root / relative).read_bytes()
            self.entries.append({"path": relative, "size": len(data),
                                 "sha256": hashlib.sha256(data).hexdigest()})
        self.manifest = {"schema_version": 1, "state": "frozen_pre_execution",
                         "numeric_contract_version": "minilm-cpu-f32-v1",
                         "inputs": self.entries}
        self.manifest_path = self.root / "manifest.json"
        self.manifest_path.write_text(json.dumps(self.manifest, sort_keys=True), encoding="utf-8")
        self.manifest_sha256 = hashlib.sha256(self.manifest_path.read_bytes()).hexdigest()
        self.attempt = self.root / "attempt"
        (self.attempt / "logs").mkdir(parents=True)
        rust_log = self.attempt / "logs" / "rust-export.stdout.log"
        python_log = self.attempt / "logs" / "python-export.stdout.log"
        rust_log.write_text(json.dumps({"identity": {}}), encoding="utf-8")
        python_log.write_text(json.dumps({"identity": {}}), encoding="utf-8")
        self.receipt = {
            "status": "complete",
            "manifest": {"sha256": self.manifest_sha256},
            "stages": [
                {"name": "rust-export", "logs": [
                    {"stream": "stdout", "path": str(rust_log),
                     "sha256": hashlib.sha256(rust_log.read_bytes()).hexdigest()}]},
                {"name": "python-export", "logs": [
                    {"stream": "stdout", "path": str(python_log),
                     "sha256": hashlib.sha256(python_log.read_bytes()).hexdigest()}]},
            ],
        }
        (self.attempt / "attempt-receipt.json").write_text(
            json.dumps(self.receipt, sort_keys=True), encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def evaluate(self, **kwargs):
        arguments = {"root": self.root, "manifest_path": self.manifest_path,
                     "manifest_sha256": self.manifest_sha256,
                     "attempt_dir": self.attempt, "corpus_relative": "corpus.json",
                     "labels_relative": "labels.json", "policy_relative": "policy.json",
                     "expected_policy_relative": "policy.json",
                     "expected_policy_sha256": hashlib.sha256(
                         (self.root / "policy.json").read_bytes()).hexdigest()}
        arguments.update(kwargs)
        with mock.patch.object(c1_acceptance.c1_numeric_policy, "evaluate",
                               return_value={"state": "numeric_pass_not_acceptance"}) as fake:
            return c1_acceptance.evaluate_attempt(**arguments), fake

    def test_evaluates_a_complete_attempt(self):
        artifact, fake = self.evaluate()
        self.assertEqual(artifact["numeric_parity"], "pass")
        self.assertEqual(artifact["encoder_acceptance"],
                         "not_established_by_numeric_parity")
        self.assertEqual(artifact["policy"]["sha256"],
                         self.entries[2]["sha256"])
        self.assertTrue(fake.called)

    def test_receipt_with_an_incomplete_status_is_refused(self):
        self.receipt["status"] = "partial"
        (self.attempt / "attempt-receipt.json").write_text(
            json.dumps(self.receipt, sort_keys=True), encoding="utf-8")
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate()
        self.assertEqual(str(context.exception), "receipt_status")

    def test_receipt_bound_to_another_manifest_is_refused(self):
        self.receipt["manifest"]["sha256"] = "0" * 64
        (self.attempt / "attempt-receipt.json").write_text(
            json.dumps(self.receipt, sort_keys=True), encoding="utf-8")
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate()
        self.assertEqual(str(context.exception), "receipt_manifest_mismatch")

    def test_changed_export_bytes_are_refused(self):
        log = self.attempt / "logs" / "rust-export.stdout.log"
        log.write_text(json.dumps({"identity": {"changed": True}}), encoding="utf-8")
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate()
        self.assertEqual(str(context.exception), "export_digest_mismatch")

    def test_changed_policy_bytes_are_refused(self):
        (self.root / "policy.json").write_bytes(json.dumps(
            {**POLICY, "max_abs_error": 0.5}).encode("utf-8"))
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate()
        self.assertEqual(str(context.exception), "frozen_input_digest_mismatch")

    def test_policy_outside_the_manifest_is_refused(self):
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate(policy_relative="absent.json",
                           expected_policy_relative="absent.json",
                           expected_policy_sha256="0" * 64)
        self.assertEqual(str(context.exception), "frozen_input_not_admitted")

    def test_receipt_path_outside_attempt_dir_is_ignored(self):
        """export_bytes must derive the log path itself, not trust the receipt.

        A forged receipt can claim any path it likes for a log entry. The
        acceptance driver must still read the log from the fixed, attempt-dir
        relative location, never the attacker-controlled path field.
        """
        outside = Path(self.temp.name) / "outside.log"
        outside.write_text(json.dumps({"identity": {"forged": True}}), encoding="utf-8")
        self.receipt["stages"][0]["logs"][0]["path"] = str(outside)
        (self.attempt / "attempt-receipt.json").write_text(
            json.dumps(self.receipt, sort_keys=True), encoding="utf-8")
        # The real log at the canonical path is untouched, so acceptance must
        # use its bytes rather than the forged outside path's bytes.
        artifact, fake = self.evaluate()
        self.assertEqual(artifact["numeric_parity"], "pass")

    def test_symlinked_log_is_refused(self):
        real = self.attempt / "logs" / "elsewhere.log"
        real.write_text(json.dumps({"identity": {}}), encoding="utf-8")
        log = self.attempt / "logs" / "rust-export.stdout.log"
        log.unlink()
        try:
            log.symlink_to(real)
        except (OSError, NotImplementedError):
            self.skipTest("platform does not support symlinks")
        with self.assertRaises(c1_acceptance.AcceptanceError):
            self.evaluate()

    def test_stage_missing_required_name_is_refused(self):
        self.receipt["stages"] = [self.receipt["stages"][0]]
        (self.attempt / "attempt-receipt.json").write_text(
            json.dumps(self.receipt, sort_keys=True), encoding="utf-8")
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate()
        self.assertEqual(str(context.exception), "stage_missing")

    def test_duplicate_required_stage_is_refused(self):
        self.receipt["stages"].append(dict(self.receipt["stages"][0]))
        (self.attempt / "attempt-receipt.json").write_text(
            json.dumps(self.receipt, sort_keys=True), encoding="utf-8")
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate()
        self.assertEqual(str(context.exception), "stage_duplicate")


    def test_policy_digest_not_matching_the_frozen_value_is_refused(self):
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate(expected_policy_sha256="0" * 64)
        self.assertEqual(str(context.exception), "frozen_policy_digest_mismatch")

    def test_policy_relative_not_matching_the_frozen_value_is_refused(self):
        with self.assertRaises(c1_acceptance.AcceptanceError) as context:
            self.evaluate(expected_policy_relative="other-policy.json")
        self.assertEqual(str(context.exception), "frozen_policy_path_mismatch")


if __name__ == "__main__":
    unittest.main()
