"""Inventory regressions; scanner integration is exercised by the gate itself."""

import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    "gate", Path(__file__).with_name("check-semgrep-path-exceptions.py"))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="semgrep-inventory-test-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        for filename in {row[0] for row in gate.NEW + gate.OLD} | {gate.DOC}:
            target = self.root / filename
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(gate.ROOT / filename, target)

    def mutate(self, filename, original, replacement):
        target = self.root / filename
        content = target.read_text()
        self.assertIn(original, content)
        target.write_text(content.replace(original, replacement, 1))
        with self.assertRaises(ValueError):
            gate.check_inventory(self.root)

    def test_exact_inventory_passes(self):
        gate.check_inventory(self.root)

    def test_missing_new_annotation_fails(self):
        self.mutate(gate.NEW[0][0], f"// nosemgrep: {gate.RULE}", "// removed")

    def test_sink_change_fails(self):
        self.mutate(gate.NEW[0][0], gate.NEW[0][2], "match another_read(&resolved) {")

    def test_function_change_fails(self):
        self.mutate(gate.NEW[0][0], "pub fn load(", "pub fn renamed(")

    def test_reason_change_fails(self):
        self.mutate(gate.NEW[0][0], gate.NEW[0][4], "Different reason.")

    def test_evidence_link_change_fails(self):
        self.mutate(gate.NEW[0][0], "#huginn-config", "#other")

    def test_missing_historical_annotation_fails(self):
        self.mutate("apps/rook/src/main.rs", f"// nosemgrep: {gate.RULE}", "// removed")

    def test_added_annotation_in_untracked_file_fails(self):
        (self.root / "extra.rs").write_text(f"// nosemgrep: {gate.RULE}\nfn extra() {{}}\n")
        with self.assertRaises(ValueError):
            gate.check_inventory(self.root)

    def test_unscoped_annotation_fails(self):
        (self.root / "extra.rs").write_text("// nosemgrep\nfn extra() {}\n")
        with self.assertRaises(ValueError):
            gate.check_inventory(self.root)


class CiWiringTests(unittest.TestCase):
    def test_inventory_regressions_are_run_in_ci(self):
        workflow = (gate.ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn(
            "\n          python3 .github/scripts/test-semgrep-path-exceptions.py\n",
            workflow,
        )


if __name__ == "__main__":
    unittest.main()
