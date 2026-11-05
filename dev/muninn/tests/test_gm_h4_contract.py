"""GM-02/03/04 frozen comparator rules; no Docker, provider, model or oracle.

Every document here is synthetic contract unit data. Nothing in this file is a
reference measurement, a capture of the pinned server or an accuracy result.
"""
import asyncio
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))

FIXTURE = json.loads((BASE / 'fixtures/gm_h4_v1.json').read_text())
AUTO_TEXT = FIXTURE['gm02']['scenarios'][0]['text']


def key(name, entity_type):
    return name + '|' + entity_type


def spans(text, items):
    """Declared synthetic mentions: (name, type, subtype) resolved onto offsets."""
    result = []
    for name, entity_type, subtype in items:
        start = text.index(name)
        result.append({'name': name, 'type': entity_type, 'subtype': subtype,
                       'start_pos': start, 'end_pos': start + len(name),
                       'confidence': 0.85, 'extractor': 'spacy'})
    return sorted(result, key=lambda item: item['start_pos'])


GM03_ENTITIES = {
    'auto': spans(AUTO_TEXT, [('Alice', 'PERSON', None),
                              ('Acme Corporation', 'ORGANIZATION', None),
                              ('Paris', 'LOCATION', 'GEOPOLITICAL'),
                              ('4 May 2024', 'EVENT', 'DATE')]),
    'unicode': spans(FIXTURE['gm03']['texts'][1]['text'],
                     [('Zoe Q. Moritz', 'PERSON', None),
                      ('Zurich', 'LOCATION', 'GEOPOLITICAL')]),
    'mixed': spans(FIXTURE['gm03']['texts'][2]['text'],
                   [('Tokyo', 'LOCATION', 'GEOPOLITICAL'),
                    ('12 March 2025', 'EVENT', 'DATE')]),
    'empty': [],
    'whitespace': [],
}

GM02_ENTITIES = [
    {'key': key('Dana Reyes', 'PERSON'), 'name': 'Dana Reyes', 'type': 'PERSON', 'subtype': None},
    {'key': key('Alice', 'PERSON'), 'name': 'Alice', 'type': 'PERSON', 'subtype': None},
    {'key': key('Acme Corporation', 'ORGANIZATION'), 'name': 'Acme Corporation',
     'type': 'ORGANIZATION', 'subtype': None},
    {'key': key('Paris', 'LOCATION'), 'name': 'Paris', 'type': 'LOCATION',
     'subtype': 'GEOPOLITICAL'},
    {'key': key('4 May 2024', 'EVENT'), 'name': '4 May 2024', 'type': 'EVENT', 'subtype': 'DATE'},
    {'key': key('Olive Turner', 'PERSON'), 'name': 'Olive Turner', 'type': 'PERSON',
     'subtype': None},
]
AUTO_MENTIONS = [(name, entity_type) for name, entity_type, _ in
                 [('Alice', 'PERSON', None), ('Acme Corporation', 'ORGANIZATION', None),
                  ('Paris', 'LOCATION', None), ('4 May 2024', 'EVENT', None)]]

GM04_SEEDS = FIXTURE['gm04']['seeds']
GM04_ATTEMPTS = [
    ('prefix-1', 'none', 0.0, None, key('Gm H4 Cypress Candidate', 'PERSON')),
    ('prefix-2', 'flagged', 0.87, key('Gm H4 Candidate Boreal', 'OBJECT'),
     key('Gm H4 Cypress Candidate', 'OBJECT')),
    ('prefix-3', 'merged', 0.91, key('Gm H4 Candidate Boreal', 'LOCATION'),
     key('Gm H4 Candidate Boreal', 'LOCATION')),
    ('exact', 'merged', 1.0, key('Gm H4 Candidate Alpine', 'OBJECT'),
     key('Gm H4 Candidate Alpine', 'OBJECT')),
    ('exact-repeat', 'merged', 1.0, key('Gm H4 Candidate Alpine', 'OBJECT'),
     key('Gm H4 Candidate Alpine', 'OBJECT')),
    ('no-candidate', 'none', 0.0, None, key('Gm H4 Distant Sigma', 'ORGANIZATION')),
]
GM04_ENTITIES = [{'key': key(seed['name'], seed['type']), 'name': seed['name'],
                  'type': seed['type']} for seed in GM04_SEEDS] + [
    {'key': key('Gm H4 Cypress Candidate', 'PERSON'), 'name': 'Gm H4 Cypress Candidate',
     'type': 'PERSON'},
    {'key': key('Gm H4 Cypress Candidate', 'OBJECT'), 'name': 'Gm H4 Cypress Candidate',
     'type': 'OBJECT'},
    {'key': key('Gm H4 Distant Sigma', 'ORGANIZATION'), 'name': 'Gm H4 Distant Sigma',
     'type': 'ORGANIZATION'},
]


def document(side='reference'):
    """Synthetic capture with exactly the declared per-side invariants."""
    messages = [{'key': scenario['key'], 'scenario': scenario['key'], 'mode': scenario['mode'],
                 'content': scenario['text']} for scenario in FIXTURE['gm02']['scenarios']]
    mentions = []

    def add(message, name, entity_type, confidence, start, end, mode='auto'):
        provenance = None
        if side == 'product':
            provenance = {
                'derived': mode == 'auto',
                'extractor': 'spacy' if mode == 'auto' else None,
                'support': 1,
                'source_message_ids': [message],
                'evidence': next(item['content'] for item in messages if item['key'] == message),
            }
        mentions.append({'message': message, 'entity': key(name, entity_type),
                         'confidence': confidence, 'start_pos': start, 'end_pos': end,
                         'provenance': provenance})

    for message in ('auto', 'auto-replay'):
        for name, entity_type in AUTO_MENTIONS:
            start = AUTO_TEXT.index(name)
            add(message, name, entity_type, 0.85, start, start + len(name))
    add('explicit', 'Dana Reyes', 'PERSON', 1.0, None, None, mode='explicit')
    add('explicit', 'Olive Turner', 'PERSON', 1.0, None, None, mode='explicit')
    attempts = [{'key': attempt, 'name': name, 'entity_type': FIXTURE['gm04']['attempts'][index]
                 ['entity_type'], 'action': action, 'is_duplicate': action != 'none',
                 'matched': matched, 'similarity_score': score,
                 'match_type': 'fuzzy' if action != 'none' else None,
                 'entity': entity, 'canonical_name': entity.split('|')[0]}
                for index, (attempt, action, score, matched, entity) in enumerate(GM04_ATTEMPTS)
                for name in [FIXTURE['gm04']['attempts'][index]['name']]]
    return {
        'schema': 'gm_h4-capture-v1',
        'side': side,
        'gm02': {'messages': messages, 'entities': [dict(item) for item in GM02_ENTITIES],
                 'mentions': sorted(mentions, key=lambda item: (item['message'], item['entity'])),
                 'relations': []},
        'gm03': {'texts': [dict(item) for item in FIXTURE['gm03']['texts']],
                 'entities': [dict({'text': text}, **entity)
                              for text, items in GM03_ENTITIES.items() for entity in items]},
        'gm04': {'seeds': [{'key': key(seed['name'], seed['type']), 'name': seed['name'],
                            'type': seed['type']} for seed in GM04_SEEDS],
                 'attempts': attempts,
                 'entities': [dict(item) for item in GM04_ENTITIES],
                 'links': [{'source': key('Gm H4 Cypress Candidate', 'OBJECT'),
                            'target': key('Gm H4 Candidate Boreal', 'OBJECT'),
                            'status': 'pending', 'match_type': 'fuzzy', 'confidence': 0.87}]},
    }


class ModuleTests(unittest.TestCase):
    def setUp(self):
        from gm_h4_contract import compare_gm_h4, declarations_sha256, EPSILON
        self.compare = compare_gm_h4
        self.sha = declarations_sha256
        self.epsilon = EPSILON

    def test_declarations_and_epsilon_are_frozen_before_measurement(self):
        digest = self.sha()
        self.assertRegex(digest, r'^[0-9a-f]{64}$')
        self.assertEqual(digest, self.sha())
        self.assertEqual(self.epsilon, 1e-9)
        self.assertLessEqual(self.epsilon, 1e-9)
        self.assertEqual(FIXTURE['numeric_epsilon'], self.epsilon)
        from gm_h4_contract import THRESHOLDS
        self.assertEqual(THRESHOLDS, FIXTURE['thresholds'])
        self.assertIsInstance(self.compare(self_pair()[0], self_pair()[1])['declarations_sha256'],
                              str)

    def test_valid_pair_passes_without_error(self):
        verdict = self.compare(document('reference'), document('product'))
        self.assertEqual(verdict['comparison'], 'pass', verdict['errors'])
        self.assertEqual(verdict['errors'], [])
        self.assertEqual(verdict['diffs'], [])
        self.assertEqual([item['item'] for item in verdict['items']],
                         ['gm02', 'gm03', 'gm04'])
        self.assertTrue(all(item['verdict'] == 'pass' for item in verdict['items']))
        self.assertEqual(verdict['epsilon'], 1e-9)

    def test_tolerance_above_the_declared_epsilon_is_rejected(self):
        for value in (1e-6, 1e-8, 0):
            verdict = self.compare(document('reference'), document('product'), epsilon=value)
            self.assertEqual(verdict['comparison'], 'fail')
            self.assertIn('epsilon: only the declared 1e-09 tolerance is authorized',
                          verdict['errors'])

    def test_equal_but_corrupt_captures_fail_closed(self):
        reference, product = document('reference'), document('product')
        for capture in (reference, product):
            capture['gm02']['entities'].append(
                {'key': key('Skip Probe', 'ORGANIZATION'), 'name': 'Skip Probe',
                 'type': 'ORGANIZATION', 'subtype': None})
            capture['gm02']['mentions'].append(
                {'message': 'skip', 'entity': key('Skip Probe', 'ORGANIZATION'),
                 'confidence': 0.85, 'start_pos': 0, 'end_pos': 9,
                 'provenance': None if capture['side'] == 'reference' else {
                     'derived': True, 'extractor': 'spacy', 'support': 1,
                     'source_message_ids': ['skip'], 'evidence': AUTO_TEXT}})
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('declared absent entity' in error for error in verdict['errors']))
        reference, product = document('reference'), document('product')
        for capture in (reference, product):
            capture['gm03']['entities'] = [entity for entity in capture['gm03']['entities']
                                           if entity['text'] != 'auto']
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('shorter than the GM-02 auto mentions' in error
                            for error in verdict['errors']))

    def test_span_label_and_offset_mismatch_fail(self):
        reference, product = document('reference'), document('product')
        product["gm03"]["entities"][2]["subtype"] = None
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('.subtype' in diff['path'] for diff in verdict['diffs']))
        reference, product = document('reference'), document('product')
        product['gm03']['entities'][0]['type'] = 'OBJECT'
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any(error.startswith('gm03.entities[0].type')
                            for error in verdict['errors']))
        reference, product = document('reference'), document('product')
        product['gm03']['entities'][0]['start_pos'] += 1
        product['gm03']['entities'][0]['end_pos'] += 1
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('span' in error for error in verdict['errors']))

    def test_confidence_must_be_the_pinned_constant(self):
        for value in (0.86, 0.84, 1.0):
            reference, product = document('reference'), document('product')
            product['gm03']['entities'][0]['confidence'] = value
            verdict = self.compare(reference, product)
            self.assertEqual(verdict['comparison'], 'fail')
            self.assertTrue(any('0.85' in error for error in verdict['errors']))

    def test_reordered_candidate_selection_fails(self):
        reference, product = document('reference'), document('product')
        product['gm04']['attempts'][1]['matched'] = key('Gm H4 Candidate Alpine', 'OBJECT')
        product['gm04']['links'][0]['target'] = key('Gm H4 Candidate Alpine', 'OBJECT')
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('.matched' in diff['path'] for diff in verdict['diffs']))

    def test_undeclared_identity_fails(self):
        reference, product = document('reference'), document('product')
        product['gm02']['entities'][0]['key'] = 'entity-0'
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('derive the key' in error for error in verdict['errors']))
        reference, product = document('reference'), document('product')
        product['gm04']['attempts'][1]['matched'] = key('Undeclared Candidate', 'OBJECT')
        product['gm04']['links'][0]['target'] = key('Undeclared Candidate', 'OBJECT')
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('undeclared matched identity' in error
                            for error in verdict['errors']))
        reference, product = document('reference'), document('product')
        product['gm02']['mentions'][0]['message'] = 'not-a-scenario'
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('undeclared message identity' in error
                            for error in verdict['errors']))

    def test_product_provenance_is_required_and_reference_provenance_forbidden(self):
        reference, product = document('reference'), document('product')
        product['gm02']['mentions'][0]['provenance']['support'] = 2
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        reference, product = document('reference'), document('product')
        del product['gm02']['mentions'][0]['provenance']['evidence']
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('declared Muninn columns' in error for error in verdict['errors']))
        reference, product = document('reference'), document('product')
        reference['gm02']['mentions'][0]['provenance'] = {'support': 1}
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('reference exposes no additional MENTIONS column' in error
                            for error in verdict['errors']))

    def test_explicit_links_only_supplied_references(self):
        reference, product = document('reference'), document('product')
        for capture in (reference, product):
            capture['gm02']['mentions'].append(
                {'message': 'explicit', 'entity': key('Unlinked Unknown Name', 'PERSON'),
                 'confidence': 1.0, 'start_pos': None, 'end_pos': None,
                 'provenance': None if capture['side'] == 'reference' else {
                     'derived': False, 'extractor': None, 'support': 1,
                     'source_message_ids': ['explicit'],
                     'evidence': 'Approved synthetic entity references.'}})
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('mentions differ from the fixture declaration' in error
                            for error in verdict['errors']))

    def test_auto_scenario_must_produce_the_declared_minimum(self):
        reference, product = document('reference'), document('product')
        for capture in (reference, product):
            capture['gm02']['mentions'] = [mention for mention in capture['gm02']['mentions']
                                           if mention['message'] != 'auto']
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('the declared minimum is 1' in error for error in verdict['errors']))

    def test_one_capture_side_per_pair_is_required(self):
        verdict = self.compare(document('product'), document('product'))
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertIn('capture.side: two different server sides are required',
                      verdict['errors'])

    def test_scores_use_the_declared_epsilon_and_never_a_wider_one(self):
        reference, product = document('reference'), document('product')
        product['gm04']['attempts'][1]['similarity_score'] = 0.87 + 5e-10
        self.assertEqual(self.compare(reference, product)['comparison'], 'pass')
        product['gm04']['attempts'][1]['similarity_score'] = 0.87 + 1e-6
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('similarity_score' in diff['path'] for diff in verdict['diffs']))

    def test_mentions_confidence_is_exact_not_tolerated(self):
        reference, product = document('reference'), document('product')
        product['gm02']['mentions'][0]['confidence'] = 0.85 + 1e-12
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')

    def test_same_as_links_must_match_the_flagged_attempts(self):
        reference, product = document('reference'), document('product')
        for capture in (reference, product):
            capture['gm04']['links'] = []
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'fail')
        self.assertTrue(any('pending SAME_AS' in error for error in verdict['errors']))
        reference, product = document('reference'), document('product')
        for capture in (reference, product):
            capture['gm04']['attempts'][1]['action'] = 'none'
            capture['gm04']['attempts'][1]['is_duplicate'] = False
            capture['gm04']['attempts'][1]['matched'] = None
            capture['gm04']['attempts'][1]['similarity_score'] = 0.0
            capture['gm04']['attempts'][1]['match_type'] = None
            capture['gm04']['links'] = []
        verdict = self.compare(reference, product)
        self.assertEqual(verdict['comparison'], 'pass', verdict['errors'])


class FakeAdapter:
    """Side-specific stand-in that emulates the graph the capture reads.

    It is not a server, a model or a measurement: it exists so the frozen
    capture operations and the frozen contract are proven to agree on one
    document shape end to end.
    """

    def __init__(self, side):
        self.side = side
        self.entities = {}
        self.messages = {}
        self.mentions = []
        self.links = []
        self.serial = 0

    def _identifier(self):
        self.serial += 1
        return f'{self.side}-{self.serial:04d}'

    def _entity(self, name, entity_type, subtype=None):
        for identifier, value in self.entities.items():
            if value['name'] == name and value['type'] == entity_type:
                return identifier, value
        identifier = self._identifier()
        self.entities[identifier] = {'id': identifier, 'name': name, 'type': entity_type,
                                    'subtype': subtype, 'canonical_name': name}
        return identifier, self.entities[identifier]

    def query(self, statement, parameters):
        if '-[r:MENTIONS]->' in statement:
            return [{'message': item['message'], 'entity': item['entity'],
                     'properties': dict(item['properties'])} for item in self.mentions]
        if '-[r:SAME_AS]->' in statement:
            return [{'source': item['source'], 'target': item['target'],
                     'properties': dict(item['properties'])} for item in self.links]
        if '-[r:RELATED_TO]->' in statement:
            return []
        return [{'properties': dict(value)} for value in self.entities.values()]

    def _provenance(self, message, mode):
        if self.side != 'product':
            return {}
        return {'derived': mode == 'auto', 'extractor': 'spacy' if mode == 'auto' else None,
                'support': 1, 'source_message_ids': [message], 'evidence': self.content[message]}

    def _mention(self, message, entity, confidence, start, end, mode):
        self.mentions.append({'message': message, 'entity': entity,
                              'properties': {'confidence': confidence, 'start_pos': start,
                                             'end_pos': end, **self._provenance(message, mode)}})

    async def call(self, operation, **arguments):
        if operation == 'add_entity':
            if arguments.get('resolve'):
                return self._resolved(arguments['name'], arguments['entity_type'])
            identifier, value = self._entity(arguments['name'], arguments['entity_type'])
            return {'entity': {'id': identifier, **value},
                    'deduplication': {'is_duplicate': False, 'action': 'none',
                                      'matched_entity_id': None, 'similarity_score': 0.0,
                                      'match_type': None}}
        if operation == 'add_message':
            message = self._identifier()
            self.messages[message] = arguments['content']
            self.content = self.messages
            mode = arguments['extraction_mode']
            if mode == 'auto':
                for entity in GM03_ENTITIES['auto']:
                    identifier, _ = self._entity(entity['name'], entity['type'],
                                                 entity['subtype'])
                    self._mention(message, identifier, entity['confidence'],
                                  entity['start_pos'], entity['end_pos'], mode)
            if mode == 'explicit':
                for reference in arguments['explicit_mentions']:
                    if 'id' in reference:
                        self._mention(message, reference['id'], 1.0, None, None, mode)
                    elif 'type' in reference:
                        identifier, _ = self._entity(reference['name'], reference['type'])
                        self._mention(message, identifier, 1.0, None, None, mode)
            return {'id': message, 'content': arguments['content']}
        raise AssertionError(operation)

    def _resolved(self, name, entity_type):
        attempt = next(item for item in GM04_ATTEMPTS
                       if item[0] == next(entry['key'] for entry in FIXTURE['gm04']['attempts']
                                         if entry['name'] == name
                                         and entry['entity_type'] == entity_type))
        _, action, score, matched, entity = attempt
        matched_id = None
        if matched is not None:
            matched_name, matched_type = matched.split('|')
            matched_id = next(identifier for identifier, value in self.entities.items()
                              if value['name'] == matched_name and value['type'] == matched_type)
        if action == 'merged':
            stored = self.entities[matched_id]
        else:
            existing = [identifier for identifier, value in self.entities.items()
                        if value['name'] + '|' + value['type'] == entity]
            stored = (self.entities[existing[0]] if existing
                      else self._entity(*entity.split('|'))[1])
            if action == 'flagged':
                self.links.append({'source': stored['id'], 'target': matched_id,
                                   'properties': {'status': 'pending', 'match_type': 'fuzzy',
                                                  'confidence': score}})
        return {'entity': dict(stored, id=stored['id']),
                'deduplication': {'is_duplicate': action != 'none', 'action': action,
                                  'matched_entity_id': matched_id,
                                  'similarity_score': score,
                                  'match_type': 'fuzzy' if action != 'none' else None}}


class CaptureTests(unittest.TestCase):
    def build(self, side):
        from gm_h4_capture import capture_gm02, capture_gm04
        from gm_h4_extract import capture_gm03
        adapter = FakeAdapter(side)
        gm02 = asyncio.run(capture_gm02(adapter))
        gm04 = asyncio.run(capture_gm04(adapter))
        texts = [dict(item) for item in FIXTURE['gm03']['texts']]
        by_text = {item['text']: GM03_ENTITIES[item['key']] for item in texts}
        gm03 = capture_gm03(texts, lambda text: [dict(item) for item in by_text[text]], side)
        return {'schema': 'gm_h4-capture-v1', 'side': side, 'gm02': gm02, 'gm03': gm03,
                'gm04': gm04}

    def test_capture_and_contract_agree_end_to_end(self):
        from gm_h4_contract import compare_gm_h4
        reference, product = self.build('reference'), self.build('product')
        self.assertTrue(reference['gm02']['mentions'])
        self.assertTrue(product['gm02']['mentions'][0]['provenance'])
        verdict = compare_gm_h4(reference, product)
        self.assertEqual(verdict['comparison'], 'pass', verdict['errors'])
        self.assertEqual([item['verdict'] for item in verdict['items']],
                         ['pass', 'pass', 'pass'])

    def test_capture_maps_every_identity_to_a_declared_token(self):
        document = self.build('product')
        tokens = {entity['key'] for entity in document['gm02']['entities']}
        self.assertIn(key('Alice', 'PERSON'), tokens)
        self.assertNotIn(key('Unlinked Unknown Name', 'PERSON'), tokens)
        self.assertTrue(all(mention['entity'] in tokens
                            for mention in document['gm02']['mentions']))
        self.assertEqual([attempt['key'] for attempt in document['gm04']['attempts']],
                         [item['key'] for item in FIXTURE['gm04']['attempts']])

    def test_undeclared_graph_identity_is_refused(self):
        from gm_h4_capture import capture_gm02
        from gm_h4_contract import ContractError
        adapter = FakeAdapter('reference')
        original = adapter.query
        adapter.query = lambda statement, parameters: (
            [{'message': 'm-unknown', 'entity': 'e', 'properties': {}}]
            if '-[r:MENTIONS]->' in statement else original(statement, parameters))
        with self.assertRaisesRegex(ContractError, 'undeclared identity'):
            asyncio.run(capture_gm02(adapter))


def self_pair():
    return document('reference'), document('product')


class CommandTests(unittest.TestCase):
    def invoke(self, *arguments):
        return subprocess.run([sys.executable, '-B', str(BASE / 'gm_h4_contract.py'),
                               *arguments], text=True, capture_output=True)

    def write(self, directory, name, value):
        path = Path(directory) / name
        path.write_text(value if isinstance(value, str) else json.dumps(value))
        return path

    def test_cli_reports_pass_with_exact_streams(self):
        from gm_h4_contract import compare_gm_h4
        with tempfile.TemporaryDirectory(prefix='gm-h4-contract-') as temp:
            expected = self.write(temp, 'reference.json', document('reference'))
            actual = self.write(temp, 'product.json', document('product'))
            result = self.invoke('--expected', str(expected), '--actual', str(actual))
            verdict = compare_gm_h4(json.loads(expected.read_text()),
                                    json.loads(actual.read_text()))
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout, json.dumps(verdict, sort_keys=True) + '\n')
            self.assertEqual(result.stderr, '')

    def test_cli_reports_failure_with_exact_streams(self):
        from gm_h4_contract import compare_gm_h4
        with tempfile.TemporaryDirectory(prefix='gm-h4-contract-') as temp:
            reference = document('reference')
            product = document('product')
            product['gm03']['entities'][0]['start_pos'] += 2
            expected = self.write(temp, 'reference.json', reference)
            actual = self.write(temp, 'product.json', product)
            result = self.invoke('--expected', str(expected), '--actual', str(actual))
            verdict = compare_gm_h4(reference, product)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, json.dumps(verdict, sort_keys=True) + '\n')
            self.assertEqual(result.stderr, '')

    def test_cli_fails_closed_on_unreadable_capture(self):
        with tempfile.TemporaryDirectory(prefix='gm-h4-contract-') as temp:
            expected = self.write(temp, 'reference.json', '{not json')
            actual = self.write(temp, 'product.json', document('product'))
            result = self.invoke('--expected', str(expected), '--actual', str(actual))
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, json.dumps(
                {'comparison': 'fail', 'diffs': [],
                 'errors': ['capture: unreadable or invalid JSON'], 'items': []},
                sort_keys=True) + '\n')
            self.assertEqual(result.stderr, '')

    def test_cli_offers_no_tolerance_bypass(self):
        result = self.invoke('--expected', 'a', '--actual', 'b', '--epsilon', '1e-3')
        self.assertEqual(result.returncode, 2)
        self.assertEqual(result.stdout, '')
        self.assertIn('unrecognized arguments: --epsilon 1e-3', result.stderr)


if __name__ == '__main__':
    unittest.main()
