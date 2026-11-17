"""Regresiones del runner TCK sin proveedores ni bases externas."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import AsyncMock


ROOT = Path(__file__).resolve().parents[1]


def load_runner():
    path = ROOT / "tck_runner.py"
    if not path.is_file():
        raise AssertionError("Falta el runner TCK")
    spec = importlib.util.spec_from_file_location("tck_runner", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def adaptation(base: bytes, old: bytes, new: bytes, target="tck/reference/adapter.py"):
    return {
        "id": "test.relationship-properties-to-attributes.v1",
        "base_commit": "pinned-commit",
        "target": target,
        "upstream_sha256": hashlib.sha256(base).hexdigest(),
        "patch": {"replacements": [{"old": old.decode(), "new": new.decode()}]},
        "patch_sha256": "test-patch-hash",
        "scope": "adapted oracle only; not Muninn conformance",
    }


class ReportTests(unittest.TestCase):
    def test_cli_exposes_both_profiles_without_connecting_to_a_database(self):
        script = ROOT / "tck_runner.py"
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


class AdapterAdaptationTests(unittest.TestCase):
    def setUp(self):
        self.runner = load_runner()
        self.base = (
            b"async def add_relationship(self, properties=None):\n"
            b"    rel = await self.client.add_relationship(\n"
            b"        source_id, target_id, relationship_type,\n"
            b"    )\n"
            b"    return TCKRelationship(properties=rel.properties or {})\n"
        )
        self.old = (
            "        source_id, target_id, relationship_type,\n"
            "    )\n"
            "    return TCKRelationship(properties=rel.properties or {})"
        )
        self.new = (
            "        source_id, target_id, relationship_type, attributes=properties,\n"
            "    )\n"
            "    return TCKRelationship(properties=rel.attributes or {})"
        )
        self.spec = adaptation(self.base, self.old.encode(), self.new.encode())

    def test_loads_checked_in_adaptation_against_integrity_pin(self):
        runner = self.runner
        integrity = json.loads((ROOT / "source-integrity.json").read_text(encoding="utf-8"))
        adaptation_spec = runner.load_reference_adaptation(ROOT / "adaptations", integrity)
        self.assertEqual(adaptation_spec["base_commit"], integrity["agent-memory-tck"]["commit"])
        self.assertEqual(adaptation_spec["target"], "tck/reference/adapter.py")
        self.assertEqual(adaptation_spec["patch_sha256"], runner.ADAPTATION_PATCH_SHA256)

    def test_rejects_manifest_with_unexpected_commit(self):
        runner = self.runner
        integrity = json.loads((ROOT / "source-integrity.json").read_text(encoding="utf-8"))
        integrity["agent-memory-tck"]["commit"] = "unexpected"
        with self.assertRaisesRegex(ValueError, "commit base"):
            runner.load_reference_adaptation(ROOT / "adaptations", integrity)

    def test_checked_in_patch_translates_relationship_properties_both_directions(self):
        integrity = json.loads((ROOT / "source-integrity.json").read_text(encoding="utf-8"))
        adaptation_spec = self.runner.load_reference_adaptation(ROOT / "adaptations", integrity)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / adaptation_spec["target"]
            target.parent.mkdir(parents=True)
            upstream = (
                b"async def add_relationship(self, source_id, target_id, relationship_type, *, properties=None):\n"
                b"    rel = await self._client.long_term.add_relationship(\n"
                b"        source_id,\n"
                b"        target_id,\n"
                b"            relationship_type,\n"
                b"        )\n"
                b"        return TCKRelationship(\n"
                b"            id=rel.id,\n"
                b"            source_id=rel.source_id,\n"
                b"            target_id=rel.target_id,\n"
                b"            relationship_type=rel.type,\n"
                b"            properties=rel.properties or {},\n"
                b"        )\n"
            )
            target.write_bytes(upstream)
            fixture_adaptation = dict(adaptation_spec)
            fixture_adaptation["upstream_sha256"] = hashlib.sha256(upstream).hexdigest()

            receipt = self.runner.apply_reference_adaptation(root, fixture_adaptation)

            patched = target.read_text(encoding="utf-8")
            self.assertIn("attributes=properties", patched)
            self.assertIn("properties=rel.attributes or {}", patched)
            self.assertEqual(receipt["upstream_sha256"], fixture_adaptation["upstream_sha256"])
            self.assertEqual(receipt["patch_sha256"], adaptation_spec["patch_sha256"])
            self.assertEqual(receipt["adapted_sha256"], hashlib.sha256(target.read_bytes()).hexdigest())

    def test_applies_only_to_pinned_target_and_records_separate_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / self.spec["target"]
            target.parent.mkdir(parents=True)
            target.write_bytes(self.base)

            receipt = self.runner.apply_reference_adaptation(root, self.spec)

            adapted = target.read_bytes()
            self.assertIn(b"attributes=properties", adapted)
            self.assertIn(b"properties=rel.attributes", adapted)
            self.assertEqual(receipt["upstream_sha256"], hashlib.sha256(self.base).hexdigest())
            self.assertEqual(receipt["adapted_sha256"], hashlib.sha256(adapted).hexdigest())
            self.assertNotEqual(receipt["upstream_sha256"], receipt["adapted_sha256"])
            self.assertIn("no es Muninn", receipt["scope"])

    def test_rejects_unexpected_upstream_bytes_without_mutating_target(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / self.spec["target"]
            target.parent.mkdir(parents=True)
            changed = self.base + b"# changed upstream\n"
            target.write_bytes(changed)

            with self.assertRaisesRegex(ValueError, "hash"):
                self.runner.apply_reference_adaptation(root, self.spec)

            self.assertEqual(target.read_bytes(), changed)

    def test_rejects_non_unique_patch_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / self.spec["target"]
            target.parent.mkdir(parents=True)
            target.write_bytes(self.base + self.base)
            self.spec["upstream_sha256"] = hashlib.sha256(target.read_bytes()).hexdigest()

            with self.assertRaisesRegex(ValueError, "exactamente una vez"):
                self.runner.apply_reference_adaptation(root, self.spec)

            self.assertEqual(target.read_bytes(), self.base + self.base)

    def test_rejects_target_escape(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.spec["target"] = "../outside.py"
            with self.assertRaisesRegex(ValueError, "Ruta"):
                self.runner.apply_reference_adaptation(root, self.spec)

    def test_rejects_missing_old_context(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / self.spec["target"]
            target.parent.mkdir(parents=True)
            target.write_bytes(self.base.replace(b"rel.properties", b"rel.unknown"))
            self.spec["upstream_sha256"] = hashlib.sha256(target.read_bytes()).hexdigest()

            with self.assertRaisesRegex(ValueError, "contexto"):
                self.runner.apply_reference_adaptation(root, self.spec)

            self.assertEqual(target.read_bytes(), self.base.replace(b"rel.properties", b"rel.unknown"))


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
