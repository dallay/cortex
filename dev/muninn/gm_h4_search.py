"""GM-07 retrieval capture: text, vector and graph search plus `get_context`.

One server side, the frozen GM-07 inventory and four retrieval surfaces. The
capture only projects what each side returned and maps every repository
identity onto a declared token; it never sorts a ranked list, never widens a
score and never completes a missing value from the other server.

Declared projection, before any measurement:

* vector - `search_entities` hits: entity token and similarity score, in the
  order the server returned them;
* text - `search_messages` hits for the declared owner: declared message key
  and similarity score, in the order the server returned them;
* graph - `get_related_entities` one hop from the declared source, plus the
  `RELATED_TO` inventory read symmetrically through Cypher on both sides;
* context - the fused `get_context` document verbatim;
* isolation - the foreign owner's message keys that the scoped search returned.

Graph results are unordered and are sorted by their token; every ranked list
keeps the order the server returned. The adapter normalizes one search hit to
`{'id', 'score'}` (plus `name`/`type` for entities) and never re-ranks it.
"""
from gm_h4_capture import RELATIONS, _declared, declared_divergences, frozen_fixture, inventory
from gm_h4_contract import ContractError, GM07_DIVERGENCES, entity_key


async def capture_gm07(adapter, fixture=None):
    """Seed the declared inventory, then capture the four retrieval surfaces."""
    frozen = frozen_fixture()
    if fixture is not None and fixture != frozen:
        raise ValueError('fixture differs from the frozen GM H4 inputs')
    spec = frozen['gm07']
    owner, other = spec['scope']['owner'], spec['scope']['other']
    identifiers = {}
    for item in spec['entities']:
        result = await adapter.call('add_entity', name=item['name'],
                                    entity_type=item['type'], resolve=False,
                                    deduplicate=False)
        identifiers[entity_key(item['name'], item['type'])] = str(result['entity']['id'])
    for edge in spec['edges']:
        await adapter.call('add_relationship', source_id=identifiers[edge['source']],
                           target_id=identifiers[edge['target']],
                           relationship_type=edge['type'], confidence=edge['confidence'])
    inventory_tokens, tokens = inventory(adapter.query)
    for token, identifier in identifiers.items():
        if tokens.get(identifier) != token:
            raise ContractError('gm07.entities: seeded identity is absent from the graph')
    owned = await _messages(adapter, spec, owner)
    vector = await _ranking(adapter, spec['vector'], tokens, 'search_entities',
                            'gm07.vector', **{'user_identifier': owner})
    text = await _ranking(adapter, spec['text'], {value: key for key, value in
                                                 owned.items()}, 'search_messages',
                          'gm07.text', user_identifier=owner)
    context = await adapter.call('get_context', query=spec['context']['query'],
                                 max_items=spec['context']['max_items'],
                                 user_identifier=owner)
    foreign = await _messages(adapter, spec, other)
    scoped = await adapter.call('search_messages', query=spec['isolation']['query'],
                                limit=spec['isolation']['limit'], threshold=0.0,
                                user_identifier=owner)
    known = {**owned, **foreign}
    identities = {value: token for token, value in known.items()}
    included = sorted(identities[str(hit['id'])] for hit in scoped
                      if identities.get(str(hit['id'])) in foreign)
    related = await adapter.call('get_related_entities',
                                 entity_id=identifiers[spec['graph']['source']], depth=1)
    edges = []
    for row in adapter.query(RELATIONS, {}):
        properties = row['properties']
        edges.append({'source': _declared(row['source'], tokens, 'gm07.edges'),
                      'target': _declared(row['target'], tokens, 'gm07.edges'),
                      'type': properties.get('type'),
                      'confidence': properties.get('confidence'),
                      'extra_columns': sorted(set(properties) - {'type', 'confidence'})})
    return {
        'entities': sorted({entity['key'] for entity in inventory_tokens}),
        'edges': edges,
        'vector': vector,
        'text': text,
        'graph': {'source': spec['graph']['source'],
                  'results': [entity_key(hit['name'], hit['type']) for hit in related]},
        'context': {'query': spec['context']['query'], 'document': context},
        'isolation': {'included': included},
        'divergences': declared_divergences(adapter, GM07_DIVERGENCES),
    }


async def _messages(adapter, spec, owner):
    """Create the declared messages of one owner; repository ids stay local."""
    created = {}
    for message in spec['messages']:
        if message['owner'] != owner:
            continue
        result = await adapter.call('add_message', session_id=spec['session'],
                                    role=message['role'], content=message['text'],
                                    user_identifier=owner)
        created[message['key']] = str(result['id'])
    return created


async def _ranking(adapter, declared, identities, operation, path, **arguments):
    """Project one ranked answer, keeping the returned order exactly."""
    hits = await adapter.call(operation, query=declared['query'], limit=declared['limit'],
                              threshold=0.0, **arguments)
    results, seen = [], set()
    for index, hit in enumerate(hits):
        identity = str(hit['id'])
        if identity not in identities:
            raise ContractError(f'{path}.results[{index}]: undeclared identity')
        token = identities[identity]
        if token in seen:
            raise ContractError(f'{path}.results[{index}]: duplicate result identity')
        seen.add(token)
        results.append({'id': token, 'score': hit['score']})
    return {'query': declared['query'], 'results': results}
