"""GM-07 retrieval capture rules; no server, provider, model or oracle.

Every adapter here is a synthetic stand-in for one capture side. Nothing in this
file is a reference measurement or a ranking result.
"""
import asyncio
import json
from pathlib import Path
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

FIXTURE = json.loads((BASE / 'fixtures/gm_h4_v1.json').read_text())
SPEC = FIXTURE['gm07']
TOKENS = [item['name'] + '|' + item['type'] for item in SPEC['entities']]
ENTITIES = {token: f'e{index}' for index, token in enumerate(TOKENS)}
MESSAGES = {item['key']: f'm{index}' for index, item in enumerate(SPEC['messages'])}
OWNER = SPEC['scope']['owner']
OTHER = SPEC['scope']['other']
NORTH, SOUTH = TOKENS


class FakeSearchAdapter:
    """Side-specific stand-in for the retrieval surfaces and the graph.

    It reproduces the declared owner scoping: the reference `search_messages`
    has no owner filter, so the foreign owner's message comes back; the product
    search is scoped and does not.
    """

    def __init__(self, side='reference', vector_order=None, text_order=None,
                 related=None, edges=None, leak=None):
        self.side = side
        self.leak = leak
        self.messages, self.serial = {}, 0
        self.vector_order = vector_order or [SOUTH, NORTH]
        self.text_order = text_order or ['gm07-beta', 'gm07-alpha']
        self.related = related if related is not None else [SOUTH]
        self.edges = edges if edges is not None else [
            {'source': ENTITIES[NORTH], 'target': ENTITIES[SOUTH],
             'properties': {'type': 'CONNECTS_TO', 'confidence': 1.0}}]

    def query(self, statement, parameters):
        if '-[r:RELATED_TO]->' in statement:
            return [dict(row) for row in self.edges]
        return [{'properties': {'id': identifier, 'name': token.split('|')[0],
                                'type': token.split('|')[1]}}
                for token, identifier in ENTITIES.items()]

    async def call(self, operation, **arguments):
        if operation == 'add_entity':
            return {'entity': {'id': ENTITIES[arguments['name'] + '|'
                                             + arguments['entity_type']]}}
        if operation == 'add_relationship':
            return {'id': 'relationship'}
        if operation == 'add_message':
            self.serial += 1
            identifier = 'created-' + str(self.serial)
            for key, value in MESSAGES.items():
                if next(item for item in SPEC['messages']
                        if item['key'] == key)['text'] == arguments['content']:
                    identifier = value
            self.messages[identifier] = arguments['user_identifier']
            return {'id': identifier}
        if operation == 'search_entities':
            return [{'id': ENTITIES[token], 'name': token.split('|')[0],
                     'type': token.split('|')[1], 'score': 0.91 - 0.47 * index}
                    for index, token in enumerate(self.vector_order)]
        if operation == 'search_messages':
            if arguments['query'] == SPEC['isolation']['query']:
                foreign = self.side == 'reference' if self.leak is None else self.leak
                keys = ['gm07-alpha'] + (['gm07-owner-b'] if foreign else [])
            else:
                keys = self.text_order
            return [{'id': MESSAGES[key], 'score': 0.9 - 0.1 * index}
                    for index, key in enumerate(keys)]
        if operation == 'get_context':
            return ('## Conversation History\n### Recent Conversation\n**user**: parcel\n\n'
                    '## Relevant Knowledge\n### Relevant Entities\n'
                    '- Gm H7 Northern Depot (LOCATION)')
        if operation == 'get_related_entities':
            return [{'id': ENTITIES[token], 'name': token.split('|')[0],
                     'type': token.split('|')[1]} for token in self.related]
        raise AssertionError(operation)


def capture(side='reference', **arguments):
    from gm_h4_search import capture_gm07
    return asyncio.run(capture_gm07(FakeSearchAdapter(side=side, **arguments)))


def pair(reference, product):
    """Insert both captured sections into otherwise synthetic full captures."""
    from test_gm_h4_contract import document
    expected, actual = document('reference'), document('product')
    expected['gm07'], actual['gm07'] = reference, product
    return expected, actual


class CaptureTests(unittest.TestCase):
    def test_capture_reports_the_declared_inventory_and_ranking_order(self):
        document = capture('reference')
        self.assertEqual(document['entities'], sorted(ENTITIES))
        self.assertEqual(document['edges'], [{'source': NORTH, 'target': SOUTH,
                                              'type': 'CONNECTS_TO', 'confidence': 1.0,
                                              'extra_columns': []}])
        self.assertEqual([hit['id'] for hit in document['vector']['results']],
                         [SOUTH, NORTH])
        self.assertEqual([hit['id'] for hit in document['text']['results']],
                         ['gm07-beta', 'gm07-alpha'])
        self.assertEqual(document['graph'], {'source': NORTH, 'results': [SOUTH]})
        self.assertEqual(document['isolation']['included'], ['gm07-owner-b'])
        self.assertEqual(document['divergences'], ['gm07-reference-search-unscoped'])
        self.assertEqual(capture('product')['divergences'], [])

    def test_capture_and_contract_agree_end_to_end(self):
        from gm_h4_contract import compare_gm_h4
        expected, actual = pair(capture('reference'), capture('product'))
        verdict = compare_gm_h4(expected, actual)
        self.assertEqual(verdict['comparison'], 'pass', verdict['errors'])
        self.assertEqual(verdict['items'][4]['item'], 'gm07')

    def test_scoped_search_must_not_return_the_foreign_owner(self):
        from gm_h4_contract import compare_gm_h4
        expected, actual = pair(capture('reference'), capture('product', leak=True))
        verdict = compare_gm_h4(expected, actual)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('cross-owner' in error for error in verdict['errors']))

    def test_reordered_ranking_from_the_capture_fails(self):
        from gm_h4_contract import compare_gm_h4
        expected, actual = pair(capture('reference', vector_order=[SOUTH, NORTH]),
                                capture('product', vector_order=[NORTH, SOUTH]))
        verdict = compare_gm_h4(expected, actual)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('vector' in diff['path'] for diff in verdict['diffs']))

    def test_absent_edge_from_the_capture_fails(self):
        from gm_h4_contract import compare_gm_h4
        expected, actual = pair(capture('reference', edges=[]),
                                capture('product', edges=[]))
        verdict = compare_gm_h4(expected, actual)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('declared RELATED_TO edge' in error for error in verdict['errors']))

    def test_undeclared_message_identity_is_refused(self):
        from gm_h4_contract import ContractError
        from gm_h4_search import capture_gm07
        adapter = FakeSearchAdapter()
        original = adapter.call

        async def call(operation, **arguments):
            if operation == 'search_messages' and arguments['query'] != (
                    SPEC['isolation']['query']):
                return [{'id': 'generated-uuid', 'score': 0.5}]
            return await original(operation, **arguments)

        adapter.call = call
        with self.assertRaisesRegex(ContractError, 'undeclared identity'):
            asyncio.run(capture_gm07(adapter))

    def test_a_fixture_other_than_the_frozen_one_is_refused(self):
        from gm_h4_search import capture_gm07
        with self.assertRaisesRegex(ValueError, 'frozen GM H4 inputs'):
            asyncio.run(capture_gm07(FakeSearchAdapter(), {'gm07': {}}))


if __name__ == '__main__':
    unittest.main()
