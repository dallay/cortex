"""GM-02/03/04/06/07 fail-closed comparison contract for H4.

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
* GM-04 - the resolution/deduplication decision: observed band, matched
  candidate, resulting canonical identity, returned deduplication result and
  the pending `SAME_AS` versus merged graph effects. Similarity scores are the
  only numeric values and are compared with an absolute epsilon of `1e-9`.
* GM-06 - enrichment and geocoding from recorded provider answers: the written
  enrichment properties, the stored geocode point, warm-cache replay and the
  rate-limit spacing between recorded provider requests. Providers answer from
  local recorded fixtures only; no live Wikipedia, Diffbot or Nominatim call
  exists. Declared product-only surface (closed failure vocabulary, the point
  index prerequisite, no background worker, no fuzzy fallback) is validated
  per side and never compared across servers.
* GM-07 - text, vector and graph search plus the fused `get_context` document:
  returned ids, ranking order (compared, never sorted), the context document
  and one declared owner-scope isolation case. Entity-message similarity scores
  use the same declared `1e-9` epsilon as GM-04.

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
# Cross-engine float32 storage rounds identical vectors differently (the
# Neo4j index answered 0.999994158744812 to the reference and exactly 1.0
# to the product for the same pair), so similarity scores carry the
# declared float32 tolerance while every other numeric stays exact.
SIMILARITY_EPSILON = 1e-5
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
GM06_COMPARED = ('enriched_description', 'enrichment_provider', 'wikipedia_url',
                 'wikidata_id', 'image_url')
GM06_REFERENCE_ONLY = ('enrichment_data',)
GM06_PRODUCT_ONLY = ('enriched_summary', 'diffbot_uri')
GM06_UNCOMPARED = ('enriched_at',)
GM06_STATUSES = ('success', 'not_found', 'error', 'rate_limited', 'skipped')
GM06_FAILURES = ('timeout', 'transport', 'service', 'credential', 'response',
                 'rate_limited', 'unsupported_type')
GM06_OPERATIONS = ('enrich', 'geocode')
GM07_RESULT_FIELDS = ('id', 'score')
GM06_DIVERGENCES = {
    'reference': frozenset(),
    'product': frozenset({'gm06-closed-failure-vocabulary',
                          'gm06-geocode-requires-point-index',
                          'gm06-no-background-worker',
                          'gm06-no-fuzzy-fallback'}),
}
GM07_DIVERGENCES = {
    'reference': frozenset({'gm07-reference-search-unscoped'}),
    'product': frozenset(),
}

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
                'that carry the declared float32 similarity tolerance of 1e-5 (cross-engine '
                'storage rounds identical vectors differently); every other value stays '
                'exact at 1e-9.'),
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
    'GM06-01': ('the enrichment property document carries exactly the declared compared, '
                'reference-only, product-only and uncompared names; a dropped or unknown '
                'property key fails and no value is inferred from the other server.'),
    'GM06-02': ('only enriched_description, enrichment_provider, wikipedia_url, '
                'wikidata_id and image_url are cross-side values; the reference-only '
                'enrichment_data blob, the product-only enriched_summary and diffbot_uri '
                'and the server-stamped enriched_at are validated per side only.'),
    'GM06-03': ('a successful enrichment writes every declared compared value exactly as '
                'the frozen provider answer declares it; a failure, absence or skipped '
                'answer writes none of them and reports its status.'),
    'GM06-04': ('the geocode point is the recorded provider coordinate and uses the '
                'declared 1e-9 epsilon; a missing, earlier or foreign point fails.'),
    'GM06-05': ('a warm-cache replay issues zero provider requests and repeats the '
                'properties of the attempt it replays; a changed value or one extra '
                'request fails.'),
    'GM06-06': ('consecutive recorded requests to one provider are at least '
                'rate_limit_seconds - clock_slack_seconds apart, both declared in the '
                'frozen fixture; provider request totals match the declared per-side '
                'count and an unmatched provider answer fails before any write.'),
    'GM06-07': ('the closed failure vocabulary is product-only: a value outside it, a '
                'failure on the reference side or an undeclared enrichment status fails; '
                'the declared product divergences are the only authorized differences '
                'and an undeclared divergence fails.'),
    'GM07-01': ('vector and text results are compared positionally in the returned order; '
                'a reordered ranking fails even when both sides contain the same ids.'),
    'GM07-02': ('entity and message similarity scores are the only GM-07 numeric values and '
                'use the declared absolute epsilon of 1e-9; ranking order itself is exact.'),
    'GM07-03': ('every returned identity must be a declared token and every ranked score '
                'must be finite, non-increasing along the list and duplicate-free.'),
    'GM07-04': ('the declared RELATED_TO inventory read through Cypher is the same on both '
                'sides and contains every frozen edge; an absent edge fails.'),
    'GM07-05': ('the fused get_context document is compared character for character and '
                'carries the declared sections in the declared order.'),
    'GM07-06': ('the owner-scoped message search is validated per side: the product result '
                'contains no foreign-owner message, the reference result contains exactly '
                'the declared foreign-owner key, and neither the result nor the context '
                'document leaks foreign content into the compared values.'),
    'GM07-07': ('the declared per-side divergences are the only authorized differences; an '
                'undeclared divergence id fails.'),
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
        if sorted(relation['extra_columns']) != sorted(spec['relation_extra_columns']):
            raise ContractError(location + '.extra_columns: differs from the declared set')
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
        if 'expect_score' in frozen:
            # Cross-engine float32 storage rounds identical vectors differently:
            # the declared expectation is per side, and the cross-side epsilon
            # still governs the recorded score comparison.
            expected_score = frozen['expect_score']
            if isinstance(expected_score, dict):
                expected_score = expected_score[document['side']]
            if abs(score - expected_score) > EPSILON:
                raise ContractError(location + '.similarity_score: differs from the frozen '
                                               'score declaration')
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


def _divergences(section, side, declared, path):
    """Per-side declared differences; an undeclared or missing id fails closed."""
    value = section['divergences']
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ContractError(path + '.divergences: list of declared ids required')
    if len(set(value)) != len(value):
        raise ContractError(path + '.divergences: duplicate declaration')
    unknown = set(value) - declared[side]
    if unknown:
        raise ContractError(path + '.divergences: undeclared divergence ' + str(sorted(unknown)))
    missing = declared[side] - set(value)
    if missing:
        raise ContractError(path + '.divergences: declared divergence absent '
                            + str(sorted(missing)))


def _gm06(document, fixture):
    spec, side = fixture['gm06'], document['side']
    section, path = document['gm06'], 'gm06'
    fields(section, {'attempts', 'spacing', 'attempt_requests', 'divergences'},
           {'attempts', 'spacing', 'attempt_requests', 'divergences'}, path)
    if tuple(spec['compared_properties']) != GM06_COMPARED or \
            tuple(spec['reference_only_properties']) != GM06_REFERENCE_ONLY or \
            tuple(spec['product_only_properties']) != GM06_PRODUCT_ONLY or \
            tuple(spec['uncompared_properties']) != GM06_UNCOMPARED:
        raise ContractError('fixture.gm06: the property declaration differs from the contract')
    names = set(GM06_COMPARED) | set(GM06_REFERENCE_ONLY) | set(GM06_PRODUCT_ONLY) | set(
        GM06_UNCOMPARED)
    frozen = {item['key']: item for item in spec['attempts']}
    if [item.get('key') for item in section['attempts']] != [item['key'] for item in
                                                             spec['attempts']]:
        raise ContractError(path + '.attempts: attempts must be captured once, in fixture order')
    attempts, compared = [], {}
    for index, attempt in enumerate(section['attempts']):
        location = f'{path}.attempts[{index}]'
        fields(attempt, {'key', 'operation', 'status', 'failure', 'properties', 'location'},
               {'key', 'operation', 'status', 'failure', 'properties', 'location'}, location)
        declaration = frozen[attempt['key']]
        if attempt['operation'] not in GM06_OPERATIONS:
            raise ContractError(location + '.operation: undeclared enrichment operation')
        if attempt['operation'] != declaration['operation']:
            raise ContractError(location + '.operation: differs from the frozen attempt')
        if attempt['status'] not in GM06_STATUSES:
            raise ContractError(location + '.status: undeclared enrichment status')
        properties = attempt['properties']
        if not isinstance(properties, dict):
            raise ContractError(location + '.properties: object required')
        if set(properties) != names:
            raise ContractError(location + '.properties: exactly the declared enrichment '
                                'properties are required; missing='
                                + str(sorted(names - set(properties))) + ' unknown='
                                + str(sorted(set(properties) - names)))
        for name in GM06_COMPARED:
            value = properties[name]
            if value is not None and (not isinstance(value, str) or not value):
                raise ContractError(location + f'.properties.{name}: absent or a non-empty '
                                               'string is required')
        for name in GM06_REFERENCE_ONLY + GM06_UNCOMPARED:
            if type(properties[name]) is not bool:
                raise ContractError(location + f'.properties.{name}: presence flag required')
        for name in GM06_PRODUCT_ONLY:
            value = properties[name]
            if value is not None and (not isinstance(value, str) or not value):
                raise ContractError(location + f'.properties.{name}: absent or a non-empty '
                                               'string is required')
            if side == 'reference' and value is not None:
                raise ContractError(location + f'.properties.{name}: product-only property on '
                                               'the reference side')
        if side == 'product' and any(properties[name] for name in GM06_REFERENCE_ONLY):
            raise ContractError(location + '.properties: reference-only property on the '
                                'product side')
        if attempt['status'] == 'success':
            if attempt['operation'] == 'enrich' and not properties['enriched_at']:
                raise ContractError(location + '.properties.enriched_at: a successful '
                                               'enrichment must be stamped')
        elif any(properties[name] is not None for name in GM06_COMPARED):
            raise ContractError(location + '.properties: a non-successful attempt writes no '
                                           'enrichment property')
        failure = attempt['failure']
        if side == 'reference':
            if failure is not None:
                raise ContractError(location + '.failure: the reference exposes no closed '
                                               'failure vocabulary')
        elif failure is not None and failure not in GM06_FAILURES:
            raise ContractError(location + '.failure: outside the closed failure vocabulary')
        elif failure is None and attempt['status'] not in ('success', 'not_found', 'skipped'):
            raise ContractError(location + '.failure: a failed attempt requires a declared '
                                           'failure')
        if declaration.get('expect_status') != attempt['status']:
            raise ContractError(location + '.status: differs from the frozen status')
        expected = declaration.get('expect_properties')
        if expected is None and declaration.get('replay_of') is None:
            expected = {name: None for name in GM06_COMPARED}
        if expected is not None:
            for name in GM06_COMPARED:
                if properties[name] != expected[name]:
                    raise ContractError(location + f'.properties.{name}: differs from the '
                                                   'frozen provider answer')
        if declaration.get('replay_of') is not None:
            replayed = compared.get(declaration['replay_of'])
            if replayed is None:
                raise ContractError(location + '.replay_of: the replayed attempt is absent')
            if {name: properties[name] for name in GM06_COMPARED} != replayed:
                raise ContractError(location + '.properties: a warm-cache replay must repeat '
                                               'the replayed properties')
        point = attempt['location']
        if point is not None:
            fields(point, {'latitude', 'longitude'}, {'latitude', 'longitude'},
                   location + '.location')
            latitude = number(point['latitude'], location + '.location.latitude', -90.0, 90.0)
            longitude = number(point['longitude'], location + '.location.longitude',
                               -180.0, 180.0)
            point = {'latitude': latitude, 'longitude': longitude}
        if attempt['operation'] == 'geocode':
            if (declaration.get('expect_location') is None) is not (point is None):
                raise ContractError(location + '.location: the declared geocode point is '
                                               'required')
            if point is not None:
                for name in ('latitude', 'longitude'):
                    if abs(point[name] - declaration['expect_location'][name]) > EPSILON:
                        raise ContractError(location + f'.location.{name}: differs from the '
                                                       'frozen provider answer')
        elif point is not None:
            raise ContractError(location + '.location: an enrichment attempt stores no point')
        compared[attempt['key']] = {name: properties[name] for name in GM06_COMPARED}
        attempts.append({'key': attempt['key'], 'operation': attempt['operation'],
                         'status': attempt['status'], 'properties': compared[attempt['key']],
                         'location': point})
    requests = section['attempt_requests']
    if not isinstance(requests, dict) or set(requests) != set(frozen):
        raise ContractError(path + '.attempt_requests: exactly the declared attempts are '
                                   'required')
    for key, value in requests.items():
        if type(value) is not int or value < 0:
            raise ContractError(f'{path}.attempt_requests.{key}: non-negative integer required')
        if value != frozen[key]['requests'][side]:
            raise ContractError(f'{path}.attempt_requests.{key}: differs from the declared '
                                f'{side} count')
    if frozen['enrich-replay']['replay_of'] is not None and requests['enrich-replay']:
        raise ContractError(path + '.attempt_requests.enrich-replay: a warm-cache replay '
                                   'issues no provider request')
    spacing = section['spacing']
    if not isinstance(spacing, dict) or set(spacing) != set(spec['spacing']):
        raise ContractError(path + '.spacing: exactly the declared providers are required')
    slack = number(spec['clock_slack_seconds'], 'fixture.gm06.clock_slack_seconds', 0.0, 1.0)
    for provider, declared in spec['spacing'].items():
        entry = spacing[provider]
        location = f'{path}.spacing.{provider}'
        fields(entry, {'requests', 'min_gap_seconds'}, {'requests', 'min_gap_seconds'},
               location)
        if type(entry['requests']) is not int or entry['requests'] < 0:
            raise ContractError(location + '.requests: non-negative integer required')
        expected = sum(requests[item['key']] for item in spec['attempts']
                       if item['provider'] == provider)
        if entry['requests'] != expected:
            raise ContractError(location + '.requests: differs from the recorded attempt total')
        if entry['requests'] < 2:
            if entry['min_gap_seconds'] is not None:
                raise ContractError(location + '.min_gap_seconds: absent below two requests')
            continue
        gap = number(entry['min_gap_seconds'], location + '.min_gap_seconds', 0.0, 3600.0)
        if gap < declared['rate_limit_seconds'] - slack:
            raise ContractError(location + '.min_gap_seconds: below the declared rate-limit '
                                           'spacing')
    _divergences(section, side, GM06_DIVERGENCES, path)
    return {'attempts': attempts}


def _ranking(value, path, allowed, limit=None):
    fields(value, {'query', 'results'}, {'query', 'results'}, path)
    if not isinstance(value['results'], list):
        raise ContractError(path + '.results: list required')
    if limit is not None and len(value['results']) > limit:
        raise ContractError(path + '.results: more results than the frozen limit')
    results, seen, previous = [], set(), None
    for index, item in enumerate(value['results']):
        location = f'{path}.results[{index}]'
        fields(item, set(GM07_RESULT_FIELDS), set(GM07_RESULT_FIELDS), location)
        if item['id'] not in allowed:
            raise ContractError(location + '.id: undeclared identity')
        if item['id'] in seen:
            raise ContractError(location + '.id: duplicate ranked identity')
        seen.add(item['id'])
        score = number(item['score'], location + '.score')
        if previous is not None and score > previous + EPSILON:
            raise ContractError(path + '.results: ranking order must be non-increasing')
        previous = score
        results.append({'id': item['id'], 'score': score})
    return {'query': value['query'], 'results': results}


def _gm07(document, fixture):
    spec, side = fixture['gm07'], document['side']
    section, path = document['gm07'], 'gm07'
    fields(section, {'entities', 'edges', 'vector', 'text', 'graph', 'context', 'isolation',
                     'divergences'},
           {'entities', 'edges', 'vector', 'text', 'graph', 'context', 'isolation',
            'divergences'}, path)
    entities = section['entities']
    if not isinstance(entities, list) or not all(isinstance(item, str) for item in entities):
        raise ContractError(path + '.entities: entity token list required')
    for token in entities:
        name, _, entity_type = token.partition('|')
        if entity_key(name, entity_type) != token:
            raise ContractError(path + '.entities: undeclared entity token')
    if len(set(entities)) != len(entities):
        raise ContractError(path + '.entities: duplicate entity inventory token')
    declared = {entity_key(item['name'], item['type']) for item in spec['entities']}
    if not declared <= set(entities):
        raise ContractError(path + '.entities: a declared seed is absent from the inventory')
    edges = []
    for index, edge in enumerate(section['edges']):
        location = f'{path}.edges[{index}]'
        fields(edge, set(RELATION_FIELDS), set(RELATION_FIELDS), location)
        for name in ('source', 'target'):
            if edge[name] not in entities:
                raise ContractError(location + f'.{name}: undeclared entity identity')
        if not isinstance(edge['type'], str) or not edge['type']:
            raise ContractError(location + '.type: non-empty string required')
        if sorted(edge['extra_columns']) != sorted(spec['relation_extra_columns']):
            raise ContractError(location + '.extra_columns: differs from the declared set')
        edges.append({'source': edge['source'], 'target': edge['target'], 'type': edge['type'],
                      'confidence': number(edge['confidence'], location + '.confidence')})
    observed = {(edge['source'], edge['target'], edge['type']) for edge in edges}
    for frozen in spec['edges']:
        if (frozen['source'], frozen['target'], frozen['type']) not in observed:
            raise ContractError(path + '.edges: the declared RELATED_TO edge is absent')
    vector = _ranking(section['vector'], path + '.vector', set(entities),
                      spec['vector']['limit'])
    if vector['query'] != spec['vector']['query']:
        raise ContractError(path + '.vector.query: differs from the frozen query')
    keys = {item['key'] for item in spec['messages']}
    text = _ranking(section['text'], path + '.text', keys, spec['text']['limit'])
    if text['query'] != spec['text']['query']:
        raise ContractError(path + '.text.query: differs from the frozen query')
    declared_pairs = {(frozen['source'], frozen['target']) for frozen in spec['edges']}
    graph = section['graph']
    fields(graph, {'source', 'results'}, {'source', 'results'}, path + '.graph')
    if graph['source'] != spec['graph']['source']:
        raise ContractError(path + '.graph.source: differs from the frozen source')
    for index, token in enumerate(graph['results']):
        if token not in entities:
            raise ContractError(f'{path}.graph.results[{index}]: undeclared entity identity')
        if token == graph['source']:
            raise ContractError(f'{path}.graph.results[{index}]: the source is not its own '
                                'neighbour')
        if (graph['source'], token) not in declared_pairs:
            raise ContractError(f'{path}.graph.results[{index}]: no declared RELATED_TO edge '
                                'reaches this neighbour')
    context = section['context']
    fields(context, {'query', 'document'}, {'query', 'document'}, path + '.context')
    if context['query'] != spec['context']['query']:
        raise ContractError(path + '.context.query: differs from the frozen query')
    if not isinstance(context['document'], str) or not context['document'].strip():
        raise ContractError(path + '.context.document: non-empty document required')
    positions = []
    for header in spec['context']['sections']:
        found = context['document'].find(header)
        if found < 0:
            raise ContractError(path + '.context.document: the declared context section order '
                                'is not reproduced')
        positions.append(found)
    if positions != sorted(positions):
        raise ContractError(path + '.context.document: the declared context section order is '
                            'not reproduced')
    isolation = section['isolation']
    fields(isolation, {'included'}, {'included'}, path + '.isolation')
    included = isolation['included']
    if not isinstance(included, list) or not all(isinstance(item, str) for item in included):
        raise ContractError(path + '.isolation.included: message key list required')
    for key in included:
        if key not in keys:
            raise ContractError(path + '.isolation.included: undeclared message identity')
        if next(item for item in spec['messages'] if item['key'] == key)['owner'] == \
                spec['scope']['owner']:
            raise ContractError(path + '.isolation.included: a same-owner message is not a '
                                'cross-owner leak')
    if sorted(included) != sorted(spec['isolation'][side + '_included']):
        raise ContractError(path + '.isolation.included: the declared cross-owner search result '
                            'differs')
    for item in spec['messages']:
        if item['owner'] == spec['scope']['other'] and item['text'] in context['document']:
            raise ContractError(path + '.context.document: a foreign-owner message leaked into '
                                'the context document')
    _divergences(section, side, GM07_DIVERGENCES, path)
    return {'entities': sorted(entities),
            'edges': sorted(edges, key=lambda item: (item['source'], item['type'],
                                                     item['target'])),
            'vector': vector, 'text': text,
            'graph': {'source': graph['source'], 'results': sorted(graph['results'])},
            'context': {'query': context['query'], 'document': context['document']}}


def canonical_gm_h4(document):
    if not isinstance(document, dict):
        raise ContractError('capture: object required')
    fields(document, {'schema', 'side', 'gm02', 'gm03', 'gm04', 'gm06', 'gm07'},
           {'schema', 'side', 'gm02', 'gm03', 'gm04', 'gm06', 'gm07'}, 'capture')
    if document['schema'] != SCHEMA:
        raise ContractError('capture.schema: unsupported capture')
    if document['side'] not in SIDES:
        raise ContractError('capture.side: unknown capture side')
    fixture = _fixture()
    if fixture['numeric_epsilon'] != EPSILON:
        raise ContractError('fixture.numeric_epsilon: differs from the declared epsilon')
    sections = {'gm02': _gm02(document, fixture), 'gm03': _gm03(document, fixture),
                'gm04': _gm04(document, fixture), 'gm06': _gm06(document, fixture),
                'gm07': _gm07(document, fixture)}
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
    return (key == 'similarity_score'
            or (key == 'confidence' and path.startswith('gm04.links'))
            or (key == 'score' and path.startswith(('gm07.vector', 'gm07.text'))))


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
        tolerance = SIMILARITY_EPSILON if key in ('similarity_score', 'score') else EPSILON
        if abs(float(left) - float(right)) > tolerance:
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
    for name in ('gm02', 'gm03', 'gm04', 'gm06', 'gm07'):
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
