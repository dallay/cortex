"""Regresiones adversarias: API y persistencia son observaciones independientes."""
from types import SimpleNamespace
from unittest.mock import AsyncMock
import unittest

import test_gm05_contract as fixtures
from test_gm05_contract import module
from test_gm05_adapters import adapters, FakeGraph

contract = module("gm05_contract")


class AuthorizationTests(unittest.TestCase):
    def pair(self):
        reference, actual = fixtures.DeviationTests().documents()
        return reference, actual

    def test_only_requested_attributes_are_authorized(self):
        for attributes in ({'role': 'WRONG-NOT-CTO'}, {}):
            with self.subTest(attributes=attributes):
                reference, actual = self.pair()
                actual['relationships'][0]['attributes'] = attributes
                actual['graph']['edges'][0]['properties']['properties'] = attributes
                self.assertEqual(contract.compare_gm05(reference, actual, deviations=['GM05-DEV-001'])['comparison'], 'fail')

    def test_blob_disagreement_wrong_edge_and_other_changes_fail(self):
        for kind in ('blob', 'edge', 'description'):
            with self.subTest(kind=kind):
                reference, actual = self.pair()
                if kind == 'blob':
                    actual['graph']['edges'][0]['properties']['properties'] = {'role': 'other'}
                elif kind == 'edge':
                    for capture in (reference, actual):
                        capture['graph']['edges'][0]['source'] = capture['graph']['edges'][0]['target']
                else:
                    actual['relationships'][0]['description'] = 'wrong'
                self.assertEqual(contract.compare_gm05(reference, actual, deviations=['GM05-DEV-001'])['comparison'], 'fail')


class ObservationTests(unittest.IsolatedAsyncioTestCase):
    async def test_bridge_never_repairs_api_values_with_database(self):
        for operation, section, id_key in (
            ('add_fact', 'fact', 'Fact'), ('add_preference', 'preference', 'Preference'),
            ('add_relationship', 'relationship', 'RELATED_TO'),
            ('get_facts_about', 'fact', 'Fact'), ('get_preferences_for', 'preference', 'Preference')):
            for missing in (False, True):
                with self.subTest(operation=operation, missing=missing):
                    field = {'fact': 'object', 'preference': 'preference', 'relationship': 'description'}[section]
                    payload = {'id': 'x'} if missing else {'id': 'x', field: 'CORRUPT'}
                    post = AsyncMock(return_value=[payload] if operation.startswith('get_') else payload)
                    graph = FakeGraph({(id_key, 'x'): {'id': 'x', field: 'DB-CORRECT'}})
                    adapter = adapters.BridgeAdapter(post, graph)
                    value = await adapter.call(operation, **(dict(source_id='s', target_id='t', type='T', description='d', confidence=1, valid_from=None, valid_until=None, attributes={}) if operation == 'add_relationship' else {}))
                    value = value[0] if isinstance(value, list) else value
                    self.assertEqual(value.get(field), None if missing else 'CORRUPT')
                    if missing:
                        self.assertTrue(adapter.gaps)

    async def test_reference_never_repairs_mutations_or_reads(self):
        for operation in ('add_fact', 'add_preference', 'add_relationship',
                          'get_facts_about', 'get_preferences_for'):
            for missing in (False, True):
                with self.subTest(operation=operation, missing=missing):
                    category = 'fact' if 'fact' in operation else 'preference' if 'preference' in operation else 'relationship'
                    field = {'fact': 'object', 'preference': 'preference', 'relationship': 'description'}[category]
                    label = {'fact': 'Fact', 'preference': 'Preference', 'relationship': 'RELATED_TO'}[category]
                    value = SimpleNamespace(id='x', metadata={}, **({} if missing else {field: 'CORRUPT'}))
                    response = [value] if operation.startswith('get_') else value
                    lt = SimpleNamespace(**{operation: AsyncMock(return_value=response)})
                    adapter = adapters.ReferenceAdapter(SimpleNamespace(long_term=lt),
                        FakeGraph({(label, 'x'): {'id': 'x', field: 'DB-CORRECT'}}))
                    arguments = {'subject': 'ana', 'predicate': 'p', 'obj': 'o', 'confidence': 1,
                                 'valid_from': None, 'valid_until': None} if operation == 'add_fact' else {}
                    if operation == 'add_preference':
                        arguments = dict(category='c', preference='p', context=None, confidence=1, user_identifier='u')
                    if operation == 'add_relationship':
                        arguments = dict(source_id='s', target_id='t', type='T', description='d', confidence=1,
                                         valid_from=None, valid_until=None, attributes={})
                    if operation == 'get_facts_about': arguments = {'subject': 'ana'}
                    if operation == 'get_preferences_for': arguments = {'user_identifier': 'u'}
                    result = await adapter.call(operation, **arguments)
                    item = result[0] if isinstance(result, list) else result
                    self.assertEqual(item.get(field), None if missing else 'CORRUPT')
                    if missing: self.assertTrue(adapter.gaps)

    async def test_reference_related_supports_entityref_and_entity_relationship_tuples(self):
        for result in ([SimpleNamespace(id='e', name='ana', type='PERSON')],
                       [(SimpleNamespace(id='e', name='ana', type='PERSON'), SimpleNamespace(id='r'))]):
            with self.subTest(result=result):
                lt = SimpleNamespace(get_related_entities=AsyncMock(return_value=result))
                adapter = adapters.ReferenceAdapter(SimpleNamespace(long_term=lt), FakeGraph())
                related = await adapter.call('get_related_entities', entity_id='s')
                self.assertEqual(related[0]['id'], 'e')
                self.assertEqual(related[0]['name'], 'ana')

    async def test_reference_entityref_without_id_is_gap_not_database_lookup(self):
        lt = SimpleNamespace(get_related_entities=AsyncMock(return_value=[SimpleNamespace(name='ana', type='PERSON', id=None)]))
        adapter = adapters.ReferenceAdapter(SimpleNamespace(long_term=lt), FakeGraph())
        related = await adapter.call('get_related_entities', entity_id='s')
        self.assertIsNone(related[0].get('id'))
        self.assertTrue(adapter.gaps)

    async def test_absent_entire_bridge_response_is_a_gap(self):
        adapter = adapters.BridgeAdapter(AsyncMock(return_value=None), FakeGraph())
        self.assertEqual(await adapter.call('add_fact'), {})
        self.assertIn('add_fact:response_absent', adapter.gaps)
