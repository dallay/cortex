"""Regresiones del runner TCK sin proveedores ni bases externas."""

import importlib.util
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import AsyncMock


def load_runner():
    path = Path(__file__).resolve().parents[1] / "tck_runner.py"
    if not path.is_file():
        raise AssertionError("Falta el runner TCK")
    spec = importlib.util.spec_from_file_location("tck_runner", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReportTests(unittest.TestCase):
    def test_cli_exposes_both_profiles_without_connecting_to_a_database(self):
        script = Path(__file__).resolve().parents[1] / "tck_runner.py"
        result = subprocess.run([sys.executable, str(script), "--help"], text=True,
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0)
        self.assertIn("native-delete", result.stdout)

    def test_skips_and_empty_runs_cannot_be_reported_as_conformance(self):
        runner = load_runner()
        result = runner.assess_report({"summary": {"passed": 2, "skipped": 1, "total": 3}}, 0)
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(runner.assess_report({"summary": {"total": 0}}, 0)["verdict"], "BLOCKED")
        self.assertEqual(runner.assess_report({"summary": {"failed": 1, "total": 1}}, 1)["verdict"], "FAIL")
        # Sin evidencia de colección no se puede afirmar cobertura completa.
        self.assertEqual(runner.assess_report({"summary": {"passed": 3, "total": 3}}, 0)["verdict"], "BLOCKED")
        self.assertEqual(runner.assess_report({"summary": {"passed": 3, "total": 3}}, 3)["verdict"], "BLOCKED")

    def test_partial_selection_and_inconsistent_reports_cannot_pass(self):
        runner = load_runner()
        partial = {"summary": {"passed": 1, "total": 1, "collected": 189, "deselected": 188},
                   "tests": [{"nodeid": "only-one", "outcome": "passed"}]}
        self.assertEqual(runner.assess_report(partial, 0)["verdict"], "BLOCKED")

    def test_complete_report_requires_exact_expected_collection_and_outcomes(self):
        runner = load_runner()
        report = {"summary": {"passed": 2, "total": 2, "collected": 2},
                  "tests": [{"nodeid": name, "outcome": "passed"} for name in ("a", "b")]}
        expected = {"all": {"a", "b"}, "selected": {"a", "b"}}
        self.assertEqual(runner.assess_report(report, 0, expected, {"a", "b"})["verdict"], "PASS")
        self.assertEqual(runner.assess_report(report, 0, expected, {"a"})["verdict"], "BLOCKED")
        report["tests"][1]["outcome"] = "skipped"
        self.assertEqual(runner.assess_report(report, 0, expected, {"a", "b"})["verdict"], "BLOCKED")

    def test_report_preserves_skip_reason_and_selected_counts(self):
        runner = load_runner()
        result = runner.assess_report({"summary": {"passed": 4, "total": 4,
                                                  "collected": 189, "deselected": 185}}, 0)
        self.assertEqual(result["counts"]["collected"], 189)
        self.assertEqual(result["counts"]["deselected"], 185)
        diagnostic = runner.phase_diagnostic({"outcome": "skipped",
            "longrepr": "('/tmp/run/suite/test.py', 12, 'Skipped: not implemented')"}, "/tmp/run")
        self.assertIn("Skipped: not implemented", diagnostic["longrepr"])
        self.assertNotIn("/tmp/run", diagnostic["longrepr"])

    def test_official_tests_are_copied_byte_for_byte_with_local_adapter_fixture(self):
        runner = load_runner()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source/tck/tests"
            (source / "v1").mkdir(parents=True)
            test = b'def test_message():\n    """SPEC-2.1.1"""\n    assert True\n'
            (source / "v1/test_short_term.py").write_bytes(test)
            (source / "conftest.py").write_bytes(b"# shared upstream fixtures\n")
            (source / "v1/conftest.py").write_bytes(b"raise RuntimeError('original fixture')\n")
            destination = root / "suite"
            files = runner.prepare_suite(root / "source", destination)
            self.assertEqual((destination / "v1/test_short_term.py").read_bytes(), test)
            self.assertEqual((destination / "conftest.py").read_bytes(), b"# shared upstream fixtures\n")
            self.assertIn("tck_fixture", (destination / "v1/conftest.py").read_text())
            self.assertIn("tck/tests/v1/test_short_term.py", files)
            self.assertEqual(runner.expected_collection(destination, "adapted"), {
                "all": {"v1/test_short_term.py::test_message"},
                "selected": {"v1/test_short_term.py::test_message"},
            })


class SafetyTests(unittest.IsolatedAsyncioTestCase):
    async def test_existing_data_cannot_be_claimed_as_synthetic(self):
        runner = load_runner()
        result = AsyncMock()
        result.single.return_value = {"count": 1}
        transaction = AsyncMock()
        transaction.run.return_value = result
        with self.assertRaisesRegex(ValueError, "vacía"):
            await runner.claim_transaction(transaction, "synthetic-token")
        self.assertEqual(transaction.run.await_count, 1)

    async def test_reset_requires_guard_and_never_falls_back_to_unscoped_deletion(self):
        runner = load_runner()
        read = AsyncMock(return_value=[{"count": 0}])
        write = AsyncMock()
        with self.assertRaisesRegex(ValueError, "marca"):
            await runner.guarded_reset(read, write, "synthetic-token")
        write.assert_not_awaited()
        read.return_value = [{"count": 1, "owned": 1}]
        write.return_value = [{"deleted": 3}]
        await runner.guarded_reset(read, write, "synthetic-token")
        self.assertEqual(write.await_args.args[1], {"token": "synthetic-token"})
        write.return_value = []
        with self.assertRaisesRegex(ValueError, "marca"):
            await runner.guarded_reset(read, write, "synthetic-token")
        read.return_value = [{"count": 2, "owned": 1}]
        write.reset_mock()
        with self.assertRaisesRegex(ValueError, "marca"):
            await runner.guarded_reset(read, write, "synthetic-token")
        write.assert_not_awaited()


if __name__ == "__main__":
    unittest.main()
