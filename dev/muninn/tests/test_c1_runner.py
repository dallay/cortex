"""Synthetic runner tests; never load a model, never touch Neo4j.

Every stage in this file is a tiny stand-in script written into a temporary
directory. The runner is exercised only against those synthetic exporters.
"""
import hashlib
import json
import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest
from unittest.mock import patch

import c1_runner

CORPUS = b'{"corpus_id":"c1-test","batches":[{"id":"mixed","texts":["a","longer text"]}]}'
LABELS = b'{"schema_version":1}'

EXPORTER = '''\
import hashlib
import json
import os
import sys

out_dir = sys.argv[1]
mode = sys.argv[2] if len(sys.argv) > 2 else "stdin"
if mode == "stdin":
    data = sys.stdin.buffer.read()
else:
    with open(mode, "rb") as handle:
        data = handle.read()
marker = os.environ.get("C1_TEST_MARKER")
if marker:
    with open(marker, "a", encoding="utf-8") as handle:
        handle.write("ran\\n")
digest = hashlib.sha256(data).hexdigest()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
with open(os.path.join(out_dir, "embeddings.json"), "w", encoding="utf-8") as handle:
    json.dump({"corpus_sha256": digest, "bytes": len(data)}, handle)
'''

EXPORTER_WRONG_DIGEST = '''\
import hashlib
import json
import os
import sys

out_dir = sys.argv[1]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
with open(os.path.join(out_dir, "embeddings.json"), "w", encoding="utf-8") as handle:
    json.dump({"corpus_sha256": hashlib.sha256(b"other bytes").hexdigest()}, handle)
'''

EXPORTER_FAILS = '''\
import sys

sys.stderr.write("exporter failed on purpose\\n")
sys.exit(3)
'''

EXPORTER_SYMLINK = '''\
import os
import sys

out_dir = sys.argv[1]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
target = os.path.join(out_dir, "target.bin")
with open(target, "wb") as handle:
    handle.write(data)
os.symlink(target, os.path.join(out_dir, "embeddings.json"))
'''

EXPORTER_SYMLINK_DIR = '''\
import os
import sys

out_dir = sys.argv[1]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
elsewhere = os.path.join(os.path.dirname(out_dir), "elsewhere")
os.makedirs(elsewhere, exist_ok=True)
with open(os.path.join(elsewhere, "embeddings.json"), "wb") as handle:
    handle.write(data)
os.rmdir(out_dir)
os.symlink(elsewhere, out_dir)
'''

EXPORTER_MUTATES_INPUT = '''\
import hashlib
import json
import os
import sys

out_dir = sys.argv[1]
frozen = sys.argv[3]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
with open(os.path.join(out_dir, "embeddings.json"), "w", encoding="utf-8") as handle:
    json.dump({"corpus_sha256": hashlib.sha256(data).hexdigest()}, handle)
with open(frozen, "ab") as handle:
    handle.write(b" ")
'''


EXPORTER_REPLACES_MANIFEST = '''\
import hashlib
import json
import os
import sys

out_dir = sys.argv[1]
frozen = sys.argv[3]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
with open(os.path.join(out_dir, "embeddings.json"), "w", encoding="utf-8") as handle:
    json.dump({"corpus_sha256": hashlib.sha256(data).hexdigest()}, handle)
with open(frozen, "rb") as handle:
    original = handle.read()
swap = frozen + ".swap"
with open(swap, "wb") as handle:
    handle.write(original)
os.replace(swap, frozen)
'''


EXPORTER_STDOUT_REPORT = '''\
import hashlib
import json
import os
import sys

out_dir = sys.argv[1]
data = sys.stdin.buffer.read()
os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, "received.bin"), "wb") as handle:
    handle.write(data)
sys.stdout.write(json.dumps({"schema_version": 2, "corpus_id": "c1-test",
                              "input_sha256": hashlib.sha256(data).hexdigest()}))
'''


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":")).encode("utf-8")


class C1RunnerTests(unittest.TestCase):
    def setUp(self):
        # Synthetic exporters only; this mock is not provenance evidence.
        verifier = patch("c1_provenance.verify", return_value={})
        verifier.start()
        self.addCleanup(verifier.stop)
        self.temp = tempfile.TemporaryDirectory()
        self.tmp = Path(self.temp.name)
        self.root = self.tmp / "workspace"
        self.root.mkdir()
        (self.root / "corpus.json").write_bytes(CORPUS)
        (self.root / "labels.json").write_bytes(LABELS)
        self.entries = []
        for name in ("corpus.json", "labels.json"):
            data = (self.root / name).read_bytes()
            self.entries.append({"path": name, "size": len(data),
                                 "sha256": hashlib.sha256(data).hexdigest()})
        self.manifest = {"schema_version": 1, "state": "frozen_pre_execution",
                         "numeric_contract_version": "minilm-cpu-f32-v1",
                         "inputs": self.entries}
        self.manifest_path = self.root / "manifest.json"
        self.write_manifest()
        self.attempt_dir = self.tmp / "attempt"
        self.marker = self.tmp / "marker.txt"

    def tearDown(self):
        self.temp.cleanup()

    def write_manifest(self):
        self.manifest_path.write_text(json.dumps(self.manifest, sort_keys=True), encoding="utf-8")

    def admit_file(self, relative):
        """Register a workspace file as a frozen manifest input."""
        path = self.root / relative
        data = path.read_bytes()
        entry = {"path": relative, "size": len(data),
                 "sha256": hashlib.sha256(data).hexdigest()}
        self.manifest["inputs"] = [item for item in self.manifest["inputs"]
                                   if item["path"] != relative]
        self.manifest["inputs"].append(entry)
        self.manifest["inputs"].sort(key=lambda item: item["path"])
        self.write_manifest()
        return entry

    def digest(self):
        return hashlib.sha256(self.manifest_path.read_bytes()).hexdigest()

    def corpus_digest(self):
        return hashlib.sha256(CORPUS).hexdigest()

    def exporter(self, source=EXPORTER):
        """Write a synthetic exporter whose source lives inside the workspace.

        The exporter source is a frozen input of the attempt, so it is stored
        under the workspace root and registered in the manifest.
        """
        tools = self.root / "tools"
        tools.mkdir(exist_ok=True)
        script = tools / "exporter.py"
        script.write_text(source, encoding="utf-8")
        self.admit_file("tools/exporter.py")
        wrapper = self.tmp / "exporter.sh"
        wrapper.write_text("#!/bin/sh\nexec %s %s \"$@\"\n" % (shlex.quote(sys.executable),
                                                              shlex.quote(str(script))),
                           encoding="utf-8")
        wrapper.chmod(0o755)
        return wrapper, script

    def build(self, wrapper, script):
        return c1_runner.BuildBinding(
            binary=str(wrapper),
            provenance=("synthetic",),
            binary_sha256=hashlib.sha256(wrapper.read_bytes()).hexdigest(),
            sources=(c1_runner.SourceBinding(
                path=str(script),
                sha256=hashlib.sha256(script.read_bytes()).hexdigest()),),
            toolchain=(("python", sys.version.split()[0]),),
        )

    def stage(self, wrapper, script, *, name="c1-export", argv=None, inputs=None,
              outputs=("received.bin", "embeddings.json"),
              consumed=(("embeddings.json", "corpus", "corpus_sha256"),), env=()):
        if argv is None:
            argv = (str(wrapper), "{output_dir}", "stdin")
        if inputs is None:
            inputs = (c1_runner.InputBinding(role="corpus", relative="corpus.json",
                                             delivery="stdin"),)
        return c1_runner.Stage(
            name=name,
            argv=argv,
            build=self.build(wrapper, script),
            inputs=inputs,
            outputs=outputs,
            consumed=tuple(c1_runner.ConsumptionCheck(output=output, role=role, field=field)
                           for output, role, field in consumed),
            env=tuple(env),
        )

    def run_runner(self, stages, attempt_dir=None, after_capture=None):
        return c1_runner.run_attempt(self.root, self.manifest_path, self.digest(), stages,
                                     attempt_dir or self.attempt_dir,
                                     after_capture=after_capture)

    def assert_no_receipt(self, attempt_dir=None):
        attempt_dir = attempt_dir or self.attempt_dir
        self.assertFalse((attempt_dir / c1_runner.RECEIPT_NAME).exists())

    # -- manifest-bound build provenance ---------------------------------

    def test_build_source_outside_the_manifest_is_rejected(self):
        wrapper, _script = self.exporter()
        outside = self.tmp / "outside.py"
        outside.write_text("# not frozen\n", encoding="utf-8")
        stage = c1_runner.Stage(
            name="c1-export",
            argv=(str(wrapper), "{output_dir}", "stdin"),
            build=c1_runner.BuildBinding(
                binary=str(wrapper),
            provenance=("synthetic",),
                binary_sha256=hashlib.sha256(wrapper.read_bytes()).hexdigest(),
                sources=(c1_runner.SourceBinding(
                    path=str(outside),
                    sha256=hashlib.sha256(outside.read_bytes()).hexdigest()),),
                toolchain=(("python", sys.version.split()[0]),),
            ),
            inputs=(c1_runner.InputBinding(role="corpus", relative="corpus.json",
                                           delivery="stdin"),),
            outputs=("received.bin", "embeddings.json"),
            consumed=(c1_runner.ConsumptionCheck(output="embeddings.json", role="corpus",
                                                 field="corpus_sha256"),),
        )
        with self.assertRaises(c1_runner.RunnerError) as context:
            self.run_runner([stage])
        self.assertEqual(context.exception.category, "build_source_not_admitted")

    def test_build_source_digest_must_match_the_manifest_entry(self):
        wrapper, script = self.exporter()
        stage = c1_runner.Stage(
            name="c1-export",
            argv=(str(wrapper), "{output_dir}", "stdin"),
            build=c1_runner.BuildBinding(
                binary=str(wrapper),
            provenance=("synthetic",),
                binary_sha256=hashlib.sha256(wrapper.read_bytes()).hexdigest(),
                sources=(c1_runner.SourceBinding(
                    path=str(script),
                    sha256=hashlib.sha256(b"other bytes").hexdigest()),),
                toolchain=(("python", sys.version.split()[0]),),
            ),
            inputs=(c1_runner.InputBinding(role="corpus", relative="corpus.json",
                                           delivery="stdin"),),
            outputs=("received.bin", "embeddings.json"),
            consumed=(c1_runner.ConsumptionCheck(output="embeddings.json", role="corpus",
                                                 field="corpus_sha256"),),
        )
        with self.assertRaises(c1_runner.RunnerError) as context:
            self.run_runner([stage])
        self.assertIn(context.exception.category,
                      ("build_source_digest_mismatch", "build_source_manifest_digest_mismatch"))

    def test_admitted_build_source_is_recorded_in_the_receipt(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script)
        receipt = self.run_runner([stage])
        source = receipt["stages"][0]["build"]["sources"][0]
        self.assertEqual(source["sha256"], hashlib.sha256(script.read_bytes()).hexdigest())
        self.assertTrue(source["admitted"])

    # -- happy path ----------------------------------------------------

    def test_receipt_binds_manifest_sources_build_runtime_outputs_and_logs(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, env=(("C1_TEST_MARKER", str(self.marker)),))
        receipt = self.run_runner([stage])

        self.assertEqual(receipt["status"], "complete")
        self.assertEqual(receipt["numeric_contract_version"], "minilm-cpu-f32-v1")
        self.assertEqual(receipt["manifest"]["sha256"], self.digest())
        self.assertIn({"path": "corpus.json", "size": len(CORPUS),
                       "sha256": self.corpus_digest()}, receipt["frozen_inputs"])

        record = receipt["stages"][0]
        self.assertEqual(record["build"]["binary_sha256"],
                         hashlib.sha256(wrapper.read_bytes()).hexdigest())
        self.assertEqual(record["build"]["sources"][0]["sha256"],
                         hashlib.sha256(script.read_bytes()).hexdigest())
        self.assertIn("python", receipt["runtime"])
        self.assertIn("platform", receipt["runtime"])

        outputs = {item["name"]: item for item in record["outputs"]}
        self.assertEqual(outputs["received.bin"]["sha256"], self.corpus_digest())
        self.assertEqual(outputs["embeddings.json"]["size"],
                         (self.attempt_dir / "outputs" / "c1-export" / "embeddings.json").stat().st_size)
        self.assertEqual((self.attempt_dir / "outputs" / "c1-export" / "received.bin").read_bytes(),
                         CORPUS)
        self.assertIn("stdout", {item["stream"] for item in record["logs"]})
        self.assertIn("stderr", {item["stream"] for item in record["logs"]})
        for item in record["logs"]:
            self.assertTrue(Path(item["path"]).exists())
            self.assertEqual(item["sha256"],
                             hashlib.sha256(Path(item["path"]).read_bytes()).hexdigest())

        consumed = record["consumed"][0]
        self.assertEqual(consumed["role"], "corpus")
        self.assertEqual(consumed["snapshot_sha256"], self.corpus_digest())
        self.assertEqual(consumed["reported_sha256"], self.corpus_digest())
        self.assertTrue((self.attempt_dir / c1_runner.RECEIPT_NAME).exists())

    def test_receipt_hash_covers_its_own_body(self):
        wrapper, script = self.exporter()
        receipt = self.run_runner([self.stage(wrapper, script)])
        on_disk = json.loads((self.attempt_dir / c1_runner.RECEIPT_NAME).read_text(encoding="utf-8"))
        claimed = on_disk.pop("receipt_sha256")
        self.assertEqual(hashlib.sha256(canonical(on_disk)).hexdigest(), claimed)
        self.assertEqual(claimed, receipt["receipt_sha256"])

    def test_stdin_delivery_hands_the_exact_snapshot_bytes(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script)
        self.run_runner([stage])
        received = (self.attempt_dir / "outputs" / "c1-export" / "received.bin").read_bytes()
        self.assertEqual(received, CORPUS)

    def test_path_delivery_hands_the_exact_snapshot_bytes(self):
        wrapper, script = self.exporter()
        stage = c1_runner.Stage(
            name="c1-export",
            argv=(str(wrapper), "{output_dir}", "{corpus}"),
            build=self.build(wrapper, script),
            inputs=(c1_runner.InputBinding(role="corpus", relative="corpus.json",
                                           delivery="path"),),
            outputs=("received.bin", "embeddings.json"),
            consumed=(c1_runner.ConsumptionCheck(output="embeddings.json", role="corpus",
                                                 field="corpus_sha256"),),
        )
        receipt = self.run_runner([stage])
        self.assertEqual(receipt["stages"][0]["consumed"][0]["reported_sha256"],
                         self.corpus_digest())
        received = (self.attempt_dir / "outputs" / "c1-export" / "received.bin").read_bytes()
        self.assertEqual(received, CORPUS)

    def test_stdout_report_is_verified_against_the_snapshot(self):
        wrapper, script = self.exporter(EXPORTER_STDOUT_REPORT)
        stage = c1_runner.Stage(
            name="c1-export",
            argv=(str(wrapper), "{output_dir}", "stdin"),
            build=self.build(wrapper, script),
            inputs=(c1_runner.InputBinding(role="corpus", relative="corpus.json",
                                           delivery="stdin"),),
            outputs=("received.bin",),
            consumed=(c1_runner.ConsumptionCheck(output="stdout", role="corpus",
                                                 field="input_sha256"),),
        )
        receipt = self.run_runner([stage])
        consumed = receipt["stages"][0]["consumed"][0]
        self.assertEqual(consumed["output"], "stdout")
        self.assertEqual(consumed["reported_sha256"], self.corpus_digest())
        stdout_log = Path(receipt["stages"][0]["logs"][0]["path"])
        self.assertEqual(json.loads(stdout_log.read_text(encoding="utf-8"))["input_sha256"],
                         self.corpus_digest())

    def test_rejects_stdout_report_with_a_digest_other_than_the_snapshot(self):
        wrapper, script = self.exporter(EXPORTER_STDOUT_REPORT)
        stage = c1_runner.Stage(
            name="c1-export",
            argv=(str(wrapper), "{output_dir}", "stdin"),
            build=self.build(wrapper, script),
            inputs=(c1_runner.InputBinding(role="corpus", relative="corpus.json",
                                           delivery="stdin"),),
            outputs=("received.bin",),
            consumed=(c1_runner.ConsumptionCheck(output="stdout", role="corpus",
                                                 field="input_sha256"),),
        )
        original = script.read_text(encoding="utf-8")
        script.write_text(original.replace("hashlib.sha256(data).hexdigest()",
                                           "hashlib.sha256(b'other').hexdigest()"),
                          encoding="utf-8")
        self.admit_file("tools/exporter.py")
        stage = c1_runner.Stage(
            name=stage.name,
            argv=stage.argv,
            build=self.build(wrapper, script),
            inputs=stage.inputs,
            outputs=stage.outputs,
            consumed=stage.consumed,
        )
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "output_consumption_mismatch")
        self.assert_no_receipt()

    def test_rejects_path_placeholder_for_a_stdin_only_role(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, argv=(str(wrapper), "{output_dir}", "{corpus}"))
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "stage_argv_placeholder_unknown")
        self.assert_no_receipt()

    def test_rejects_stage_name_that_escapes_the_attempt_directory(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, name="../escape")
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "stage_name_argument")
        self.assert_no_receipt()

    # -- manifest and input alteration ----------------------------------

    def test_rejects_manifest_altered_after_the_trusted_digest(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, env=(("C1_TEST_MARKER", str(self.marker)),))
        self.manifest["inputs"][0]["size"] = len(CORPUS) + 1
        self.write_manifest()
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "admission_failed")
        self.assertFalse(self.marker.exists())
        self.assert_no_receipt()

    def test_rejects_frozen_input_altered_after_the_trusted_digest(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, env=(("C1_TEST_MARKER", str(self.marker)),))
        (self.root / "corpus.json").write_bytes(CORPUS + b" ")
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "admission_failed")
        self.assertFalse(self.marker.exists())
        self.assert_no_receipt()

    def test_rejects_frozen_input_mutated_during_execution(self):
        wrapper, script = self.exporter(EXPORTER_MUTATES_INPUT)
        stage = self.stage(wrapper, script, argv=(str(wrapper), "{output_dir}", "stdin",
                                                   str(self.root / "corpus.json")))
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "frozen_inputs_changed")
        self.assert_no_receipt()

    def test_rejects_manifest_replaced_with_identical_bytes(self):
        wrapper, script = self.exporter(EXPORTER_REPLACES_MANIFEST)
        stage = self.stage(wrapper, script, argv=(str(wrapper), "{output_dir}", "stdin",
                                                   str(self.manifest_path)))
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "manifest_replaced")
        self.assert_no_receipt()

    # -- output substitution --------------------------------------------

    def test_rejects_preexisting_attempt_directory(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, env=(("C1_TEST_MARKER", str(self.marker)),))
        self.attempt_dir.mkdir()
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "attempt_dir_preexisting")
        self.assertFalse(self.marker.exists())
        self.assert_no_receipt()

    def test_rejects_symlinked_output_file(self):
        wrapper, script = self.exporter(EXPORTER_SYMLINK)
        stage = self.stage(wrapper, script)
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "output_invalid")
        self.assert_no_receipt()

    def test_rejects_output_directory_replaced_by_symlink(self):
        wrapper, script = self.exporter(EXPORTER_SYMLINK_DIR)
        stage = self.stage(wrapper, script)
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "output_dir_invalid")
        self.assert_no_receipt()

    def test_rejects_output_mutated_after_capture(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script)
        target = self.attempt_dir / "outputs" / "c1-export" / "received.bin"

        def mutate(stage_record, attempt_dir):
            (attempt_dir / "outputs" / "c1-export" / "received.bin").write_bytes(b"tampered")

        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage], after_capture=mutate)
        self.assertEqual(caught.exception.category, "output_mutated")
        self.assert_no_receipt()
        self.assertTrue(target.exists())

    # -- source to binary binding ---------------------------------------

    def test_rejects_source_changed_after_the_build_record(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, env=(("C1_TEST_MARKER", str(self.marker)),))
        script.write_text(EXPORTER.replace("corpus_sha256", "corpus_sha256_mutated"),
                          encoding="utf-8")
        self.admit_file("tools/exporter.py")
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "build_source_digest_mismatch")
        self.assertFalse(self.marker.exists())
        self.assert_no_receipt()

    def test_rejects_binary_changed_after_the_build_record(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, env=(("C1_TEST_MARKER", str(self.marker)),))
        wrapper.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        wrapper.chmod(0o755)
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "build_binary_digest_mismatch")
        self.assertFalse(self.marker.exists())
        self.assert_no_receipt()

    # -- consumption and completion --------------------------------------

    def test_rejects_consumption_different_from_the_snapshot(self):
        wrapper, script = self.exporter(EXPORTER_WRONG_DIGEST)
        stage = self.stage(wrapper, script)
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "output_consumption_mismatch")
        self.assert_no_receipt()

    def test_rejects_incomplete_execution(self):
        wrapper, script = self.exporter(EXPORTER_FAILS)
        stage = self.stage(wrapper, script, outputs=("received.bin", "embeddings.json"))
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "stage_failed")
        self.assert_no_receipt()

    def test_rejects_missing_output_file(self):
        wrapper, script = self.exporter()
        stage = self.stage(wrapper, script, outputs=("received.bin", "embeddings.json",
                                                     "missing.json"))
        with self.assertRaises(c1_runner.RunnerError) as caught:
            self.run_runner([stage])
        self.assertEqual(caught.exception.category, "output_missing")
        self.assert_no_receipt()


if __name__ == "__main__":
    unittest.main()
