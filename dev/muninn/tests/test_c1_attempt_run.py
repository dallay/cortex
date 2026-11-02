"""Synthetic tests for the C1 attempt driver; no inference, no Neo4j."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import c1_attempt_run
import c1_runner

CORPUS = b'{"corpus_id":"c1-test","batches":[{"id":"mixed","texts":["a","b"]}]}'

EXPORTER = '''\
import hashlib
import json
import os
import sys

out_dir = sys.argv[1]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
sys.stdout.write(json.dumps({"input_sha256": hashlib.sha256(data).hexdigest()}))
'''


class C1AttemptRunTests(unittest.TestCase):
    def setUp(self):
        # Synthetic exporters only; never evidence of real attestation.
        verifier = patch("c1_provenance.verify", side_effect=lambda binary, *args: {"binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest()})
        verifier.start()
        self.addCleanup(verifier.stop)
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "corpus.json").write_bytes(CORPUS)
        (self.root / "tools").mkdir()
        script = self.root / "tools/exporter.py"
        script.write_text(EXPORTER, encoding="utf-8")
        wrapper = self.root / "tools/exporter.sh"
        wrapper.write_text("#!/bin/sh\nexec %s %s \"$@\"\n" % (sys.executable, script),
                           encoding="utf-8")
        wrapper.chmod(0o755)
        self.entries = []
        for relative in ("corpus.json", "tools/exporter.py"):
            data = (self.root / relative).read_bytes()
            self.entries.append({"path": relative, "size": len(data),
                                 "sha256": hashlib.sha256(data).hexdigest()})
        self.manifest = {"schema_version": 1, "state": "frozen_pre_execution",
                         "numeric_contract_version": "minilm-cpu-f32-v1",
                         "inputs": self.entries}
        self.manifest_path = self.root / "manifest.json"
        self.write_manifest()
        self.plan = {
            "schema_version": 1,
            "numeric_contract_version": "minilm-cpu-f32-v1",
            "stages": [{
                "name": "c1-export",
                "provenance": {"record": "synthetic", "bundle": "synthetic",
                               "source_commit": "a" * 40, "toolchain": "synthetic"},
                "binary": "tools/exporter.sh",
                "argv": ["{binary}", "{output_dir}"],
                "files": ["tools/exporter.py"],
                "toolchain": [["python", sys.version.split()[0]]],
                "inputs": [{"role": "corpus", "relative": "corpus.json",
                            "delivery": "stdin"}],
                "outputs": ["received.bin"],
                "consumed": [{"output": "stdout", "role": "corpus",
                              "field": "input_sha256"}],
            }],
        }
        self.plan_path = self.root / "plan.json"
        self.write_plan()

    def tearDown(self):
        self.temp.cleanup()

    def write_manifest(self):
        self.manifest_path.write_text(json.dumps(self.manifest, sort_keys=True), encoding="utf-8")

    def write_plan(self):
        self.plan_path.write_text(json.dumps(self.plan, sort_keys=True), encoding="utf-8")
        data = self.plan_path.read_bytes()
        self.manifest["inputs"] = [item for item in self.manifest["inputs"]
                                   if item["path"] != "plan.json"]
        self.manifest["inputs"].append({"path": "plan.json", "size": len(data),
                                        "sha256": hashlib.sha256(data).hexdigest()})
        self.write_manifest()

    def manifest_digest(self):
        return hashlib.sha256(self.manifest_path.read_bytes()).hexdigest()

    def test_runs_a_planned_stage_and_writes_a_receipt(self):
        attempt_dir = self.root / "attempt"
        receipt = c1_attempt_run.run_plan(self.root, self.manifest_path,
                                          self.manifest_digest(), self.plan_path, attempt_dir)
        self.assertEqual(receipt["status"], "complete")
        self.assertTrue((attempt_dir / c1_runner.RECEIPT_NAME).is_file())
        self.assertEqual(receipt["stages"][0]["build"]["sources"][0]["admitted"], True)

    def test_plan_outside_the_manifest_is_refused(self):
        self.manifest["inputs"] = [item for item in self.manifest["inputs"]
                                   if item["path"] != "plan.json"]
        self.write_manifest()
        with self.assertRaises(c1_attempt_run.PlanError) as context:
            c1_attempt_run.run_plan(self.root, self.manifest_path, self.manifest_digest(),
                                    self.plan_path, self.root / "attempt")
        self.assertEqual(str(context.exception), "plan_not_admitted")

    def test_source_outside_the_manifest_is_refused(self):
        self.plan["stages"][0]["files"] = ["tools/absent.py"]
        self.write_plan()
        with self.assertRaises(c1_attempt_run.PlanError) as context:
            c1_attempt_run.run_plan(self.root, self.manifest_path, self.manifest_digest(),
                                    self.plan_path, self.root / "attempt")
        self.assertEqual(str(context.exception), "source_not_admitted")

    def test_manifest_digest_mismatch_is_refused(self):
        with self.assertRaises(c1_attempt_run.PlanError) as context:
            c1_attempt_run.run_plan(self.root, self.manifest_path, "0" * 64, self.plan_path,
                                    self.root / "attempt")
        self.assertEqual(str(context.exception), "manifest_digest")

    def test_changed_source_bytes_are_refused(self):
        (self.root / "tools/exporter.py").write_text("# changed\n", encoding="utf-8")
        with self.assertRaises(c1_attempt_run.PlanError) as context:
            c1_attempt_run.run_plan(self.root, self.manifest_path, self.manifest_digest(),
                                    self.plan_path, self.root / "attempt")
        self.assertEqual(str(context.exception), "source_digest_mismatch")


if __name__ == "__main__":
    unittest.main()
