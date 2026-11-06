"""Frozen GM-02, GM-04 and GM-06 operations, shared verbatim by both captured sides.

The adapter owns its own API projection and its own database connection; this
module only freezes the scenario, reads the graph symmetrically and maps every
identity onto the fixture-derived token. A capture never completes a missing
value from the other server: an absent edge, column or identity fails here.
"""
from copy import deepcopy
import json
from pathlib import Path

from gm_h4_contract import (ContractError, GM06_COMPARED, GM06_DIVERGENCES, GM06_PRODUCT_ONLY,
                             GM06_REFERENCE_ONLY, GM06_UNCOMPARED, MENTIONS_PRODUCT_ONLY,
                             MENTIONS_SHARED, SCHEMA, entity_key)

FIXTURE = Path(__file__).with_name('fixtures') / 'gm_h4_v1.json'

ENTITIES = 'MATCH (e:Entity) RETURN properties(e) AS properties'
MENTIONS = ('MATCH (m:Message)-[r:MENTIONS]->(e:Entity) '
            'WHERE m.id IN $message_ids '
            'RETURN m.id AS message, e.id AS entity, properties(r) AS properties')
RELATIONS = ('MATCH (a:Entity)-[r:RELATED_TO]->(b:Entity) '
             'RETURN a.id AS source, b.id AS target, properties(r) AS properties')
SAME_AS = ('MATCH (a:Entity)-[r:SAME_AS]->(b:Entity) '
           'RETURN a.id AS source, b.id AS target, properties(r) AS properties')
ENTITY_STATE = ('MATCH (e:Entity {id: $id}) RETURN properties(e) AS properties, '
                'e.location.latitude AS latitude, e.location.longitude AS longitude')
GM06_PROPERTIES = (GM06_COMPARED + GM06_REFERENCE_ONLY + GM06_PRODUCT_ONLY
                   + GM06_UNCOMPARED)


def frozen_fixture():
    return json.loads(FIXTURE.read_text())


def _fixture(fixture):
    frozen = frozen_fixture()
    if fixture is None:
        return frozen
    if fixture != frozen:
        raise ValueError('fixture differs from the frozen GM H4 inputs')
    return frozen


def inventory(query):
    """Entity inventory plus the bijective uuid -> name|type token map."""
    entities, identifiers = [], {}
    for row in query(ENTITIES, {}):
        properties = row['properties']
        key = entity_key(properties.get('name'), properties.get('type'))
        identifiers[str(properties['id'])] = key
        entities.append({'key': key, 'name': properties['name'], 'type': properties['type'],
                         'subtype': properties.get('subtype')})
    return entities, identifiers


def _declared(identity, identifiers, path):
    key = identifiers.get(str(identity))
    if key is None:
        raise ContractError(path + ': undeclared identity')
    return key


def _reference(item, seeded):
    if not isinstance(item, dict):
        raise ContractError('explicit_mentions: object required')
    if 'id_of' in item:
        if set(item) != {'id_of'} or item['id_of'] not in seeded:
            raise ContractError('explicit_mentions: undeclared id_of reference')
        return {'id': seeded[item['id_of']]}
    if set(item) == {'name'}:
        return {'name': item['name']}
    if set(item) == {'name', 'type'}:
        return {'name': item['name'], 'type': item['type']}
    raise ContractError('explicit_mentions: name or name+type reference required')


async def capture_gm02(adapter, fixture=None):
    """Run the declared extraction-mode scenarios, then read the graph."""
    spec = _fixture(fixture)['gm02']
    seeded = {}
    for seed in spec['seeded']:
        result = await adapter.call('add_entity', name=seed['name'], entity_type=seed['type'],
                                    resolve=False, deduplicate=False)
        entity = result['entity']
        seeded[entity_key(seed['name'], seed['type'])] = str(entity['id'])
    messages, owners = [], {}
    for scenario in spec['scenarios']:
        references = [_reference(item, seeded)
                      for item in scenario.get('explicit_mentions', ())]
        created = await adapter.call('add_message', session_id=spec['session_id'],
                                     role=spec['role'], content=scenario['text'],
                                     extraction_mode=scenario['mode'],
                                     explicit_mentions=references,
                                     extract_entities=None, extract_relations=True,
                                     generate_embedding=True)
        messages.append({'key': scenario['key'], 'scenario': scenario['key'],
                         'mode': scenario['mode'], 'content': scenario['text']})
        owners[str(created['id'])] = scenario['key']
    entities, identifiers = inventory(adapter.query)
    import sys
    unscoped = adapter.query('MATCH (m:Message)-[r:MENTIONS]->(e:Entity) '
                            'RETURN m.id AS message, e.id AS entity, '
                            'properties(r) AS properties', {})
    print(f'[gm02-diag] side={adapter.side} unscoped={len(unscoped)} '
          f'declared={len(owners)}', file=sys.stderr, flush=True)
    for row in unscoped[:4]:
        print('[gm02-diag] row:', row['message'], '->', row['entity'],
              file=sys.stderr, flush=True)
    mentions = []
    # GM-02 compares the mentions its own scenarios created; messages seeded by
    # other sections are captured and validated by their own sections.
    for row in adapter.query(MENTIONS, {'message_ids': sorted(owners)}):
        properties = row['properties']
        unknown = set(properties) - set(MENTIONS_SHARED) - set(MENTIONS_PRODUCT_ONLY)
        if unknown:
            raise ContractError('gm02.mentions: undeclared MENTIONS column '
                                + str(sorted(unknown)))
        provenance = {name: deepcopy(properties[name]) for name in MENTIONS_PRODUCT_ONLY
                      if name in properties} or None
        if provenance and 'source_message_ids' in provenance:
            provenance['source_message_ids'] = [
                _declared(value, owners, 'gm02.mentions.provenance.source_message_ids')
                for value in provenance['source_message_ids']]
        mentions.append({
            'message': _declared(row['message'], owners, 'gm02.mentions'),
            'entity': _declared(row['entity'], identifiers, 'gm02.mentions'),
            'confidence': properties.get('confidence'),
            'start_pos': properties.get('start_pos'),
            'end_pos': properties.get('end_pos'),
            'provenance': provenance,
        })
    relations = []
    for row in adapter.query(RELATIONS, {}):
        properties = row['properties']
        relations.append({
            'source': _declared(row['source'], identifiers, 'gm02.relations'),
            'target': _declared(row['target'], identifiers, 'gm02.relations'),
            'type': properties.get('type'),
            'confidence': properties.get('confidence'),
            'extra_columns': sorted(set(properties) - {'type', 'confidence'}),
        })
    return {'messages': messages, 'entities': entities, 'mentions': mentions,
            'relations': relations}


async def capture_gm04(adapter, fixture=None):
    """Seed the declared candidates, run the attempts, then read decisions."""
    spec = _fixture(fixture)['gm04']
    for seed in spec['seeds']:
        await adapter.call('add_entity', name=seed['name'], entity_type=seed['type'],
                           resolve=False, deduplicate=False)
    _, seeded = inventory(adapter.query)
    attempts = []
    for attempt in spec['attempts']:
        result = await adapter.call('add_entity', name=attempt['name'],
                                    entity_type=attempt['entity_type'], resolve=True,
                                    deduplicate=True, resolution=spec.get('config'))
        entity, deduplication = result['entity'], result['deduplication']
        matched = None
        if deduplication['matched_entity_id'] is not None:
            matched = _declared(deduplication['matched_entity_id'], seeded,
                                'gm04.attempts.matched')
        attempts.append({'key': attempt['key'], 'name': attempt['name'],
                         'entity_type': attempt['entity_type'],
                         'action': deduplication['action'],
                         'is_duplicate': deduplication['is_duplicate'],
                         'matched': matched,
                         'similarity_score': deduplication['similarity_score'],
                         'match_type': deduplication['match_type'],
                         'entity': entity_key(entity['name'], entity['type']),
                         'canonical_name': entity.get('canonical_name')})
    entities, identifiers = inventory(adapter.query)
    links = []
    for row in adapter.query(SAME_AS, {}):
        properties = row['properties']
        links.append({
            'source': _declared(row['source'], identifiers, 'gm04.links'),
            'target': _declared(row['target'], identifiers, 'gm04.links'),
            'status': properties.get('status'),
            'match_type': properties.get('match_type'),
            'confidence': properties.get('confidence'),
        })
    return {'seeds': [{'key': entity_key(seed['name'], seed['type']), 'name': seed['name'],
                       'type': seed['type']} for seed in spec['seeds']],
            'attempts': attempts,
            'entities': [{'key': entity['key'], 'name': entity['name'],
                          'type': entity['type']} for entity in entities],
            'links': links}


def declared_divergences(adapter, declared):
    """The declared per-side divergences; an unknown side is never guessed."""
    side = getattr(adapter, 'side', None)
    if side not in declared:
        raise ContractError('divergences: unknown capture side')
    return sorted(declared[side])


async def capture_gm06(adapter, fixture=None):
    """Enrich and geocode the declared entities from recorded provider answers.

    `enrich_entity` and `geocode_entity` are the frozen operations; the adapter
    owns how each side triggers them. Nothing is written by this module: the
    properties and the point are read back from the graph after each attempt.
    """
    spec = _fixture(fixture)['gm06']
    attempts, requests = [], {}
    observed = adapter.provider_requests()
    for attempt in spec['attempts']:
        before = len(observed)
        result = await adapter.call(attempt['operation'] + '_entity',
                                    name=attempt['name'], entity_type=attempt['type'])
        observed = adapter.provider_requests()
        identifier = str(result['entity']['id'])
        rows = adapter.query(ENTITY_STATE, {'id': identifier})
        if not isinstance(rows, list) or len(rows) != 1:
            raise ContractError(f'gm06.attempts[{attempt["key"]}]: entity state is absent')
        row = rows[0]
        properties = {name: row['properties'].get(name) for name in GM06_COMPARED}
        for name in GM06_PRODUCT_ONLY:
            properties[name] = row['properties'].get(name)
        for name in GM06_REFERENCE_ONLY + GM06_UNCOMPARED:
            properties[name] = name in row['properties']
        point = None
        if row['latitude'] is not None and row['longitude'] is not None:
            point = {'latitude': row['latitude'], 'longitude': row['longitude']}
        attempts.append({'key': attempt['key'], 'operation': attempt['operation'],
                         'status': result['status'], 'failure': result.get('failure'),
                         'properties': properties, 'location': point})
        requests[attempt['key']] = len(observed) - before
    spacing = {}
    for provider in spec['spacing']:
        points = [item['at'] for item in observed if item['provider'] == provider]
        gaps = [second - first for first, second in zip(points, points[1:])]
        spacing[provider] = {'requests': len(points),
                             'min_gap_seconds': min(gaps) if gaps else None}
    return {'attempts': attempts, 'spacing': spacing, 'attempt_requests': requests,
            'divergences': declared_divergences(adapter, GM06_DIVERGENCES)}


def capture_schema():
    return SCHEMA
