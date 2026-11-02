"""La selección del puente no puede convertir cobertura parcial en conformidad."""

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest


def runner():
    base = Path(__file__).resolve().parents[1]
    sys.path.insert(0, str(base))
    try:
        spec = importlib.util.spec_from_file_location("rust_tck_selection", base / "rust_tck_spike.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.pop(0)


class SelectionTests(unittest.TestCase):
    def test_chains_preserves_sessions_and_requires_both_new_groups(self):
        module = runner()
        groups = ("TestGetConversation", "TestListSessions", "TestClearSession", "TestDeleteMessage",
                  "TestMessageChainStructure", "TestIdempotency")
        ids = {module.TARGET} | {f"v1/test_short_term.py::{g}::test_case" for g in groups}
        selected, _ = module.select_cases(ids | {"v1/other.py::TestIdempotency::test_wrong"}, "chains")
        self.assertEqual(selected, ids)
        for group in ("TestMessageChainStructure", "TestIdempotency"):
            with self.assertRaises(ValueError):
                module.select_cases(ids - {f"v1/test_short_term.py::{group}::test_case"}, "chains")

    def test_sessions_includes_dependencies_and_previous_message_groups(self):
        module = runner()
        groups = ("TestGetConversation", "TestListSessions", "TestClearSession", "TestDeleteMessage")
        ids = {module.TARGET} | {f"v1/test_short_term.py::{g}::test_case" for g in groups}
        selected, _ = module.select_cases(ids | {"v1/test_schema.py::TestDeleteMessage::test_other"}, "sessions")
        self.assertEqual(selected, ids)
        with self.assertRaises(ValueError):
            module.select_cases(ids - {"v1/test_short_term.py::TestDeleteMessage::test_case"}, "sessions")

    def test_bronze_inventory_inherits_class_marker_and_matches_exact_ids(self):
        module = runner()
        from unittest.mock import Mock
        with tempfile.TemporaryDirectory() as temporary:
            suite = Path(temporary)
            tests = suite / "v1"
            tests.mkdir()
            (tests / "test_schema.py").write_text("# selectors use collected marker metadata\n", encoding="utf-8")
            included = Mock(nodeid="v1/test_schema.py::TestIncluded::test_class_marked")
            included.get_closest_marker.return_value = object()
            unmarked = Mock(nodeid="v1/test_schema.py::TestOther::test_unmarked")
            unmarked.get_closest_marker.return_value = None
            self.assertEqual(module.bronze_ids_from_suite(suite, [included, unmarked]),
                             {included.nodeid})

    def test_bronze_selects_exact_marker_inventory_and_requires_93_cases(self):
        module = runner()
        bronze = {module.TARGET} | {f"v1/test_{i}.py::TestBronze::test_{i}" for i in range(92)}
        non_bronze = {"v1/test_schema.py::TestSilver::test_entity_lookup"}
        selected, expression = module.select_cases(bronze | non_bronze, "bronze", bronze_ids=bronze)
        self.assertEqual(selected, bronze)
        self.assertEqual(expression, "bronze")
        with self.assertRaises(ValueError):
            module.select_cases((bronze - {next(iter(bronze))}) | non_bronze, "bronze", bronze_ids=bronze)
        with self.assertRaises(ValueError):
            module.select_cases(bronze | non_bronze, "bronze", bronze_ids=bronze - {next(iter(bronze))})
        with self.assertRaises(ValueError):
            module.select_cases(bronze - {next(iter(bronze))}, "bronze", bronze_ids=bronze)

    def test_runner_requires_digest_pinned_image_references(self):
        module = runner()
        self.assertTrue(module.IMAGE_REF.fullmatch(
            "muninn-bridge@sha256:" + "a" * 64))
        self.assertIsNone(module.IMAGE_REF.fullmatch("muninn-bridge:latest"))
        digest = "a" * 64
        self.assertEqual(module.validate_image_receipt(
            "muninn-bridge@sha256:" + digest, "sha256:" + digest),
            ("muninn-bridge@sha256:" + digest, "sha256:" + digest))
        with self.assertRaises(ValueError):
            module.validate_image_receipt("muninn-bridge@sha256:" + digest, "sha256:" + "b" * 64)

    def test_smoke_preserves_exact_original_case(self):
        module = runner()
        expected, expression = module.select_cases({module.TARGET, "v1/other.py::TestAddMessage::test_wrong"}, "smoke")
        self.assertEqual(expected, {module.TARGET})
        self.assertIn("test_add_message_creates_conversation_on_first_call", expression)

    def test_message_slice_is_bounded_to_exact_file_and_classes(self):
        module = runner()
        ids = {module.TARGET, "v1/test_short_term.py::TestGetConversation::test_read",
               "v1/test_short_term.py::TestDeleteMessage::test_delete",
               "v1/other.py::TestAddMessage::test_wrong"}
        selected, _ = module.select_cases(ids, "messages")
        self.assertEqual(selected, {module.TARGET, "v1/test_short_term.py::TestGetConversation::test_read"})

    def test_collection_evidence_records_preselection_ids(self):
        from unittest.mock import Mock

        base = Path(__file__).resolve().parents[1]
        sys.path.insert(0, str(base))
        try:
            from tck_runner import CollectionEvidence
            evidence = CollectionEvidence("pinned", Path("/synthetic/suite"))
            items = [Mock(nodeid="v1/test_sample.py::test_keep"),
                     Mock(nodeid="v1/test_sample.py::test_drop")]
            evidence.capture_before_deselection(items)
            self.assertEqual(evidence.all_items, {item.nodeid for item in items})
            self.assertEqual(evidence.collected_items, items)
        finally:
            sys.path.pop(0)

    def test_schema_selection_uses_exact_collected_nodeids(self):
        module = runner()
        suite = Path(__file__).resolve().parents[1] / ".cache/sources/agent-memory-tck/tck/tests"
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        try:
            from tck_runner import expected_collection
            all_ids = expected_collection(suite, "adapted")["all"]
        finally:
            sys.path.pop(0)
        expected = module.schema_ids_from_suite(suite)
        selected, _ = module.select_cases(all_ids, "schema", suite)
        self.assertEqual(selected, expected)

    def test_schema_profile_fails_closed_when_collection_is_incomplete(self):
        module = runner()
        suite = Path(__file__).resolve().parents[1] / ".cache/sources/agent-memory-tck/tck/tests"
        expected = module.schema_ids_from_suite(suite)
        with self.assertRaises(ValueError):
            module.select_cases(expected - {next(iter(expected))} | {module.TARGET}, "schema", suite)

    def test_missing_group_or_unknown_profile_fails_closed(self):
        module = runner()
        with self.assertRaises(ValueError):
            module.select_cases({module.TARGET}, "chains")
        with self.assertRaises(ValueError):
            module.select_cases({module.TARGET}, "unknown")

    def test_counter_or_id_drift_blocks_an_otherwise_green_report(self):
        module = runner()
        import sys
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        try:
            from tck_runner import assess_report
        finally:
            sys.path.pop(0)
        expected = {"all": {module.TARGET}, "selected": {module.TARGET}}
        report = {"summary": {"passed": 1, "failed": 0, "error": 0, "skipped": 0, "xfailed": 0,
                               "xpassed": 0, "total": 1, "collected": 1, "deselected": 0},
                  "tests": [{"nodeid": module.TARGET, "outcome": "passed"}]}
        self.assertEqual(assess_report(report, 0, expected, {module.TARGET})["verdict"], "PASS")
        report["summary"]["total"] = 2
        self.assertEqual(assess_report(report, 0, expected, {module.TARGET})["verdict"], "BLOCKED")
