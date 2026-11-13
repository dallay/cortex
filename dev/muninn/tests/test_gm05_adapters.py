"""Los adaptadores no pueden disimular una operación o un campo ausentes."""

import asyncio
from datetime import datetime, timezone
import importlib.util
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


adapters = module("gm05_adapters")
ContractAbsent = adapters.ContractAbsent


class FakeGraph:
    def __init__(self, records=None, snapshot=None):
        self.records = records or {}
        self._snapshot = snapshot or {"nodes": [], "edges": []}
        self.queries = []

    def node(self, label, identifier):
        if (label, identifier) not in self.records:
            raise ContractAbsent("node_absent")
        return dict(self.records[(label, identifier)])

    def preference(self, identifier):
        return self.node("Preference", identifier)

    def fact(self, identifier):
        return self.node("Fact", identifier)

    def relationship(self, identifier):
        return self.node("RELATED_TO", identifier)

    def snapshot(self):
        return self._snapshot


class BridgeTests(unittest.IsolatedAsyncioTestCase):
    async def test_relationship_sends_its_own_arguments_and_attributes_separately(self):
        sent = {}

        async def post(operation, payload):
            sent[operation] = payload
            return {"id": "r-1"}

        graph = FakeGraph({("RELATED_TO", "r-1"): {
            "id": "r-1", "source_id": "e-1", "target_id": "e-2", "type": "WORKS_AT",
            "description": "trabaja en", "confidence": 0.75, "attributes": {"role": "cto"},
            "valid_from": "2026-03-01T00:00:00+00:00"}})
        adapter = adapters.BridgeAdapter(post, graph)
        await adapter.call("add_relationship", source_id="e-1", target_id="e-2",
                           type="WORKS_AT", description="trabaja en", confidence=0.75,
                           valid_from="2026-03-01T00:00:00+00:00", valid_until=None,
                           attributes={"role": "cto"})
        self.assertEqual(sent["add_relationship"], {
            "source_id": "e-1", "target_id": "e-2", "relationship_type": "WORKS_AT",
            "description": "trabaja en", "confidence": 0.75,
            "valid_from": "2026-03-01T00:00:00+00:00", "properties": {"role": "cto"}})

    async def test_relationship_response_uses_the_wire_names(self):
        async def post(operation, payload):
            return {"id": "r-1", "source_id": "e-1", "target_id": "e-2",
                    "relationship_type": "WORKS_AT", "description": "trabaja en",
                    "confidence": 0.75, "valid_from": "2026-03-01T00:00:00+00:00",
                    "valid_until": None, "properties": {"role": "cto"}}

        adapter = adapters.BridgeAdapter(post, FakeGraph())
        record = await adapter.call("add_relationship", source_id="e-1", target_id="e-2",
                                    type="WORKS_AT", description="trabaja en", confidence=0.75,
                                    valid_from=None, valid_until=None,
                                    attributes={"role": "cto"})
        self.assertEqual(record["type"], "WORKS_AT")
        self.assertEqual(record["attributes"], {"role": "cto"})
        self.assertNotIn("relationship_type", record)
        self.assertNotIn("properties", record)
        self.assertEqual([gap for gap in adapter.gaps if "field_absent" in gap], [])

    async def test_unknown_response_field_is_an_explicit_gap(self):
        async def post(operation, payload):
            return {"id": "p-1", "category": "food", "preference": "cafe", "context": None,
                    "confidence": 0.8, "metadata": {}, "surprise": 1}

        adapter = adapters.BridgeAdapter(post, FakeGraph())
        await adapter.call("add_preference", category="food", preference="cafe",
                           context=None, confidence=0.8, user_identifier="u",
                           applies_to=None)
        self.assertIn("add_preference.surprise:unknown_field", adapter.gaps)

    async def test_get_facts_about_observes_api_without_graph_repair(self):
        async def post(operation, payload):
            assert operation == "get_facts_about"
            assert payload == {"subject": "ana", "limit": 10}
            return [{"id": "f-1"}]

        graph = FakeGraph({("Fact", "f-1"): {
            "id": "f-1", "subject": "ana", "predicate": "vive_en", "object": "Madrid",
            "confidence": 0.9, "metadata": {}, "valid_from": "2026-02-01T00:00:00+00:00"}})
        adapter = adapters.BridgeAdapter(post, graph)
        found = await adapter.call("get_facts_about", subject="ana", limit=10)
        self.assertEqual(found, [{"id": "f-1"}])
        self.assertIn("get_facts_about.object:field_absent", adapter.gaps)

    async def test_supersede_preference_is_available(self):
        sent = {}

        async def post(operation, payload):
            sent[operation] = payload

        adapter = adapters.BridgeAdapter(post, FakeGraph())
        result = await adapter.call("supersede_preference", old_id="o", new_id="n")
        self.assertIsNone(result)
        self.assertEqual(sent["supersede_preference"], {"old_id": "o", "new_id": "n"})

    async def test_get_preferences_for_observes_api_without_graph_repair(self):
        sent = {}

        async def post(operation, payload):
            sent[operation] = payload
            return [{"id": "p-1"}]

        graph = FakeGraph({("Preference", "p-1"): {
            "id": "p-1", "category": "food", "preference": "cafe", "context": None,
            "confidence": 0.8, "metadata": {}, "linked_entities": [],
            "valid_from": "2026-01-01T00:00:00+00:00"}})
        adapter = adapters.BridgeAdapter(post, graph)
        found = await adapter.call("get_preferences_for", user_identifier="u", active_only=False)
        self.assertEqual(sent["get_preferences_for"],
                         {"user_identifier": "u", "active_only": False})
        self.assertEqual(found, [{"id": "p-1"}])
        self.assertIn("get_preferences_for.preference:field_absent", adapter.gaps)

    async def test_unpublished_operation_is_contract_absent(self):
        async def post(operation, payload):
            raise LookupError("unknown method")

        adapter = adapters.BridgeAdapter(post, FakeGraph())
        with self.assertRaises(ContractAbsent):
            await adapter.call("supersede_preference", old_id="a", new_id="b")

    async def test_fact_metadata_comes_from_the_response(self):
        async def post(operation, payload):
            return {"id": "f-1", "confidence": 0.7, "valid_from": None,
                    "valid_until": None, "metadata": {"deduplicated": True}}

        graph = FakeGraph({("Fact", "f-1"): {
            "id": "f-1", "subject": "ana", "predicate": "vive_en", "object": "Madrid",
            "confidence": 0.7, "metadata": {}, "valid_from": "2026-02-01T00:00:00+00:00"}})
        adapter = adapters.BridgeAdapter(post, graph)
        record = await adapter.call("add_fact", subject="ana", predicate="vive_en", obj="Madrid",
                                    confidence=0.7, valid_from="2026-02-01T00:00:00+00:00",
                                    valid_until=None)
        self.assertEqual(record["metadata"], {"deduplicated": True})
        self.assertIn("add_fact.object:field_absent", adapter.gaps)

    async def test_documented_arguments_are_sent_with_bridge_names(self):
        sent = {}

        async def post(operation, payload):
            sent[operation] = payload
            return {"id": "11111111-1111-4111-8111-111111111111"}

        graph = FakeGraph({("Preference", "11111111-1111-4111-8111-111111111111"):
                           {"id": "11111111-1111-4111-8111-111111111111", "category": "food",
                            "preference": "cafe", "context": None, "confidence": 0.8,
                            "metadata": {}, "linked_entities": [],
                            "valid_from": "2026-01-01T00:00:00+00:00"}})
        adapter = adapters.BridgeAdapter(post, graph)
        record = await adapter.call("add_preference", category="food", preference="cafe",
                                    context=None, confidence=0.8, user_identifier="u",
                                    applies_to=None)
        self.assertEqual(sent["add_preference"], {"category": "food", "preference": "cafe",
                                                  "confidence": 0.8, "user_identifier": "u"})
        self.assertNotIn("confidence", record)
        self.assertIn("add_preference.confidence:field_absent", adapter.gaps)
        self.assertIn("add_preference.metadata:field_absent", adapter.gaps)

    async def test_entity_uses_the_bridge_argument_name(self):
        sent = {}

        async def post(operation, payload):
            sent[operation] = payload
            return {"id": "e-1", "name": "ana", "type": "PERSON"}

        adapter = adapters.BridgeAdapter(post, FakeGraph())
        record = await adapter.call("add_entity", name="ana", type="PERSON")
        self.assertEqual(sent["add_entity"], {"name": "ana", "entity_type": "PERSON"})
        self.assertEqual(record, {"id": "e-1", "name": "ana", "type": "PERSON"})
        self.assertIn("add_entity.description:field_absent", adapter.gaps)

    async def test_relationship_keeps_supplied_validity_and_attributes(self):
        sent = {}

        async def post(operation, payload):
            sent[operation] = payload
            return {"id": "r-1"}

        graph = FakeGraph({("RELATED_TO", "r-1"): {
            "id": "r-1", "source_id": "e-1", "target_id": "e-2", "type": "WORKS_AT",
            "description": "trabaja en", "confidence": 0.75, "attributes": {"role": "cto"},
            "valid_from": "2026-03-01T00:00:00+00:00"}})
        adapter = adapters.BridgeAdapter(post, graph)
        await adapter.call("add_relationship", source_id="e-1", target_id="e-2",
                           type="WORKS_AT", description="trabaja en", confidence=0.75,
                           valid_from="2026-03-01T00:00:00+00:00", valid_until=None,
                           attributes={"role": "cto"})
        self.assertEqual(sent["add_relationship"]["properties"], {"role": "cto"})


class AliasRegressionTests(unittest.IsolatedAsyncioTestCase):
    async def test_api_alias_conflicts_reject_without_payload_mutation_or_gaps(self):
        from copy import deepcopy
        for wire, name in [('relationship_type', 'type'), ('properties', 'attributes')]:
            for first, second in [('PRIVATE-A', 'PRIVATE-B'), (None, 'PRIVATE-B'),
                                  ('PRIVATE-A', None)]:
                with self.subTest(wire=wire, first=first, second=second):
                    response = {'id': 'r', wire: first, name: second}
                    before = deepcopy(response)
                    async def post(*_args):
                        return response
                    adapter = adapters.BridgeAdapter(post, FakeGraph())
                    with self.assertRaisesRegex(ValueError, wire + '/' + name) as caught:
                        await adapter.call('add_relationship', source_id='a', target_id='b',
                                           type='WORKS_AT')
                    self.assertNotIn('PRIVATE', str(caught.exception))
                    self.assertEqual(response, before)
                    self.assertEqual(adapter.gaps, [])

    async def test_api_equal_aliases_and_single_names_preserve_projection(self):
        from copy import deepcopy
        for wire, name, value in [('relationship_type', 'type', 'WORKS_AT'),
                                   ('properties', 'attributes', {'role': 'cto'}),
                                   ('relationship_type', 'type', None),
                                   ('properties', 'attributes', None)]:
            records = []
            for fields in ({wire: value}, {name: value}, {wire: value, name: value}):
                response = {'id': 'r', **fields}
                before = deepcopy(response)
                async def post(*_args):
                    return response
                adapter = adapters.BridgeAdapter(post, FakeGraph())
                records.append(await adapter.call('add_relationship', source_id='a',
                                                  target_id='b', type='WORKS_AT'))
                self.assertEqual(response, before)
            self.assertEqual(records[0], records[1])
            self.assertEqual(records[1], records[2])


class ReferenceAliasTests(unittest.IsolatedAsyncioTestCase):
    async def test_reference_api_aliases_follow_the_same_conflict_and_equal_rules(self):
        from copy import deepcopy
        for wire, name, value in [('relationship_type', 'type', 'WORKS_AT'),
                                   ('properties', 'attributes', {'role': 'cto'})]:
            for duplicate in (value, None):
                with self.subTest(wire=wire, duplicate=duplicate):
                    response = {'id': 'r', wire: value, name: duplicate}
                    before = deepcopy(response)
                    class LongTerm:
                        async def add_relationship(self, *_args, **_kwargs):
                            return response
                    client = type('Client', (), {'long_term': LongTerm()})()
                    adapter = adapters.ReferenceAdapter(client, FakeGraph())
                    async def call():
                        return await adapter.call('add_relationship', source_id='a',
                            target_id='b', type='WORKS_AT', description=None, confidence=1,
                            valid_from=None, valid_until=None, attributes={})
                    if duplicate is None:
                        with self.assertRaisesRegex(ValueError, wire + '/' + name):
                            await call()
                        self.assertEqual(adapter.gaps, [])
                    else:
                        record = await call()
                        self.assertEqual(record[name], value)
                        self.assertFalse(any('unknown_field' in gap for gap in adapter.gaps))
                    self.assertEqual(response, before)


class ReferenceTests(unittest.IsolatedAsyncioTestCase):
    async def test_fact_validity_is_parsed_and_metadata_is_kept(self):
        recorded = {}

        class LongTerm:
            async def add_fact(self, subject, predicate, obj, *, confidence,
                               valid_from, valid_until):
                recorded.update(confidence=confidence, valid_from=valid_from,
                                valid_until=valid_until)
                return type("Fact", (), {"id": "f-1", "metadata": {"deduplicated": True}})()

        client = type("Client", (), {"long_term": LongTerm()})()
        graph = FakeGraph({("Fact", "f-1"): {
            "id": "f-1", "subject": "ana", "predicate": "vive_en", "object": "Madrid",
            "confidence": 0.7, "metadata": {}, "valid_from": "2026-02-01T00:00:00+00:00"}})
        adapter = adapters.ReferenceAdapter(client, graph)
        record = await adapter.call("add_fact", subject="ana", predicate="vive_en", obj="Madrid",
                                    confidence=0.7, valid_from="2026-02-01T00:00:00+00:00",
                                    valid_until=None)
        self.assertEqual(recorded["valid_from"],
                         datetime(2026, 2, 1, tzinfo=timezone.utc))
        self.assertIsNone(recorded["valid_until"])
        self.assertEqual(record["metadata"], {"deduplicated": True})

    async def test_unmapped_reference_operation_is_contract_absent(self):
        adapter = adapters.ReferenceAdapter(object(), FakeGraph())
        with self.assertRaises(ContractAbsent):
            await adapter.call("trace_step", conversation_id="c")


class GraphReaderTests(unittest.TestCase):
    def test_relationship_reader_rejects_conflicting_type_aliases(self):
        from copy import deepcopy
        for value in ('PRIVATE-B', None):
            properties = {'id': 'r', 'relation_type': 'PRIVATE-A', 'type': value}
            before = deepcopy(properties)
            reader = adapters.GraphReader(lambda *_args: [
                {'source': 'a', 'target': 'b', 'properties': properties}])
            with self.assertRaisesRegex(ValueError, 'relation_type/type') as caught:
                reader.relationship('r')
            self.assertNotIn('PRIVATE', str(caught.exception))
            self.assertEqual(properties, before)

    def test_relationship_reader_accepts_equal_and_single_type_aliases(self):
        for fields in ({'relation_type': 'WORKS_AT'}, {'type': 'WORKS_AT'},
                       {'relation_type': 'WORKS_AT', 'type': 'WORKS_AT'}):
            reader = adapters.GraphReader(lambda *_args: [
                {'source': 'a', 'target': 'b', 'properties': {'id': 'r', **fields}}])
            self.assertEqual(reader.relationship('r')['type'], 'WORKS_AT')

    def test_missing_node_is_contract_absent(self):
        reader = adapters.GraphReader(lambda cypher, params: [])
        with self.assertRaises(ContractAbsent):
            reader.preference("p-1")

    def test_snapshot_converts_server_temporal_objects_to_iso(self):
        class Temporal:
            def iso_format(self):
                return "2026-01-01T00:00:00.123456789+00:00"

        def query(cypher, params):
            if "labels(n)" in cypher:
                return [{"labels": ["Entity"],
                         "properties": {"id": "e-1", "created_at": Temporal()}}]
            return []

        snapshot = adapters.GraphReader(query).snapshot()
        self.assertEqual(snapshot["nodes"][0]["properties"]["created_at"],
                         "2026-01-01T00:00:00.123456789+00:00")

    def test_snapshot_edge_query_binds_its_own_source_alias(self):
        seen = []

        def query(cypher, params):
            seen.append(cypher)
            return []

        adapters.GraphReader(query).snapshot()
        edges = [cypher for cypher in seen if 'a.id AS source' in cypher][0]
        self.assertIn('(a:Entity', edges)
        self.assertNotIn('n:', edges)

    def test_snapshot_keeps_known_labels_and_normalizes_edges(self):
        rows = {
            "nodes": [{"labels": ["Preference", "Extra"],
                       "properties": {"id": "p-1", "category": "food"}}],
            "edges": [{"source": "p-1", "target": "p-2", "type": "SUPERSEDED_BY",
                       "properties": {"valid_until": "2026-01-02T00:00:00+00:00",
                                      "embedding": [0.1]}}],
        }

        def query(cypher, params):
            return rows["nodes"] if "labels(n)" in cypher else rows["edges"]

        snapshot = adapters.GraphReader(query).snapshot()
        self.assertEqual(snapshot["nodes"][0]["labels"], ["Preference"])
        self.assertEqual(snapshot["edges"][0]["properties"],
                         {"valid_until": "2026-01-02T00:00:00+00:00"})


if __name__ == "__main__":
    unittest.main()
