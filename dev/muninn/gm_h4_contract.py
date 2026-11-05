"""GM-02/03/04 fail-closed comparison contract for H4.

Every rule and the single numeric epsilon are declared here, before any
measurement runs, and disclosed by `declarations_sha256()` in the runner
summary. Nothing is normalized after seeing a difference and no deviation,
exclusion or wider tolerance exists.

Measured surfaces:

* GM-02 — extraction modes (`auto`, `skip`, `explicit`) and the same-text
  replay, observed on the graph: entity inventory, `MENTIONS` edges and
  `RELATED_TO` edges. The pinned reference writes only `confidence`,
  `start_pos` and `end_pos` on `MENTIONS` (pinned
  `graph/queries.py::LINK_MESSAGE_TO_ENTITY`); Muninn adds `derived`,
  `extractor`, `support`, `source_message_ids` and `evidence`. Only the three
  shared columns are cross-side values, so only they are compared between
  servers; the five Muninn additions are validated on the product side against
  the frozen fixture, and their presence on the reference side is a failure.
* GM-03 — the spaCy stage entity stream: `name`, POLE+O `type`, `subtype`,
  `start_pos`, `end_pos`, the pinned constant `0.85` confidence and the
  `spacy` extractor name, in document order. Offsets and labels are compared
  exactly; a one-character span shift or a different label fails. The pinned
  `ExtractedEntity` also exposes `context`, `attributes` and `id`, which are
  declared reference-only and never compared: the Muninn stage does not emit
  them and H4 claims no parity for them.
* GM-04 — the resolution/deduplication decision: observed band, matched
  candidate, resulting canonical identity, returned deduplication result and
  the pending `SAME_AS` versus merged graph effects. Similarity scores are the
  only numeric values and are compared with an absolute epsilon of `1e-9`.

Random UUIDs are never compared: node, message and candidate identities are
mapped bijectively to the fixture-derived `name|type` token or to the declared
message key, and an identity outside those declarations fails. Graph-derived
lists are unordered and are sorted by their declared token; API-derived lists
(attempts, texts, entity streams) keep their capture order and are compared
positionally.
"""
from copy import deepcopy
import hashlib
import json
import math
from pathlib import Path

FIXTURE = Path(__file__).with_name('fixtures') / 'gm_h4_v1.json'
SCHEMA = 'gm_h4-capture-v1'
SIDES = ('reference', 'product')
EPSILON = 1e-9
THRESHOLDS = {'auto_merge': 0.90, 'review': 0.85, 'fuzzy': 0.85, 'semantic': 0.8,
              'candidate_limit': 12}
ACTIONS = ('none', 'merged', 'flagged')
POLEO = ('PERSON', 'ORGANIZATION', 'LOCATION', 'EVENT', 'OBJECT')
STATUSES = ('pending', 'confirmed', 'rejected')
GM03_FIELDS = ('name', 'type', 'subtype', 'start_pos', 'end_pos', 'confidence', 'extractor')
GM03_EXCLUDED = ('context', 'attributes', 'id')
MENTIONS_SHARED = ('confidence', 'start_pos', 'end_pos')
MENTIONS_PRODUCT_ONLY = ('derived', 'extractor', 'support', 'source_message_ids', 'evidence')
ENTITY_FIELDS = ('name', 'type', 'subtype')
RELATION_FIELDS = ('source', 'target', 'type', 'confidence', 'extra_columns')
ATTEMPT_FIELDS = ('key', 'name', 'entity_type', 'action', 'is_duplicate', 'matched',
                  'similarity_score', 'match_type', 'entity', 'canonical_name')
LINK_FIELDS = ('source', 'target', 'status', 'match_type', 'confidence')

DECLARATIONS = {
    'GM02-01': ('auto, skip and explicit are captured once each, in fixture order, on the '
                'same frozen session; a missing or duplicated scenario fails.'),
    'GM02-02': ('MENTIONS cross-side columns are exactly confidence, start_pos, end_pos; '
                'any other column on the reference side fails.'),
    'GM02-03': ('the Muninn-only MENTIONS columns derived, extractor, support, '
                'source_message_ids and evidence are validated against the frozen fixture '
                'on the product side and are never inferred from the reference.'),
    'GM02-04': ('an auto mention must reproduce its entity name at its own character span '
                'and carry the pinned 0.85 confidence; skip must create no mention and no '
                'entity; explicit must link exactly the supplied references.'),
    'GM02-05': ('the same-text replay is a second identical add_message: entity nodes must '
                'not duplicate and each message keeps its own MENTIONS edges.'),
    'GM02-06': ('the selected spaCy stage emits no relations; a RELATED_TO edge is measured '
                'only through its declared type and confidence columns and any other column '
                'fails.'),
    'GM03-01': ('name, POLE+O type, subtype, start_pos, end_pos, confidence and extractor '
                'are compared exactly and in document order; no span or label tolerance '
                'exists.'),
    'GM03-02': ('confidence must be the pinned constant 0.85 for every entity; spaCy NER '
                'emits no per-entity probability.'),
    'GM03-03': ('context, attributes and id are reference-only fields of the pinned model '
                'and are not compared; the Muninn stage emits none of them.'),
    'GM03-04': ('frozen texts are captured verbatim and in fixture order; empty and '
                'whitespace texts must produce an empty entity list, never a default.'),
    'GM03-05': ('every span must reproduce its own entity name at its reported character '
                'offsets, so a shifted offset fails even when both sides agree on it.'),
    'GM03-06': ('a frozen text that GM-02 ingested in auto mode cannot yield fewer stage '
                'entities than the mentions ingestion linked for it, so a truncated stream '
                'fails even when both sides truncate identically.'),
    'GM04-01': ('the band is validated against the declared thresholds 0.90/0.85 on the '
                'observed score, and the fixture-declared exact and no-candidate bands are '
                'requirements.'),
    'GM04-02': ('similarity scores and any SAME_AS confidence are the only numeric values '
                'and use an absolute epsilon of 1e-9; every other value is exact.'),
    'GM04-03': ('candidate order is observed through nested declared candidate pools: the '
                'matched candidate of each attempt must be inside its declared pool and '
                'must agree between servers, so a reordered candidate list fails.'),
    'GM04-04': ('a merged attempt creates no node and no SAME_AS edge; a flagged attempt '
                'creates exactly one incoming node and exactly one pending SAME_AS edge to '
                'the untouched candidate; no other SAME_AS edge is authorized.'),
    'GM04-05': ('a null match reports zero score and no matched identity; the incoming '
                'entity keeps its own name-derived canonical identity.'),
    'IDENTITY': ('random UUIDs are mapped to the fixture-derived name|type token or the '
                 'declared message key; an undeclared identity fails and no name-derived '
                 'identity is ever normalized as a random UUID.'),
    'ORDER': ('API-derived lists keep capture order and are compared positionally; '
              'graph-derived lists are unordered and sorted by their declared token.'),
}


class ContractError(ValueError):
    pass


def declarations_sha256():
    encoded = json.dumps(DECLARATIONS, sort_keys=True, separators=(',', ':')).encode()
    return hashlib.sha256(encoded).hexdigest()


def fields(value, required, allowed, path):
    if not isinstance(value, dict):
        raise ContractError(f'{path}: object required')
    missing = set(required) - value.keys()
    unknown = value.keys() - set(allowed)
    if missing or unknown:
        raise ContractError(f'{path}: missing={sorted(missing)} unknown={sorted(unknown)}')


def entity_key(name, entity_type):
    if not isinstance(name, str) or not name.strip() or '|' in name:
        raise ContractError('entity name: a non-empty token without "|" is required')
    if not isinstance(entity_type, str) or not entity_type.strip() or '|' in entity_type:
        raise ContractError('entity type: a non-empty token without "|" is required')
    return name + '|' + entity_type


def number(value, path, low=0.0, high=1.0):
    if type(value) not in (int, float) or not math.isfinite(value):
        raise ContractError(path + ': finite number required')
    if not low <= value <= high:
        raise ContractError(path + f': value outside [{low}, {high}]')
    return float(value)


def _fixture():
    return json.loads(FIXTURE.read_text())


def _texts(fixture):
    return [{'key': item['key'], 'text': item['text']} for item in fixture['gm03']['texts']]


def _gm02(document, fixture):
    spec = fixture['gm02']
    section = document['gm02']
    path = 'gm02'
    fields(section, {'messages', 'entities', 'mentions', 'relations'},
           {'messages', 'entities', 'mentions', 'relations'}, path)
    scenarios = {item['key']: item for item in spec['scenarios']}
    messages, content = [], {}
    for index, message in enumerate(section['messages']):
        location = f'{path}.messages[{index}]'
        fields(message, {'key', 'scenario', 'mode', 'content'},
               {'key', 'scenario', 'mode', 'content'}, location)
        scenario = scenarios.get(message['scenario'])
        if scenario is None:
            raise ContractError(location + ': undeclared scenario')
        if message['key'] != message['scenario']:
            raise ContractError(location + ': message key must be the scenario key')
        if message['mode'] != scenario['mode']:
            raise ContractError(location + '.mode: differs from the frozen scenario')
        if message['content'] != scenario['text']:
            raise ContractError(location + '.content: differs from the frozen text')
        if message['key'] in content:
            raise ContractError(location + ': duplicate message key')
        content[message['key']] = message['content']
        messages.append({name: message[name] for name in ('key', 'scenario', 'mode', 'content')})
    if [item['scenario'] for item in messages] != [item['key'] for item in spec['scenarios']]:
        raise ContractError(path + '.messages: scenarios must be captured once, in fixture order')
    entities, by_key = [], {}
    for index, entity in enumerate(section['entities']):
        location = f'{path}.entities[{index}]'
        fields(entity, {'key'} | set(ENTITY_FIELDS), {'key'} | set(ENTITY_FIELDS), location)
        if entity['key'] != entity_key(entity['name'], entity['type']):
            raise ContractError(location + '.key: name and type must derive the key')
        if entity['key'] in by_key:
            raise ContractError(location + ': duplicate entity identity')
        if entity['subtype'] is not None and (not isinstance(entity['subtype'], str)
                                             or not entity['subtype']):
            raise ContractError(location + '.subtype: absent or a non-empty string is required')
        by_key[entity['key']] = entity
        entities.append({name: entity[name] for name in ('key',) + ENTITY_FIELDS})
    for scenario in spec['scenarios']:
        for absent in scenario.get('absent_entities', ()):
            if absent in by_key:
                raise ContractError(f'{path}.scenario[{scenario["key"]}]: declared absent entity '
                                    'is present')
    raw = []
    for index, mention in enumerate(section['mentions']):
        if not isinstance(mention, dict):
            raise ContractError(f'{path}.mentions[{index}]: object required')
        raw.append((mention.get('message'), mention.get('entity')))
    for scenario in spec['scenarios']:
        key = scenario['key']
        found = sorted(entity for message, entity in raw if message == key)
        if 'expected_mentions' in scenario and found != sorted(scenario['expected_mentions']):
            raise ContractError(f'{path}.scenario[{key}]: mentions differ from the fixture '
                                'declaration')
        if 'min_mentions' in scenario and len(found) < scenario['min_mentions']:
            raise ContractError(f'{path}.scenario[{key}]: {len(found)} mentions; the declared '
                                f'minimum is {scenario["min_mentions"]}')
    mentions, seen = [], set()
    for index, mention in enumerate(section['mentions']):
        location = f'{path}.mentions[{index}]'
        fields(mention, {'message', 'entity', 'provenance'} | set(MENTIONS_SHARED),
               {'message', 'entity', 'provenance'} | set(MENTIONS_SHARED), location)
        if mention['message'] not in content:
            raise ContractError(location + ': undeclared message identity')
        if mention['entity'] not in by_key:
            raise ContractError(location + ': undeclared entity identity')
        identity = (mention['message'], mention['entity'])
        if identity in seen:
            raise ContractError(location + ': duplicate MENTIONS edge')
        seen.add(identity)
        scene = scenarios[mention['message']]
        confidence = number(mention['confidence'], location + '.confidence')
        start, end = mention['start_pos'], mention['end_pos']
        if (start is None) != (end is None):
            raise ContractError(location + ': both character offsets or neither are required')
        if start is not None:
            if type(start) is not int or type(end) is not int or not 0 <= start < end:
                raise ContractError(location + ': ordered integer character offsets required')
            if end > len(content[mention['message']]):
                raise ContractError(location + ': character offset beyond the frozen text')
        if scene['mode'] == 'auto':
            if start is None:
                raise ContractError(location + ': auto mention requires character offsets')
            if confidence != 0.85:
                raise ContractError(location + ': auto mention requires the pinned 0.85 '
                                               'confidence')
            if content[mention['message']][start:end].strip() != by_key[mention['entity']]['name']:
                raise ContractError(location + ': auto span must reproduce its entity name')
        elif start is not None:
            raise ContractError(location + ': supplied mention carries no character offsets')
        provenance = mention['provenance']
        if document['side'] == 'reference':
            if provenance is not None:
                raise ContractError(location + '.provenance: reference exposes no additional '
                                               'MENTIONS column')
        else:
            if not isinstance(provenance, dict) or set(provenance) != set(MENTIONS_PRODUCT_ONLY):
                raise ContractError(location + '.provenance: exactly the declared Muninn '
                                               'columns are required')
            if provenance['derived'] is not (scene['mode'] == 'auto'):
                raise ContractError(location + '.provenance.derived: mode provenance differs')
            expected_extractor = 'spacy' if scene['mode'] == 'auto' else None
            if provenance['extractor'] != expected_extractor:
                raise ContractError(location + '.provenance.extractor: mode provenance differs')
            if provenance['support'] != 1:
                raise ContractError(location + '.provenance.support: one observation required')
            if provenance['source_message_ids'] != [mention['message']]:
                raise ContractError(location + '.provenance.source_message_ids: exactly the '
                                               'owning message is required')
            if provenance['evidence'] != content[mention['message']]:
                raise ContractError(location + '.provenance.evidence: the supplied message '
                                               'content is required')
        # `provenance` is a per-side validated addition, never a cross-side value.
        mentions.append({'message': mention['message'], 'entity': mention['entity'],
                         'confidence': confidence, 'start_pos': start, 'end_pos': end})
    relations = []
    for index, relation in enumerate(section['relations']):
        location = f'{path}.relations[{index}]'
        fields(relation, set(RELATION_FIELDS), set(RELATION_FIELDS), location)
        for name in ('source', 'target'):
            if relation[name] not in by_key:
                raise ContractError(location + f'.{name}: undeclared entity identity')
        if not isinstance(relation['type'], str) or not relation['type']:
            raise ContractError(location + '.type: non-empty string required')
        number(relation['confidence'], location + '.confidence')
        if relation['extra_columns'] != []:
            raise ContractError(location + '.extra_columns: undeclared relation column')
        relations.append({name: relation[name] for name in RELATION_FIELDS})
    return {'messages': messages,
            'entities': sorted(entities, key=lambda item: item['key']),
            'mentions': sorted(mentions, key=lambda item: (item['message'], item['entity'])),
            'relations': sorted(relations, key=lambda item: (item['source'], item['type'],
                                                             item['target']))}


def _gm03(document, fixture):
    section = document['gm03']
    path = 'gm03'
    fields(section, {'texts', 'entities'}, {'texts', 'entities'}, path)
    expected = _texts(fixture)
    if section['texts'] != expected:
        raise ContractError(path + '.texts: frozen texts must be captured exactly and in '
                                   'order')
    texts = {item['key']: item['text'] for item in expected}
    position = {item['key']: index for index, item in enumerate(expected)}
    entities, last_group, previous = [], -1, {}
    for index, entity in enumerate(section['entities']):
        location = f'{path}.entities[{index}]'
        fields(entity, {'text'} | set(GM03_FIELDS), {'text'} | set(GM03_FIELDS), location)
        text = entity['text']
        if text not in position:
            raise ContractError(location + '.text: undeclared frozen text')
        if position[text] < last_group:
            raise ContractError(location + ': the spaCy stage stream must stay in document '
                                           'order')
        last_group = position[text]
        if not isinstance(entity['name'], str) or not entity['name'].strip():
            raise ContractError(location + '.name: non-empty string required')
        if entity['type'] not in POLEO:
            raise ContractError(location + '.type: undeclared POLE+O type')
        if entity['subtype'] is not None and (not isinstance(entity['subtype'], str)
                                              or not entity['subtype']):
            raise ContractError(location + '.subtype: absent or a non-empty string is required')
        start, end = entity['start_pos'], entity['end_pos']
        if type(start) is not int or type(end) is not int or not 0 <= start < end:
            raise ContractError(location + ': ordered integer character offsets required')
        if end > len(texts[text]):
            raise ContractError(location + ': character offset beyond the frozen text')
        if previous.get(text) is not None and start < previous[text]:
            raise ContractError(location + ': entity stream is not in character order')
        previous[text] = start
        if texts[text][start:end].strip() != entity['name']:
            raise ContractError(location + ': span must reproduce its entity name')
        if entity['confidence'] != 0.85:
            raise ContractError(location + '.confidence: the pinned constant 0.85 is required')
        if entity['extractor'] != 'spacy':
            raise ContractError(location + '.extractor: the pinned stage name is required')
        entities.append({name: entity[name] for name in ('text',) + GM03_FIELDS})
    return {'texts': expected, 'entities': entities}


def _gm04(document, fixture):
    spec = fixture['gm04']
    section = document['gm04']
    path = 'gm04'
    fields(section, {'seeds', 'attempts', 'entities', 'links'},
           {'seeds', 'attempts', 'entities', 'links'}, path)
    seeds, seed_keys = [], []
    for index, seed in enumerate(section['seeds']):
        location = f'{path}.seeds[{index}]'
        fields(seed, {'key', 'name', 'type'}, {'key', 'name', 'type'}, location)
        if seed['key'] != entity_key(seed['name'], seed['type']):
            raise ContractError(location + '.key: name and type must derive the key')
        if seed['key'] in seed_keys:
            raise ContractError(location + ': duplicate seeded candidate')
        seed_keys.append(seed['key'])
        seeds.append({name: seed[name] for name in ('key', 'name', 'type')})
    declared = {entity_key(seed['name'], seed['type']) for seed in spec['seeds']}
    if set(seed_keys) != declared:
        raise ContractError(path + '.seeds: the frozen candidate inventory must be seeded')
    attempts = []
    if [attempt['key'] for attempt in section['attempts']] != [item['key'] for item in
                                                               spec['attempts']]:
        raise ContractError(path + '.attempts: attempts must be captured once, in fixture '
                                   'order')
    for index, attempt in enumerate(section['attempts']):
        location = f'{path}.attempts[{index}]'
        fields(attempt, set(ATTEMPT_FIELDS), set(ATTEMPT_FIELDS), location)
        frozen = spec['attempts'][index]
        if attempt['name'] != frozen['name'] or attempt['entity_type'] != frozen['entity_type']:
            raise ContractError(location + ': input differs from the frozen attempt')
        if attempt['action'] not in ACTIONS:
            raise ContractError(location + '.action: undeclared deduplication action')
        incoming = entity_key(attempt['name'], attempt['entity_type'])
        if attempt['entity'] != incoming and attempt['entity'] not in seed_keys:
            raise ContractError(location + '.entity: undeclared identity')
        if attempt['is_duplicate'] is not (attempt['action'] != 'none'):
            raise ContractError(location + '.is_duplicate: differs from the observed action')
        score = number(attempt['similarity_score'], location + '.similarity_score')
        if attempt['action'] == 'none':
            if attempt['matched'] is not None:
                raise ContractError(location + '.matched: no match without a deduplication hit')
            if score != 0.0:
                raise ContractError(location + '.similarity_score: a null match reports zero')
            if attempt['match_type'] is not None:
                raise ContractError(location + '.match_type: no match type without a hit')
            if attempt['entity'] != incoming:
                raise ContractError(location + '.entity: a distinct attempt keeps its own '
                                               'canonical identity')
        else:
            if attempt['matched'] not in seed_keys:
                raise ContractError(location + '.matched: undeclared matched identity')
            if frozen.get('pool') is not None and attempt['matched'] not in frozen['pool']:
                raise ContractError(location + '.matched: outside the declared candidate pool')
            if attempt['action'] == 'merged' and attempt['entity'] != attempt['matched']:
                raise ContractError(location + '.entity: a merge returns the matched identity')
            if attempt['action'] == 'flagged' and attempt['entity'] != incoming:
                raise ContractError(location + '.entity: a flagged mention keeps its own node')
            if not isinstance(attempt['match_type'], str) or not attempt['match_type']:
                raise ContractError(location + '.match_type: non-empty string required')
        if attempt['canonical_name'] != attempt['entity'].split('|')[0]:
            raise ContractError(location + '.canonical_name: the name-derived canonical '
                                           'identity is required')
        band = ('merged' if score >= THRESHOLDS['auto_merge']
                else 'flagged' if score >= THRESHOLDS['review'] else 'none')
        if band != attempt['action']:
            raise ContractError(location + '.action: band must match the observed score')
        if 'expect_band' in frozen and attempt['action'] != frozen['expect_band']:
            raise ContractError(location + '.action: differs from the frozen band declaration')
        if 'expect_score' in frozen and abs(score - frozen['expect_score']) > EPSILON:
            raise ContractError(location + '.similarity_score: differs from the frozen score '
                                           'declaration')
        attempts.append({name: attempt[name] for name in ATTEMPT_FIELDS})
    entities, entity_keys = [], []
    for index, entity in enumerate(section['entities']):
        location = f'{path}.entities[{index}]'
        fields(entity, {'key', 'name', 'type'}, {'key', 'name', 'type'}, location)
        if entity['key'] != entity_key(entity['name'], entity['type']):
            raise ContractError(location + '.key: name and type must derive the key')
        if entity['key'] in entity_keys:
            raise ContractError(location + ': duplicate entity identity')
        entity_keys.append(entity['key'])
        entities.append({name: entity[name] for name in ('key', 'name', 'type')})
    if not set(seed_keys) <= set(entity_keys):
        raise ContractError(path + '.entities: seeded candidates disappeared from the graph')
    links = []
    for index, link in enumerate(section['links']):
        location = f'{path}.links[{index}]'
        fields(link, set(LINK_FIELDS), set(LINK_FIELDS), location)
        for name in ('source', 'target'):
            if link[name] not in entity_keys:
                raise ContractError(location + f'.{name}: undeclared entity identity')
        if link['status'] not in STATUSES:
            raise ContractError(location + '.status: undeclared SAME_AS status')
        number(link['confidence'], location + '.confidence')
        if link['match_type'] is not None and (not isinstance(link['match_type'], str)
                                               or not link['match_type']):
            raise ContractError(location + '.match_type: absent or a non-empty string is '
                                           'required')
        links.append({name: link[name] for name in LINK_FIELDS})
    expected = {(attempt['entity'], attempt['matched'], 'pending') for attempt in attempts
                if attempt['action'] == 'flagged'}
    observed = {(link['source'], link['target'], link['status']) for link in links}
    if observed != expected:
        raise ContractError(path + '.links: exactly the pending SAME_AS edges of flagged '
                                   'attempts are required')
    return {'seeds': seeds, 'attempts': attempts,
            'entities': sorted(entities, key=lambda item: item['key']),
            'links': sorted(links, key=lambda item: (item['source'], item['target'],
                                                     item['status']))}


def canonical_gm_h4(document):
    if not isinstance(document, dict):
        raise ContractError('capture: object required')
    fields(document, {'schema', 'side', 'gm02', 'gm03', 'gm04'},
           {'schema', 'side', 'gm02', 'gm03', 'gm04'}, 'capture')
    if document['schema'] != SCHEMA:
        raise ContractError('capture.schema: unsupported capture')
    if document['side'] not in SIDES:
        raise ContractError('capture.side: unknown capture side')
    fixture = _fixture()
    if fixture['numeric_epsilon'] != EPSILON:
        raise ContractError('fixture.numeric_epsilon: differs from the declared epsilon')
    sections = {'gm02': _gm02(document, fixture), 'gm03': _gm03(document, fixture),
                'gm04': _gm04(document, fixture)}
    _cross_section(sections, fixture)
    return sections


def _cross_section(container, fixture):
    """Fixture-grounded invariants that span two sections of one capture."""
    streams = {item['key'] for item in fixture['gm03']['texts']}
    counted = {}
    for entity in container['gm03']['entities']:
        counted[entity['text']] = counted.get(entity['text'], 0) + 1
    for scenario in fixture['gm02']['scenarios']:
        if scenario['mode'] != 'auto' or scenario['key'] not in streams:
            continue
        linked = len([mention for mention in container['gm02']['mentions']
                      if mention['message'] == scenario['key']])
        if counted.get(scenario['key'], 0) < linked:
            raise ContractError('gm03: the stage stream is shorter than the GM-02 auto mentions '
                                'of the same frozen text')


def _numeric(key, path):
    return key == 'similarity_score' or (key == 'confidence' and path.startswith('gm04.links'))


def _diff(left, right, path, diffs, key=None):
    if isinstance(left, dict) and isinstance(right, dict):
        for name in sorted(left.keys() | right.keys()):
            child = path + '.' + name if path else name
            if name not in left or name not in right:
                diffs.append({'path': child, 'expected_present': name in left,
                              'actual_present': name in right, 'expected': left.get(name),
                              'actual': right.get(name)})
            else:
                _diff(left[name], right[name], child, diffs, name)
    elif isinstance(left, list) and isinstance(right, list):
        if len(left) != len(right):
            diffs.append({'path': path + '.length', 'expected': len(left),
                          'actual': len(right)})
        for index, (one, two) in enumerate(zip(left, right)):
            _diff(one, two, f'{path}[{index}]', diffs, key)
    elif (_numeric(key, path) and type(left) in (int, float) and type(right) in (int, float)):
        if abs(float(left) - float(right)) > EPSILON:
            diffs.append({'path': path, 'expected': left, 'actual': right})
    elif (type(left) is not type(right)
          and not (type(left) in (int, float) and type(right) in (int, float))) or left != right:
        diffs.append({'path': path, 'expected': left, 'actual': right})


def compare_gm_h4(expected, actual, epsilon=None):
    """Public contract: never throw; retain values and structural failures."""
    errors, diffs, canonical = [], [], []
    if epsilon is not None and (type(epsilon) not in (int, float) or epsilon != EPSILON):
        errors.append('epsilon: only the declared 1e-09 tolerance is authorized')
    sides = []
    for side, document in (('expected', expected), ('actual', actual)):
        sides.append(document.get('side') if isinstance(document, dict) else None)
        try:
            canonical.append(canonical_gm_h4(document))
        except (ValueError, TypeError, KeyError, AttributeError) as error:
            errors.append(f'{side}: {error}')
            fallback = ({name: value for name, value in document.items() if name != 'side'}
                        if isinstance(document, dict) else document)
            canonical.append(deepcopy(fallback))
    if sides[0] == sides[1]:
        errors.append('capture.side: two different server sides are required')
    _diff(*canonical, '', diffs)
    errors.extend(diff['path'] + ': different value, type or presence' for diff in diffs)
    items = []
    for name in ('gm02', 'gm03', 'gm04'):
        item_diffs = [diff for diff in diffs if diff['path'].startswith(name)]
        items.append({'item': name, 'verdict': 'fail' if errors or item_diffs else 'pass',
                      'diffs': item_diffs})
    return {'comparison': 'fail' if errors or diffs else 'pass', 'epsilon': EPSILON,
            'declarations_sha256': declarations_sha256(), 'errors': errors, 'diffs': diffs,
            'items': items}


def main(argv=None):
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--expected', required=True, type=Path,
                        help='Raw captured reference JSON (read only)')
    parser.add_argument('--actual', required=True, type=Path,
                        help='Raw captured product JSON (read only)')
    args = parser.parse_args(argv)
    try:
        verdict = compare_gm_h4(json.loads(args.expected.read_text()),
                                json.loads(args.actual.read_text()))
    except (OSError, ValueError):
        verdict = {'comparison': 'fail', 'diffs': [],
                   'errors': ['capture: unreadable or invalid JSON'], 'items': []}
    print(json.dumps(verdict, sort_keys=True, allow_nan=False))
    return 0 if verdict['comparison'] == 'pass' else 2


if __name__ == '__main__':
    raise SystemExit(main())
