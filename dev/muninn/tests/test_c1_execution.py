"""Synthetic provenance-gate tests; never load a model or touch Neo4j."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

import c1_execution


class C1ExecutionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "corpus.json").write_bytes(b'{"corpus_id":"c1-test"}')
        (self.root / "labels.json").write_bytes(b'{"schema_version":1}')
        self.entries = []
        for name in ("corpus.json", "labels.json"):
            path = self.root / name
            data = path.read_bytes()
            self.entries.append({"path": name, "size": len(data),
                                 "sha256": hashlib.sha256(data).hexdigest()})
        self.manifest = {"schema_version": 1, "state": "frozen_pre_execution",
                         "numeric_contract_version": "minilm-cpu-f32-v1",
                         "inputs": self.entries}
        self.manifest_path = self.root / "manifest.json"
        self.write_manifest()

    def tearDown(self):
        self.temp.cleanup()

    def write_manifest(self):
        self.manifest_path.write_text(json.dumps(self.manifest, sort_keys=True), encoding="utf-8")

    def digest(self):
        return hashlib.sha256(self.manifest_path.read_bytes()).hexdigest()

    def test_admission_accepts_only_authentic_frozen_manifest_and_inputs(self):
        admitted = c1_execution.admit(self.root, self.manifest_path, self.digest())
        self.assertEqual(admitted.contract, "minilm-cpu-f32-v1")
        self.assertEqual(admitted.input_paths, ("corpus.json", "labels.json"))

    def test_admission_rejects_manifest_digest_substitution(self):
        with self.assertRaises(c1_execution.ProvenanceError):
            c1_execution.admit(self.root, self.manifest_path, "0" * 64)

    def test_admission_rejects_changed_frozen_input(self):
        (self.root / "corpus.json").write_bytes(b'{"corpus_id":"changed"}')
        with self.assertRaises(c1_execution.ProvenanceError):
            c1_execution.admit(self.root, self.manifest_path, self.digest())

    def test_snapshot_records_exact_bytes_consumed_and_detects_post_read_change(self):
        admitted = c1_execution.admit(self.root, self.manifest_path, self.digest())
        snapshot = c1_execution.read_frozen_input(admitted, "corpus.json")
        self.assertEqual(snapshot.data, (self.root / "corpus.json").read_bytes())
        self.assertEqual(snapshot.sha256, self.entries[0]["sha256"])
        (self.root / "corpus.json").write_bytes(b"changed")
        with self.assertRaises(c1_execution.ProvenanceError):
            c1_execution.verify_unchanged(admitted)

    def test_admission_rejects_unsupported_numeric_contract(self):
        self.manifest["numeric_contract_version"] = "other-v1"
        self.write_manifest()
        with self.assertRaises(c1_execution.ProvenanceError):
            c1_execution.admit(self.root, self.manifest_path, self.digest())

    def test_admission_rejects_boolean_schema_version(self):
        self.manifest["schema_version"] = True
        self.write_manifest()
        with self.assertRaises(c1_execution.ProvenanceError):
            c1_execution.admit(self.root, self.manifest_path, self.digest())

    def test_snapshot_rejects_intermediate_symlink_even_when_target_is_inside_root(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.manifest["inputs"][0]["path"] = "alias/corpus.json"
        self.write_manifest()
        admitted = c1_execution.admit(self.root, self.manifest_path, self.digest())
        with self.assertRaises(c1_execution.ProvenanceError):
            c1_execution.read_frozen_input(admitted, "alias/corpus.json")

    def test_snapshot_rejects_untrusted_constructed_traversal_record(self):
        outside = self.root.parent / "outside-c1-input"
        outside.write_bytes(b"outside")
        try:
            forged = c1_execution.AdmittedInputs(
                self.root, self.manifest_path, self.digest(), "minilm-cpu-f32-v1",
                (("../outside-c1-input", 7, hashlib.sha256(b"outside").hexdigest()),),
                ("../outside-c1-input",))
            with self.assertRaises(c1_execution.ProvenanceError):
                c1_execution.read_frozen_input(forged, "../outside-c1-input")
        finally:
            outside.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
