"""Synthetic tests for the C1 manifest composition; no inference, no Neo4j."""
import json
import os
from pathlib import Path
import tempfile
import unittest

import c1_manifest_build
import c1_freeze


class C1ManifestBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        (self.root / "dev/muninn").mkdir(parents=True)
        (self.root / "crates/muninn/c1-exporter/src").mkdir(parents=True)
        (self.root / "crates/muninn/c1-exporter/examples").mkdir(parents=True)
        (self.root / "third_party/candle-core/src").mkdir(parents=True)
        (self.root / "docs/muninn/evidence").mkdir(parents=True)
        for relative in c1_manifest_build.SOURCE_TREES:
            tree = self.root / relative
            tree.mkdir(parents=True, exist_ok=True)
            (tree / "fixture.rs").write_text("// fixture\n", encoding="utf-8")
        (self.root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
        (self.root / "Cargo.lock").write_text("# lock\n", encoding="utf-8")
        (self.root / "rust-toolchain.toml").write_text("[toolchain]\n", encoding="utf-8")
        (self.root / "crates/muninn/c1-exporter/Cargo.toml").write_text("[package]\n", encoding="utf-8")
        (self.root / "crates/muninn/c1-exporter/src/lib.rs").write_text("// lib\n", encoding="utf-8")
        (self.root / "crates/muninn/c1-exporter/src/encoder.rs").write_text("// enc\n", encoding="utf-8")
        (self.root / "crates/muninn/c1-exporter/src/main.rs").write_text(
            "// ex\n", encoding="utf-8")
        (self.root / "third_party/candle-core/Cargo.toml").write_text("[package]\n",
                                                                      encoding="utf-8")
        (self.root / "third_party/candle-core/src/lib.rs").write_text("// core\n",
                                                                      encoding="utf-8")
        (self.root / "third_party/candle-core/target").mkdir()
        (self.root / "third_party/candle-core/target/junk.bin").write_bytes(b"junk")
        (self.root / "docs/muninn/evidence/policy.json").write_text("{}\n", encoding="utf-8")
        for name in ("c1_runner.py", "c1_execution.py", "c1_freeze.py"):
            (self.root / "dev/muninn" / name).write_text("# %s\n" % name, encoding="utf-8")
        for relative in c1_manifest_build.SOURCE_FILES:
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            if not path.exists():
                path.write_text("# %s\n" % relative, encoding="utf-8")
        self.prior = self.root / "prior.json"
        self.prior.write_text(json.dumps({
            "schema_version": 1,
            "state": "frozen_pre_execution",
            "numeric_contract_version": "minilm-cpu-f32-v1",
            "inputs": [{"path": "docs/muninn/evidence/policy.json", "size": 3,
                        "sha256": "0" * 64}],
        }), encoding="utf-8")

    def tearDown(self):
        self.temp.cleanup()

    def test_collects_prior_build_inputs_and_sources(self):
        paths = c1_manifest_build.collect_inputs(self.root, self.prior)
        relatives = {path.relative_to(Path(os.path.realpath(str(self.root)))).as_posix()
                     for path in paths}
        self.assertIn("docs/muninn/evidence/policy.json", relatives)
        self.assertIn("Cargo.toml", relatives)
        self.assertIn("Cargo.lock", relatives)
        self.assertIn("rust-toolchain.toml", relatives)
        self.assertIn("crates/muninn/c1-exporter/Cargo.toml", relatives)
        self.assertIn("crates/muninn/c1-exporter/src/lib.rs", relatives)
        self.assertIn("crates/muninn/c1-exporter/src/encoder.rs", relatives)
        self.assertIn("crates/muninn/c1-exporter/src/main.rs", relatives)
        self.assertIn("third_party/candle-core/src/lib.rs", relatives)
        self.assertIn("dev/muninn/c1_runner.py", relatives)
        self.assertNotIn("third_party/candle-core/target/junk.bin", relatives)

    def test_rejects_a_prior_manifest_with_a_missing_input(self):
        self.prior.write_text(json.dumps({
            "schema_version": 1,
            "state": "frozen_pre_execution",
            "numeric_contract_version": "minilm-cpu-f32-v1",
            "inputs": [{"path": "docs/muninn/evidence/absent.json", "size": 1,
                        "sha256": "0" * 64}],
        }), encoding="utf-8")
        with self.assertRaises(c1_manifest_build.ManifestError):
            c1_manifest_build.collect_inputs(self.root, self.prior)

    def test_built_manifest_verifies_against_the_workspace(self):
        manifest = c1_manifest_build.build_manifest(self.root, self.prior)
        c1_freeze.verify(self.root, manifest)
        self.assertEqual(manifest["numeric_contract_version"], "minilm-cpu-f32-v1")
        self.assertGreater(len(manifest["inputs"]), 10)

    def test_write_refuses_to_overwrite_an_existing_manifest(self):
        manifest = c1_manifest_build.build_manifest(self.root, self.prior)
        output = self.root / "next.json"
        c1_manifest_build.write_manifest(manifest, output)
        with self.assertRaises(c1_manifest_build.ManifestError):
            c1_manifest_build.write_manifest(manifest, output)

    def test_missing_source_file_fails_closed(self):
        (self.root / "rust-toolchain.toml").unlink()
        with self.assertRaises(c1_manifest_build.ManifestError):
            c1_manifest_build.collect_inputs(self.root, self.prior)

    def test_symlinked_file_in_source_tree_fails_closed(self):
        target = self.root / "crates/muninn/c1-exporter/src/real.rs"
        target.write_text("// real\n", encoding="utf-8")
        link = self.root / "crates/muninn/c1-exporter/src/linked.rs"
        try:
            link.symlink_to(target)
        except (OSError, NotImplementedError):
            self.skipTest("platform does not support symlinks")
        with self.assertRaises(c1_manifest_build.ManifestError):
            c1_manifest_build.collect_inputs(self.root, self.prior)

    def test_symlinked_directory_in_source_tree_fails_closed(self):
        target_dir = self.root / "crates/muninn/c1-exporter/src_real"
        target_dir.mkdir()
        (target_dir / "nested.rs").write_text("// nested\n", encoding="utf-8")
        link_dir = self.root / "crates/muninn/c1-exporter/src/linked_dir"
        try:
            link_dir.symlink_to(target_dir, target_is_directory=True)
        except (OSError, NotImplementedError):
            self.skipTest("platform does not support symlinks")
        with self.assertRaises(c1_manifest_build.ManifestError):
            c1_manifest_build.collect_inputs(self.root, self.prior)


if __name__ == "__main__":
    unittest.main()
