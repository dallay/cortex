"""Inventory regressions; scanner integration is exercised by the gate itself."""

import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


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

    def test_cli_rejects_semgrep_command_override(self):
        with (
            tempfile.TemporaryDirectory(prefix="semgrep-cli-evidence-") as output,
            patch.object(gate, "ROOT", self.root),
            patch.object(gate, "check_inventory"),
            patch.object(gate, "scanner_controls") as scanner_controls,
            patch("sys.argv", ["check-semgrep-path-exceptions.py", "--output-dir", output]),
        ):
            gate.main()
        self.assertEqual(scanner_controls.call_args.args[0], Path(output))

        with patch("sys.argv", ["check-semgrep-path-exceptions.py", "--semgrep-command", "sh"]):
            with self.assertRaises(SystemExit) as error:
                gate.main()
        self.assertEqual(error.exception.code, 2)

    def test_cli_rejects_evidence_inside_the_source_tree_before_scanning(self):
        for output in [self.root.resolve(), (self.root / "nested/evidence").resolve()]:
            with (
                self.subTest(output=output),
                patch.object(gate, "ROOT", self.root.resolve()),
                patch.object(gate, "scanner_controls") as scanner,
                patch.object(gate, "check_inventory") as inventory,
                patch("sys.argv", ["check-semgrep-path-exceptions.py", "--output-dir", str(output)]),
            ):
                with self.assertRaisesRegex(ValueError, "evidence must be outside source tree"):
                    gate.main()
                scanner.assert_not_called()
                inventory.assert_not_called()

    def mutate(self, filename, original, replacement):
        target = self.root / filename
        content = target.read_text()
        self.assertIn(original, content)
        target.write_text(content.replace(original, replacement, 1))
        with self.assertRaises(ValueError):
            gate.check_inventory(self.root)

    def test_semgrep_fixture_git_visibility_is_initialized(self):
        target = self.root / "positive.rs"
        target.write_text("fn read(path: &str) -> std::io::Result<String> {\n    std::fs::read_to_string(path)\n}\n")
        gate.initialize_fixture_repository(self.root, target)
        status = subprocess.run(
            ["git", "-C", str(self.root), "status", "--short", "--", target.name],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        self.assertEqual(status, "")
        tracked = subprocess.run(
            ["git", "-C", str(self.root), "ls-files", "--error-unmatch", "--", target.name],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(tracked.returncode, 0, tracked.stderr)

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

    def test_missing_documentation_anchor_fails(self):
        self.mutate(gate.DOC, "## huginn-config\n", "## renamed-config\n")

    def test_annotation_cannot_be_detached_from_its_sink(self):
        filename, _, sink, _, _ = gate.NEW[0]
        self.mutate(filename, sink, f"// intervening comment\n{sink}")

    def test_scoped_annotations_cannot_add_another_rule(self):
        self.mutate(gate.NEW[0][0], f"// nosemgrep: {gate.RULE}",
                    f"// nosemgrep: {gate.RULE}, another.rule")

    def test_unscoped_shorthand_and_empty_rule_lists_fail(self):
        for annotation in ["// nosem", "// nosem:", "// nosemgrep:   "]:
            with self.subTest(annotation=annotation):
                (self.root / "extra.rs").write_text(f"{annotation}\nfn extra() {{}}\n")
                with self.assertRaisesRegex(ValueError, "unscoped suppression"):
                    gate.check_inventory(self.root)

    def test_tracked_sources_are_checked_alongside_untracked_sources(self):
        subprocess.run(["git", "-C", str(self.root), "add", "."], check=True)
        gate.check_inventory(self.root)
        self.mutate(gate.NEW[0][0], gate.NEW[0][2], "match other_read(&resolved) {")


class ScannerControlTests(unittest.TestCase):
    """Test gate decisions with scanner responses, without fetching remote rules."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="semgrep-control-test-")
        self.addCleanup(directory.cleanup)
        self.output = Path(directory.name)

    def scan_result(self, argv, *, cwd, **kwargs):
        target = argv[-1]
        name = Path(target).stem
        finding = {"check_id": gate.RULE, "extra": {"is_ignored": name != "positive"}}
        data = {
            "version": "test-engine",
            "paths": {"scanned": [target]},
            "errors": [],
            "results": [] if name == "conversion" else [finding],
        }
        self.mutate_response(name, data)
        return subprocess.CompletedProcess(
            argv, self.exit_codes.get(name, 1 if name == "positive" else 0),
            json.dumps(data), "scanner diagnostic",
        )

    def run_controls(self, mutate_response=lambda name, data: None, exit_codes=None):
        self.mutate_response = mutate_response
        self.exit_codes = exit_codes or {}
        with (
            patch.object(gate, "initialize_fixture_repository") as initialize,
            patch.object(gate.subprocess, "run", side_effect=self.scan_result) as scan,
        ):
            gate.scanner_controls(self.output)
        return initialize, scan

    def test_all_four_controls_use_the_real_rule_configuration_and_save_evidence(self):
        initialize, scan = self.run_controls()
        self.assertEqual(initialize.call_count, 4)
        self.assertEqual(scan.call_count, 4)
        for call in scan.call_args_list:
            argv = call.args[0]
            self.assertEqual(argv[:2], ["semgrep", "scan"])
            self.assertEqual(argv[argv.index("--config") + 1], "p/rust")
            self.assertEqual(argv[argv.index("--severity") + 1], "ERROR")
            self.assertEqual("--disable-nosem" in argv, argv[-1] == "disabled.rs")
            self.assertEqual(Path(call.kwargs["cwd"]).resolve(),
                             Path(argv[argv.index("--project-root") + 1]))
        for name in ["positive", "annotated", "disabled", "conversion"]:
            self.assertEqual(json.loads((self.output / f"{name}.json").read_text())["version"],
                             "test-engine")
            self.assertEqual((self.output / f"{name}.stderr").read_text(), "scanner diagnostic")

    def test_scanner_failures_are_rejected_even_with_expected_finding_counts(self):
        mutations = {
            "missing scanned file": lambda data: data["paths"].update(scanned=[]),
            "extra scanned file": lambda data: data["paths"]["scanned"].append("extra.rs"),
            "scanner error": lambda data: data["errors"].append({"message": "parse failed"}),
            "wrong rule": lambda data: data["results"][0].update(check_id="other.rule"),
            "missing positive": lambda data: data.update(results=[]),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.run_controls(lambda name, data: mutate(data) if name == "positive" else None)

    def test_ignored_finding_still_counts_when_suppression_is_disabled(self):
        with self.assertRaisesRegex(ValueError, "unexpected active findings"):
            self.run_controls(lambda name, data: data.update(results=[])
                              if name == "disabled" else None)

    def test_suppressed_and_conversion_controls_cannot_have_active_findings(self):
        for control in ["annotated", "conversion"]:
            def activate(name, data):
                if name == control:
                    data["results"] = [{"check_id": gate.RULE, "extra": {"is_ignored": False}}]

            with self.subTest(control=control), self.assertRaises(ValueError):
                self.run_controls(activate)

    def test_unexpected_exit_status_fails_each_control(self):
        for name in ["positive", "annotated", "disabled", "conversion"]:
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.run_controls(exit_codes={name: 2})


class CiWiringTests(unittest.TestCase):
    def test_inventory_regressions_are_run_in_ci(self):
        workflow = (gate.ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn(
            "\n          python3 .github/scripts/test-semgrep-path-exceptions.py\n",
            workflow,
        )


if __name__ == "__main__":
    unittest.main()
