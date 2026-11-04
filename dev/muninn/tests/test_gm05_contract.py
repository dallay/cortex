"""El contrato GM-05 no puede convertir una brecha en una comparación verde."""

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest


BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))


def module(name):
    spec = importlib.util.spec_from_file_location(name + "_under_test", BASE / (name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


def preference(identifier, **overrides):
    value = {
        "id": identifier,
        "category": "food",
        "preference": "le gusta el cafe",
        "context": "manana",
        "confidence": 0.8,
        "metadata": {},
        "linked_entities": [],
        "valid_from": "2026-01-01T00:00:00+00:00",
        "valid_until": None,
    }
    value.update(overrides)
    return value


def fact(identifier, **overrides):
    value = {
        "id": identifier,
        "subject": "ana",
        "predicate": "vive_en",
        "object": "Madrid",
        "confidence": 0.9,
        "valid_from": "2026-02-01T00:00:00+00:00",
        "valid_until": None,
        "metadata": {},
    }
    value.update(overrides)
    return value


def relationship(identifier, source, target, **overrides):
    value = {
        "id": identifier,
        "source_id": source,
        "target_id": target,
        "type": "WORKS_AT",
        "description": "trabaja en",
        "confidence": 0.75,
        "valid_from": "2026-03-01T00:00:00+00:00",
        "valid_until": None,
        "attributes": {"role": "cto"},
    }
    value.update(overrides)
    return value


def entity(identifier, **overrides):
    value = {"id": identifier, "name": "ACME", "type": "ORGANIZATION"}
    value.update(overrides)
    return value


def graph_nodes(first="e-1", second="e-2"):
    """Nodos de entidad del escenario: la identidad no depende del identificador."""
    return [{"key": first, "labels": ["Entity"],
             "properties": {"id": first, "name": "ana", "type": "PERSON"}},
            {"key": second, "labels": ["Entity"],
             "properties": {"id": second, "name": "ACME", "type": "ORGANIZATION"}}]


SCENARIO_ATTRIBUTES = module("gm05_capture").RELATIONSHIP["attributes"]


def document(**overrides):
    value = {
        "preferences": [preference("p-1")],
        "preferences_for": [preference("p-1")],
        "facts": [fact("f-1")],
        "facts_about": [fact("f-1")],
        "relationships": [relationship("r-1", "e-1", "e-2")],
        "related": [entity("e-2")],
        "deduplication": {
            "preference_same_id": True,
            "preference_flag": True,
            "fact_same_id": True,
            "fact_flag": True,
        },
        "supersession": {
            "edge": True,
            "old_valid_until_set": True,
            "repeat_is_noop": True,
            "active_only": ["p-1"],
            "history": ["p-1", "p-2"],
        },
        "graph": {"nodes": graph_nodes(), "edges": []},
        "gaps": [],
    }
    value.update(overrides)
    return value


class ComparisonTests(unittest.TestCase):
    def setUp(self):
        self.contract = module("gm05_contract")

    def verdict(self, reference, muninn, **options):
        return self.contract.compare_gm05(reference, muninn, **options)

    def test_identical_documents_pass(self):
        result = self.verdict(document(), document())
        self.assertEqual(result["comparison"], "pass")
        self.assertEqual(result["errors"], [])

    def test_generated_ids_map_bijectively_between_sides(self):
        reference = document()
        muninn = document(
            preferences=[preference("0f0f0f0f-0000-4000-8000-000000000001")],
            preferences_for=[preference("0f0f0f0f-0000-4000-8000-000000000001")],
            facts=[fact("0f0f0f0f-0000-4000-8000-000000000002")],
            facts_about=[fact("0f0f0f0f-0000-4000-8000-000000000002")],
            relationships=[relationship("0f0f0f0f-0000-4000-8000-000000000003",
                                        "0f0f0f0f-0000-4000-8000-000000000004",
                                        "0f0f0f0f-0000-4000-8000-000000000005")],
            related=[entity("0f0f0f0f-0000-4000-8000-000000000005")],
            graph={"nodes": graph_nodes("0f0f0f0f-0000-4000-8000-000000000004",
                                       "0f0f0f0f-0000-4000-8000-000000000005"), "edges": []},
            supersession={"edge": True, "old_valid_until_set": True, "repeat_is_noop": True,
                          "active_only": ["0f0f0f0f-0000-4000-8000-000000000001"],
                          "history": ["0f0f0f0f-0000-4000-8000-000000000001", "p-2"]},
        )
        self.assertEqual(self.verdict(reference, muninn)["comparison"], "pass")

    def test_generated_ids_must_be_unique_within_a_section(self):
        broken = document(preferences=[preference("p-1"), preference("p-1", preference="otra")])
        result = self.verdict(document(), broken)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("duplicate" in error for error in result["errors"]))

    def test_nanosecond_timestamps_normalize_like_microsecond_ones(self):
        fast = document(graph={"nodes": [{"key": "e-1", "labels": ["Entity"], "properties": {
            "id": "e-1", "created_at": "2026-01-01T00:00:00.123456789+00:00"}}], "edges": []})
        slow = document(graph={"nodes": [{"key": "e-1", "labels": ["Entity"], "properties": {
            "id": "e-1", "created_at": "2026-01-01T00:00:00.123456+00:00"}}], "edges": []})
        self.assertEqual(self.verdict(fast, slow)["comparison"], "pass")

    def test_generated_timestamps_require_a_timezone(self):
        broken = document(facts=[fact("f-1", valid_from="2026-02-01T00:00:00")])
        result = self.verdict(document(), broken)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("valid_from" in error for error in result["errors"]))

    def test_validity_interval_must_be_ordered(self):
        broken = document(facts=[fact("f-1", valid_from="2026-02-02T00:00:00+00:00",
                                      valid_until="2026-02-01T00:00:00+00:00")])
        result = self.verdict(document(), broken)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("interval" in error for error in result["errors"]))

    def test_supplied_validity_bounds_compare_exactly(self):
        drifted = document(facts=[fact("f-1", valid_from="2026-02-01T00:00:01+00:00")])
        result = self.verdict(document(), drifted)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("facts[0].valid_from" in error for error in result["errors"]))

    def test_missing_field_reports_the_dotted_path(self):
        absent = preference("p-1")
        del absent["context"]
        result = self.verdict(document(), document(preferences=[absent]))
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("preferences[0].context" in error for error in result["errors"]))

    def test_non_comparable_fields_never_block(self):
        decorated_preference = {**preference("p-1"),
                                "created_at": "2026-05-05T10:00:00+00:00",
                                "linked_entities": [],
                                "valid_from": "2026-05-05T10:00:01+00:00",
                                "valid_until": "2026-05-05T10:00:02+00:00"}
        bare_preference = {field: value for field, value in preference("p-1").items()
                           if field not in ("created_at", "linked_entities", "valid_from",
                                            "valid_until")}
        decorated_fact = {**fact("f-1"), "created_at": "2026-05-05T10:00:00+00:00"}
        bare_fact = {field: value for field, value in fact("f-1").items() if field != "created_at"}
        decorated_relationship = {**relationship("r-1", "e-1", "e-2"),
                                  "created_at": "2026-05-05T10:00:00+00:00"}
        bare_relationship = {field: value for field, value in
                             relationship("r-1", "e-1", "e-2").items() if field != "created_at"}
        decorated_entity = {**entity("e-2"), "created_at": "2026-05-05T10:00:00+00:00",
                            "updated_at": None, "embedding": None, "subtype": "ORGANIZATION",
                            "canonical_name": "ACME", "confidence": 1.0, "metadata": {}}
        bare_entity = {"id": "e-2", "name": "ACME", "type": "ORGANIZATION"}
        reference = document(preferences=[bare_preference],
                             preferences_for=[bare_preference],
                             facts=[bare_fact], facts_about=[bare_fact],
                             relationships=[bare_relationship], related=[bare_entity])
        muninn = document(preferences=[decorated_preference],
                          preferences_for=[decorated_preference],
                          facts=[decorated_fact], facts_about=[decorated_fact],
                          relationships=[decorated_relationship], related=[decorated_entity])
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "pass", result["errors"])

    def test_graph_ignores_declared_non_comparable_properties(self):
        nodes = [{"key": "e-1", "labels": ["Entity"],
                  "properties": {"id": "e-1", "name": "ana", "type": "PERSON"}},
                 {"key": "e-2", "labels": ["Entity"],
                  "properties": {"id": "e-2", "name": "ACME", "type": "ORGANIZATION"}}]
        decorated = [
            {"key": "e-1",
             "labels": ["Entity", "PERSON"],
             "properties": {"id": "e-1", "name": "ana", "type": "PERSON", "subtype": "ALIAS",
                            "canonical_name": "ana", "confidence": 1.0, "aliases": [],
                            "name_key": "ana", "surface_keys": "|ana|", "metadata": {},
                            "embedding": [0.1, 0.2], "updated_at": "2026-05-05T10:00:00+00:00"}},
            {**nodes[1], "labels": ["Entity", "ORGANIZATION"],
             "properties": {**nodes[1]["properties"], "name_key": "acme"}},
        ]
        edges = [{"source": "e-1", "target": "e-2", "type": "RELATED_TO",
                  "properties": {"id": "r-1", "relation_type": "WORKS_AT", "support": 1,
                                 "derived": False, "extractor": "fixture",
                                 "source_message_ids": [], "evidence": []}}]
        decorated_edges = [{**edges[0], "properties": {**edges[0]["properties"], "support": 2,
                                                       "extractor": "other"}}]
        reference = document(graph={"nodes": nodes, "edges": edges})
        muninn = document(graph={"nodes": decorated, "edges": decorated_edges})
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "pass", result["errors"])

    def test_graph_unknown_property_blocks_instead_of_being_dropped(self):
        node = {"key": "e-1", "labels": ["Entity"],
                "properties": {"id": "e-1", "name": "ana", "surprise": 1}}
        result = self.verdict(document(), document(graph={"nodes": [node], "edges": []}))
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("surprise" in error for error in result["errors"]))

    def test_an_absent_operation_blocks_even_when_both_sides_are_empty(self):
        result = self.verdict(
            document(gaps=["get_preferences_for:contract_absent"], preferences_for=[]),
            document(gaps=["get_preferences_for:contract_absent"], preferences_for=[]))
        self.assertEqual(result["comparison"], "fail")
        self.assertIn("gap:get_preferences_for:contract_absent", result["errors"])

    def test_a_gap_on_either_side_blocks_the_comparison(self):
        result = self.verdict(document(gaps=["supersede_preference:contract_absent"]), document())
        self.assertEqual(result["comparison"], "fail")
        self.assertIn("gap:supersede_preference:contract_absent", result["errors"])
        result = self.verdict(document(), document(gaps=["add_fact.valid_until:field_absent"]))
        self.assertEqual(result["comparison"], "fail")
        self.assertIn("gap:add_fact.valid_until:field_absent", result["errors"])

    def test_a_gap_does_not_hide_a_value_difference(self):
        reference = document(gaps=["get_preferences_for:contract_absent"],
                             preferences_for=[], preferences=[preference("p-1")])
        muninn = document(gaps=["get_preferences_for:contract_absent"],
                          preferences_for=[],
                          preferences=[preference("p-1", preference="otra")])
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "fail")
        self.assertIn("gap:get_preferences_for:contract_absent", result["errors"])
        self.assertTrue(any("preferences[0].preference" in error for error in result["errors"]))

    def test_preference_validity_is_generated_not_supplied(self):
        left = document(preferences=[preference("p-1", valid_from="2026-01-01T00:00:00+00:00")])
        right = document(preferences=[preference("p-1", valid_from="2026-05-05T05:05:05+00:00")])
        self.assertEqual(self.verdict(left, right)["comparison"], "pass")

    def test_graph_edge_identity_and_created_at_are_normalized(self):
        nodes = [{"key": "e-1", "labels": ["Entity"], "properties": {"id": "e-1", "name": "ana"}},
                 {"key": "e-2", "labels": ["Entity"], "properties": {"id": "e-2", "name": "ACME"}}]
        left = document(graph={"nodes": nodes, "edges": [
            {"source": "e-1", "target": "e-2", "type": "RELATED_TO",
             "properties": {"id": "r-1", "created_at": "2026-03-01T00:00:00+00:00"}}]})
        right = document(graph={"nodes": nodes, "edges": [
            {"source": "e-1", "target": "e-2", "type": "RELATED_TO",
             "properties": {"id": "r-9", "created_at": "2026-04-04T04:04:04+00:00"}}]},
            relationships=[relationship("r-9", "e-1", "e-2")])
        self.assertEqual(self.verdict(left, right)["comparison"], "pass")

    def test_result_lists_are_compared_by_identity_not_order(self):
        left = document(facts_about=[fact("f-1"), fact("f-2", object="Barcelona", confidence=0.4)])
        right = document(facts_about=[fact("f-2", object="Barcelona", confidence=0.4), fact("f-1")])
        result = self.verdict(left, right)
        self.assertEqual(result["comparison"], "pass", result["errors"])

    def test_deduplication_flags_are_compared_individually(self):
        drifted = document()
        drifted["deduplication"]["preference_flag"] = False
        result = self.verdict(document(), drifted)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("deduplication.preference_flag" in error for error in result["errors"]))

    def test_supersession_filtering_is_compared(self):
        drifted = document()
        drifted["supersession"]["active_only"] = ["p-2"]
        result = self.verdict(document(), drifted)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("supersession.active_only" in error for error in result["errors"]))

    def test_graph_comparison_ignores_order_but_not_shape(self):
        nodes = [{"key": "e-1", "labels": ["Entity"], "properties": {"id": "e-1", "name": "ana"}},
                 {"key": "e-2", "labels": ["Entity"], "properties": {"id": "e-2", "name": "ACME"}}]
        edges = [{"source": "e-1", "target": "e-2", "type": "RELATED_TO", "properties": {"type": "WORKS_AT"}}]
        reordered = document(graph={"nodes": list(reversed(nodes)), "edges": edges})
        self.assertEqual(self.verdict(document(graph={"nodes": nodes, "edges": edges}),
                                      reordered)["comparison"], "pass")
        reversed_edge = document(graph={"nodes": nodes, "edges": [
            {"source": "e-2", "target": "e-1", "type": "RELATED_TO", "properties": {"type": "WORKS_AT"}}]})
        self.assertEqual(self.verdict(document(graph={"nodes": nodes, "edges": edges}),
                                      reversed_edge)["comparison"], "fail")

    def test_confidence_drift_is_reported_without_tolerance(self):
        drifted = document(preferences=[preference("p-1", confidence=0.8000001)])
        result = self.verdict(document(), drifted)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("preferences[0].confidence" in error for error in result["errors"]))

    def test_unknown_field_blocks_instead_of_being_ignored(self):
        extra = preference("p-1")
        extra["score"] = 0.5
        result = self.verdict(document(), document(preferences=[extra]))
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("preferences[0].score" in error for error in result["errors"]))

    def test_unknown_top_level_section_is_rejected(self):
        broken = document()
        broken["traces"] = []
        result = self.verdict(document(), broken)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("traces" in error for error in result["errors"]))

    def test_identity_tokens_come_from_the_real_identifier(self):
        contract = self.contract
        def token(value):
            return f'<entity-{hashlib.sha256(value.encode("utf-8")).hexdigest()[:12]}>'
        first = contract.canonical_gm05(
            document(relationships=[relationship("r-1", "sin-fuente", "sin-destino")],
                     related=[], graph={"nodes": [], "edges": []}))
        shifted = contract.canonical_gm05(
            document(relationships=[relationship("r-0", "otra-fuente", "otro-destino"),
                                    relationship("r-1", "sin-fuente", "sin-destino")],
                     related=[], graph={"nodes": [], "edges": []}))
        self.assertEqual(first["relationships"][0]["source_id"], token("sin-fuente"))
        by_target = {item["target_id"]: item for item in shifted["relationships"]}
        self.assertEqual(by_target[token("sin-destino")]["source_id"], token("sin-fuente"))
        self.assertNotEqual(shifted["relationships"][0]["source_id"],
                            shifted["relationships"][1]["source_id"])

    def test_absent_and_empty_attributes_are_the_same_observation(self):
        without = relationship("r-1", "e-1", "e-2")
        del without["attributes"]
        empty = relationship("r-1", "e-1", "e-2", attributes={})
        self.assertEqual(self.verdict(document(relationships=[without]),
                                      document(relationships=[empty]))["comparison"], "pass")
        self.assertEqual(self.verdict(document(relationships=[empty]),
                                      document(relationships=[without]))["comparison"], "pass")

    def test_generated_marks_are_validated_against_the_window(self):
        inside = ("2025-12-01T00:00:00+00:00", "2026-12-01T00:00:00+00:00")
        self.assertEqual(self.verdict(document(), document(), window=inside)["comparison"],
                         "pass")
        outside = ("2026-06-01T00:00:00+00:00", "2026-12-01T00:00:00+00:00")
        result = self.verdict(document(), document(), window=outside)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("fuera de la ventana" in error for error in result["errors"]))

    def test_graph_generated_marks_are_validated_against_the_window(self):
        marked = graph_nodes() + [{
            "key": "p-1", "labels": ["Preference"],
            "properties": {"id": "p-1", "category": "food", "preference": "le gusta el cafe",
                           "created_at": "2026-02-01T00:00:00+00:00"}}]
        captured = document(graph={"nodes": marked, "edges": []})
        inside = ("2025-12-01T00:00:00+00:00", "2026-12-01T00:00:00+00:00")
        self.assertEqual(self.verdict(captured, captured, window=inside)["comparison"], "pass")
        outside = ("2026-03-01T00:00:00+00:00", "2026-12-01T00:00:00+00:00")
        result = self.verdict(captured, captured, window=outside)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("fuera de la ventana" in error for error in result["errors"]))

    def test_generated_marks_are_not_compared_between_servers(self):
        reference = document(
            preferences=[preference("p-1", created_at="2026-01-01T00:00:00+00:00")],
            preferences_for=[preference("p-1", created_at="2026-01-01T00:00:00+00:00")])
        muninn = document(
            preferences=[preference("p-1", created_at="2026-01-01T00:00:03+00:00")],
            preferences_for=[preference("p-1", created_at="2026-01-01T00:00:03+00:00")])
        self.assertEqual(self.verdict(reference, muninn)["comparison"], "pass")

    def test_a_malformed_generated_mark_fails(self):
        broken = document(preferences=[preference("p-1", created_at="ayer")],
                          preferences_for=[preference("p-1", created_at="ayer")])
        result = self.verdict(broken, broken)
        self.assertEqual(result["comparison"], "fail")


class NormalizationRegressionTests(unittest.TestCase):
    def test_tied_identities_use_all_comparable_values_without_losing_duplicates(self):
        from copy import deepcopy
        contract = module('gm05_contract')
        for section, factory in (
                ('preferences', lambda identifier, value: preference(identifier, context=value)),
                ('preferences_for', lambda identifier, value: preference(identifier, context=value)),
                ('facts', lambda identifier, value: fact(identifier, confidence=value)),
                ('facts_about', lambda identifier, value: fact(identifier, confidence=value)),
                ('relationships', lambda identifier, value: relationship(
                    identifier, 'e-1', 'e-2', attributes={'nested': {'value': value}})),
                ('related', lambda identifier, value: entity(identifier, description=str(value)))):
            with self.subTest(section=section):
                left = document(**{section: [factory('a', 1), factory('b', 2), factory('c', 1)]})
                right = deepcopy(left)
                right[section] = [right[section][1], right[section][2], right[section][0]]
                before = deepcopy((left, right))
                self.assertEqual(contract.compare_gm05(left, right)['comparison'], 'pass')
                self.assertEqual((left, right), before)
                right[section].pop()
                self.assertEqual(contract.compare_gm05(left, right)['comparison'], 'fail')
                right = deepcopy(left)
                right[section][-1] = factory('c', 1.0000001)
                self.assertEqual(contract.compare_gm05(left, right)['comparison'], 'fail')

    def test_tied_edges_order_by_normalized_properties_and_keep_multiplicity(self):
        from copy import deepcopy
        contract = module('gm05_contract')
        edges = [{'source': 'e-1', 'target': 'e-2', 'type': 'RELATED_TO',
                  'properties': {'confidence': value}} for value in (0.1, 0.2, 0.1)]
        left = document(graph={'nodes': graph_nodes(), 'edges': edges})
        right = deepcopy(left)
        right['graph']['edges'] = [edges[1], edges[2], edges[0]]
        self.assertEqual(contract.compare_gm05(left, right)['comparison'], 'pass')
        right['graph']['edges'] = edges[:-1]
        self.assertEqual(contract.compare_gm05(left, right)['comparison'], 'fail')
        right['graph']['edges'] = deepcopy(edges)
        right['graph']['edges'][0]['properties']['confidence'] = 0.10000001
        self.assertEqual(contract.compare_gm05(left, right)['comparison'], 'fail')

    def test_graph_type_alias_conflicts_fail_without_payload_or_mutation(self):
        from copy import deepcopy
        contract = module('gm05_contract')
        for canonical, alias in [('PRIVATE-A', 'PRIVATE-B'), (None, 'PRIVATE-B'),
                                 ('PRIVATE-A', None)]:
            captured = document(graph={'nodes': graph_nodes(), 'edges': [
                {'source': 'e-1', 'target': 'e-2', 'type': 'RELATED_TO',
                 'properties': {'relation_type': canonical, 'type': alias}}]})
            before = deepcopy(captured)
            result = contract.compare_gm05(captured, captured)
            self.assertEqual(result['comparison'], 'fail')
            self.assertIn('relation_type/type', str(result['errors']))
            self.assertNotIn('PRIVATE', str(result))
            self.assertEqual(captured, before)

    def test_graph_type_alias_equal_and_single_forms_remain_equivalent(self):
        contract = module('gm05_contract')
        def captured(properties):
            return document(graph={'nodes': graph_nodes(), 'edges': [
                {'source': 'e-1', 'target': 'e-2', 'type': 'RELATED_TO',
                 'properties': properties}]})
        for value in ('WORKS_AT', None):
            both = captured({'relation_type': value, 'type': value})
            for name in ('relation_type', 'type'):
                self.assertEqual(contract.compare_gm05(
                    both, captured({name: value}))['comparison'], 'pass')


class RobustnessTests(unittest.TestCase):
    """GM05-003 y GM05-017: el contrato nunca propaga una excepción."""

    def setUp(self):
        self.contract = module("gm05_contract")

    def test_an_unexpected_exception_becomes_a_failed_verdict(self):
        result = self.contract.compare_gm05(document(), document(), deviations=None)
        self.assertEqual(result["comparison"], "fail")
        self.assertEqual(result["deviations"], [])
        self.assertTrue(result["errors"][0].startswith("error: TypeError:"))

    def test_a_non_mapping_document_is_a_failure_not_a_crash(self):
        result = self.contract.compare_gm05(None, document())
        self.assertEqual(result["comparison"], "fail")
        self.assertFalse(result["errors"] == [])

    def test_a_malformed_window_is_a_failure_not_a_crash(self):
        result = self.contract.compare_gm05(document(), document(), window=("ayer", "hoy"))
        self.assertEqual(result["comparison"], "fail")


class DeviationTests(unittest.TestCase):
    """GM05-DEV-001: el oráculo reemite `attributes`; sólo Muninn los persiste."""

    def setUp(self):
        self.contract = module("gm05_contract")

    def documents(self):
        reference = document(relationships=[relationship("r-1", "e-1", "e-2",
                                                          attributes=dict(SCENARIO_ATTRIBUTES))],
                             related=[entity("e-2")])
        muninn = document(relationships=[relationship("r-1", "e-1", "e-2",
                                                       attributes=dict(SCENARIO_ATTRIBUTES))],
                          related=[entity("e-2")])
        blob = json.dumps(SCENARIO_ATTRIBUTES)
        nodes = graph_nodes()
        reference["graph"] = {"nodes": nodes, "edges": [
            {"source": "e-1", "target": "e-2", "type": "RELATED_TO",
             "properties": {"id": "r-1"}}]}
        muninn["graph"] = {"nodes": nodes, "edges": [
            {"source": "e-1", "target": "e-2", "type": "RELATED_TO",
             "properties": {"id": "r-1", "properties": blob}}]}
        return reference, muninn

    def verdict(self, reference, muninn):
        return self.contract.compare_gm05(reference, muninn, deviations=("GM05-DEV-001",))

    def test_without_the_deviation_the_blob_still_fails(self):
        reference, muninn = self.documents()
        result = self.contract.compare_gm05(reference, muninn)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("graph.edges[0].properties.properties" in error
                            for error in result["errors"]))

    def test_the_deviation_explains_only_the_attributes_blob(self):
        reference, muninn = self.documents()
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "pass")
        self.assertEqual(result["deviations"], ["GM05-DEV-001"])

    def test_the_deviation_does_not_mask_another_difference(self):
        reference, muninn = self.documents()
        muninn["relationships"][0]["confidence"] = 0.9
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("confidence" in error for error in result["errors"]))

    def test_the_deviation_requires_the_attributes_to_match_the_scenario(self):
        reference, muninn = self.documents()
        muninn["relationships"][0]["attributes"] = {"role": "ceo"}
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("gm05_dev_001" in error for error in result["errors"]))

    def test_converged_absence_of_blob_uses_strict_comparison(self):
        reference, muninn = self.documents()
        muninn["graph"]["edges"][0]["properties"].pop("properties")
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "pass")
        self.assertEqual(result["deviations"], [])
        self.assertEqual(result["errors"], [])

    def test_converged_persistence_uses_strict_comparison(self):
        reference, muninn = self.documents()
        reference["graph"]["edges"][0]["properties"]["properties"] = json.dumps(
            SCENARIO_ATTRIBUTES)
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "pass")
        self.assertEqual(result["deviations"], [])
        self.assertEqual(result["errors"], [])

    def test_the_deviation_requires_the_blob_to_match_the_declared_attributes(self):
        reference, muninn = self.documents()
        muninn["graph"]["edges"][0]["properties"]["properties"] = json.dumps({"role": "ceo"})
        result = self.verdict(reference, muninn)
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("gm05_dev_001" in error for error in result["errors"]))

    def test_unknown_deviation_is_rejected(self):
        reference, muninn = self.documents()
        result = self.contract.compare_gm05(reference, muninn, deviations=("GM05-DEV-999",))
        self.assertEqual(result["comparison"], "fail")
        self.assertTrue(any("gm05_dev_999" in error for error in result["errors"]))


if __name__ == "__main__":
    unittest.main()
