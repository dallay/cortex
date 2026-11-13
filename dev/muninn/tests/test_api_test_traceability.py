"""Valida trazabilidad API curada; no ejecuta TCK ni código upstream."""
import copy
import json
import subprocess
import sys
import unittest
from pathlib import Path

import api_test_traceability as traceability

ROOT = Path(__file__).resolve().parents[3]
CHECKER = ROOT / "dev/muninn/api_test_traceability.py"
REPORT = ROOT / "docs/muninn/specs/api-test-traceability.json"


class ApiTraceabilityTests(unittest.TestCase):
    def test_inventory_union_counts_ids_not_links_or_dispositions(self):
        inventory = {"SPEC-A", "api:A", "api:B", "api:C", "api:D", "type:A"}
        spec = [{"id": "SPEC-A", "tests": [{"nodeid": "test-a"}]}]
        entries = [
            {"inventory_id": "api:A", "kind": "mapped_reference_test"},
            {"inventory_id": "api:A", "kind": "mapped_reference_test"},
            {"inventory_id": "api:B", "kind": "failed_reference_test"},
            {"inventory_id": "api:C", "kind": "planned_test"},
            {"inventory_id": "api:D", "kind": "rejected_association"},
        ]
        summary = traceability.inventory_union_summary(inventory, spec, entries)
        self.assertEqual(summary, {
            "inventory_rows": 6,
            "rows_with_test_links": 2,
            "rows_with_failure_diagnostics": 1,
            "rows_with_planned_tests": 1,
            "rows_with_rejected_associations": 1,
            "rows_without_test_links": 4,
            "rows_without_recorded_handling": 1,
        })

    def test_inventory_union_deduplicates_overlapping_kinds(self):
        summary = traceability.inventory_union_summary(
            {"api:A", "api:B"}, [], [
                {"inventory_id": "api:A", "kind": kind}
                for kind in ("mapped_reference_test", "planned_test",
                             "failed_reference_test", "rejected_association")
            ],
        )
        self.assertEqual(summary["rows_with_test_links"], 1)
        self.assertEqual(summary["rows_without_test_links"], 1)
        self.assertEqual(summary["rows_without_recorded_handling"], 1)

    def test_inventory_union_rejects_unknown_ids(self):
        with self.assertRaisesRegex(traceability.TraceabilityError, "inventario"):
            traceability.inventory_union_summary(
                {"SPEC-A"}, [],
                [{"inventory_id": "api:unknown", "kind": "planned_test"}],
            )

    def test_product_profile_rows_do_not_change_reference_baseline_outcomes(self):
        candidate = "v1/test_short_term.py::TestDeleteMessage::test_delete_message_returns_true"
        rows = [
            {"profile": "adapted", "test_id": candidate, "outcome": "passed", "gap_id": ""},
            {"profile": "native-delete", "test_id": candidate, "outcome": "failed", "gap_id": "REF-001"},
            {"profile": "product-b6c7-chains", "test_id": candidate,
             "outcome": "passed", "gap_id": ""},
            {"profile": "product-b6c7-bronze", "test_id": candidate,
             "outcome": "passed", "gap_id": ""},
        ]
        self.assertEqual(
            traceability.reference_baseline_outcomes(rows, candidate),
            {("adapted", "passed", ""), ("native-delete", "failed", "REF-001")},
        )

    def test_curated_sidecar_passes_offline_consistency_check(self):
        result = subprocess.run(
            [sys.executable, str(CHECKER), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"mapped": 39', result.stdout)
        self.assertIn('"reference_failed": 6', result.stdout)
        self.assertIn('"mapped_operations": 20', result.stdout)
        self.assertIn('"planned": 3', result.stdout)
        self.assertIn('"rejected": 1', result.stdout)

    def test_rejected_adapter_outcomes_must_match_the_recorded_baseline(self):
        report = json.loads(REPORT.read_text(encoding="utf-8"))
        corrupted = copy.deepcopy(report)
        rejected = next(entry for entry in corrupted["entries"]
                        if entry["kind"] == "rejected_association")
        rejected["baseline_results"][0]["outcome"] = "failed"
        with self.assertRaisesRegex(traceability.TraceabilityError, "outcome"):
            traceability.validate(corrupted)

    def test_add_entity_uuid_claim_links_the_uuid_assertion(self):
        report = json.loads(REPORT.read_text(encoding="utf-8"))
        entity_id = "api:src/neo4j_agent_memory/memory/long_term.py:LongTermMemory.add_entity"
        uuid_links = [entry for entry in report["entries"]
                      if entry["kind"] == "mapped_reference_test"
                      and entry["inventory_id"] == entity_id
                      and entry["test"]["spec_id"] == "SPEC-3.1.12"]
        self.assertEqual(len(uuid_links), 1)
        self.assertEqual(uuid_links[0]["test"]["function"], "test_add_entity_id_is_uuid")
        self.assertIn("UUID", uuid_links[0]["test"]["assertion_scope"])

    def test_sidecar_includes_eight_distinct_api_option_cases(self):
        report = json.loads(REPORT.read_text(encoding="utf-8"))
        expected = {
            ("SPEC-2.1.5", "test_add_message_with_metadata"),
            ("SPEC-2.2.2", "test_get_conversation_with_limit"),
            ("SPEC-2.6.3", "test_clear_session_idempotent_on_empty"),
            ("SPEC-3.2.2", "test_add_preference_with_context"),
            ("SPEC-3.3.3", "test_add_fact_special_characters"),
            ("SPEC-4.2.3", "test_add_step_partial_fields"),
            ("SPEC-4.3.3", "test_record_tool_call_failure_status"),
            ("SPEC-4.6.3", "test_list_traces_respects_limit"),
        }
        actual = {
            (entry["test"]["spec_id"], entry["test"]["function"])
            for entry in report["entries"]
            if entry["kind"] == "mapped_reference_test"
        }
        self.assertTrue(expected.issubset(actual), expected - actual)

    def test_sidecar_maps_remaining_passed_cases_and_separates_adapter_failures(self):
        report = json.loads(REPORT.read_text(encoding="utf-8"))
        expected_positive = {
            "test_search_preferences_finds_relevant",
            "test_search_preferences_by_category",
            "test_search_preferences_empty_database",
            "test_search_preferences_no_match",
            "test_get_entity_by_name_found",
            "test_get_entity_by_name_not_found",
            "test_entity_enriched_across_sessions",
            "test_entity_created_by_one_session_visible_to_another",
            "test_conversations_isolated_but_entities_shared",
            "test_get_related_entities_empty",
            "test_get_tool_stats_after_calls",
            "test_get_tool_stats_multiple_tools",
            "test_get_tool_stats_no_calls",
            "test_get_tool_stats_success_rate",
            "test_merge_duplicate_entities",
        }
        mapped = {entry["test"]["function"] for entry in report["entries"]
                  if entry["kind"] == "mapped_reference_test"}
        self.assertTrue(expected_positive.issubset(mapped), expected_positive - mapped)

        expected_failures = {
            "test_end_to_end_conversation_with_entities_and_reasoning",
            "test_add_relationship_between_entities",
            "test_add_relationship_bidirectional_traversal",
            "test_add_relationship_multiple_types",
            "test_add_relationship_returns_valid_id",
            "test_merge_preserves_relationships",
        }
        failed = {entry["test"]["function"] for entry in report["entries"]
                  if entry["kind"] == "failed_reference_test"}
        self.assertEqual(failed, expected_failures)

    def test_failed_reference_diagnostic_must_match_ref002(self):
        report = json.loads(REPORT.read_text(encoding="utf-8"))
        corrupted = copy.deepcopy(report)
        failed = next(entry for entry in corrupted["entries"]
                      if entry["kind"] == "failed_reference_test")
        failed["failure_diagnostic"]["stage"] = "API operation returned an error"
        with self.assertRaisesRegex(traceability.TraceabilityError, "Diagnóstico"):
            traceability.validate(corrupted)


if __name__ == "__main__":
    unittest.main()
