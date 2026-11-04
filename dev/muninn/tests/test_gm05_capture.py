"""La captura GM-05 debe seguir midiendo cuando una operación no existe."""

import importlib.util
from pathlib import Path
import sys
import unittest
from uuid import uuid4


BASE = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name + "_under_test", BASE / (name + ".py"))
    loaded = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = loaded
    spec.loader.exec_module(loaded)
    return loaded


capture_module = module("gm05_capture")
contract = module("gm05_contract")
ContractAbsent = capture_module.ContractAbsent


class FakeAdapter:
    """Doble determinista que emula un lado que cumple el contrato completo."""

    def __init__(self, absent=()):
        self.absent = set(absent)
        self.preferences = []
        self.facts = []
        self.relationships = []
        self.entities = []
        self.edges = []

    async def call(self, operation, **arguments):
        if operation in self.absent:
            raise ContractAbsent("contract_absent")
        handler = getattr(self, "_" + operation)
        return handler(**arguments)

    async def snapshot(self):
        if "snapshot" in self.absent:
            raise ContractAbsent("contract_absent")
        nodes = [{"key": entity["id"], "labels": ["Entity"],
                  "properties": {"id": entity["id"], "name": entity["name"],
                                 "type": entity["type"]}}
                 for entity in self.entities]
        nodes += [{"key": record["id"], "labels": ["Preference"],
                   "properties": {"id": record["id"], "category": record["category"],
                                  "preference": record["preference"],
                                  "valid_from": record["valid_from"],
                                  "valid_until": record["valid_until"]}}
                  for record in self.preferences]
        nodes += [{"key": record["id"], "labels": ["Fact"],
                   "properties": {"id": record["id"], "subject": record["subject"],
                                  "predicate": record["predicate"], "object": record["object"],
                                  "valid_from": record["valid_from"],
                                  "valid_until": record["valid_until"]}}
                  for record in self.facts]
        return {"nodes": nodes, "edges": list(self.edges)}

    def _add_entity(self, name, type):
        existing = [entity for entity in self.entities if entity["name"] == name]
        if existing:
            return dict(existing[0])
        entity = {"id": str(uuid4()), "name": name, "type": type}
        self.entities.append(entity)
        return dict(entity)

    def _add_preference(self, category, preference, context, confidence, user_identifier,
                        applies_to=None):
        for stored in self.preferences:
            if stored["category"] == category and stored["preference"] == preference:
                duplicate = dict(stored)
                duplicate["metadata"] = {"deduplicated": True}
                return duplicate
        record = {"id": str(uuid4()), "category": category, "preference": preference,
                  "context": context, "confidence": confidence, "metadata": {},
                  "linked_entities": list(applies_to or ()),
                  "valid_from": "2026-01-01T00:00:00+00:00", "valid_until": None,
                  "user_identifier": user_identifier}
        self.preferences.append(record)
        return {key: value for key, value in record.items() if key != "user_identifier"}

    def _supersede_preference(self, old_id, new_id):
        for record in self.preferences:
            if record["id"] == old_id and record["valid_until"] is None:
                record["valid_until"] = "2026-01-02T00:00:00+00:00"
        edge = {"source": old_id, "target": new_id, "type": "SUPERSEDED_BY", "properties": {}}
        if edge not in self.edges:
            self.edges.append(edge)
        return None

    def _get_preferences_for(self, user_identifier, active_only=True):
        result = []
        for record in self.preferences:
            if record["user_identifier"] != user_identifier:
                continue
            if active_only and record["valid_until"] is not None:
                continue
            result.append({key: value for key, value in record.items()
                           if key != "user_identifier"})
        return result

    def _add_fact(self, subject, predicate, obj, confidence, valid_from, valid_until):
        for stored in self.facts:
            if stored["subject"] == subject and stored["predicate"] == predicate:
                stored["confidence"] = confidence
                stored["object"] = obj
                duplicate = dict(stored)
                duplicate["metadata"] = {"deduplicated": True}
                return duplicate
        record = {"id": str(uuid4()), "subject": subject, "predicate": predicate, "object": obj,
                  "confidence": confidence, "metadata": {},
                  "valid_from": valid_from, "valid_until": valid_until}
        self.facts.append(record)
        return dict(record)

    def _get_facts_about(self, subject, limit=10):
        found = [dict(fact) for fact in self.facts if fact["subject"] == subject]
        found.sort(key=lambda fact: (-fact["confidence"], fact["id"]))
        return found[:limit]

    def _add_relationship(self, source_id, target_id, type, description, confidence,
                          valid_from, valid_until, attributes):
        record = {"id": str(uuid4()), "source_id": source_id, "target_id": target_id,
                  "type": type, "description": description, "confidence": confidence,
                  "attributes": dict(attributes), "valid_from": valid_from,
                  "valid_until": valid_until}
        self.relationships.append(record)
        return dict(record)

    def _get_related_entities(self, entity_id):
        targets = {record["target_id"] for record in self.relationships
                   if record["source_id"] == entity_id}
        return [dict(entity) for entity in self.entities if entity["id"] in targets]


class CaptureTests(unittest.IsolatedAsyncioTestCase):
    async def capture(self, adapter):
        return await capture_module.capture_gm05(adapter)

    async def test_identical_captures_compare_pass(self):
        document = await self.capture(FakeAdapter())
        self.assertEqual(document["gaps"], [])
        self.assertEqual(contract.compare_gm05(document, document)["comparison"], "pass")

    async def test_capture_records_an_absent_operation_and_continues(self):
        document = await self.capture(FakeAdapter(absent={"supersede_preference"}))
        self.assertIn("supersede_preference:contract_absent", document["gaps"])
        self.assertIsNone(document["supersession"])
        self.assertTrue(document["facts"])
        self.assertEqual(contract.compare_gm05(document, document)["comparison"], "fail")

    async def test_capture_preserves_supplied_validity_bounds(self):
        document = await self.capture(FakeAdapter())
        temporal = [fact for fact in document["facts"] if fact["predicate"] == "trabajo_en"][0]
        self.assertEqual(temporal["valid_from"], "2026-01-01T00:00:00+00:00")
        self.assertEqual(temporal["valid_until"], "2026-06-01T00:00:00+00:00")

    async def test_capture_measures_supersession_and_deduplication(self):
        document = await self.capture(FakeAdapter())
        self.assertEqual(document["deduplication"]["preference_same_id"], True)
        self.assertEqual(document["deduplication"]["fact_flag"], True)
        self.assertEqual(document["supersession"]["edge"], True)
        self.assertEqual(document["supersession"]["old_valid_until_set"], True)
        self.assertEqual(document["supersession"]["repeat_is_noop"], True)
        self.assertEqual(len(document["supersession"]["history"]), 2)
        self.assertEqual(len(document["supersession"]["active_only"]), 1)

    async def test_missing_entities_block_links_without_aborting(self):
        document = await self.capture(FakeAdapter(absent={"add_entity"}))
        self.assertIn("add_entity:contract_absent", document["gaps"])
        self.assertEqual(document["relationships"], [])
        self.assertEqual(document["related"], [])
        self.assertTrue(document["preferences"])

    async def test_capture_merges_gaps_declared_by_the_adapter(self):
        adapter = FakeAdapter()
        adapter.gaps = ["add_fact.confidence:field_absent"]
        document = await self.capture(adapter)
        self.assertIn("add_fact.confidence:field_absent", document["gaps"])


if __name__ == "__main__":
    unittest.main()
