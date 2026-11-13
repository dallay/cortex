"""Contratos completos GM-01, sin servicios externos."""
import copy
import unittest

from gm01_contract import canonical_conversation, compare_conversations, canonical_graph


class ConversationContractTests(unittest.TestCase):
    def conversation(self):
        return {'id': 'conversation-a', 'session_id': 'session-a', 'title': None,
                'created_at': '2026-11-02T10:00:00+00:00', 'updated_at': None,
                'metadata': {'id': 'keep-me'}, 'messages': []}

    def test_nonempty_messages_require_identity_reference_and_all_fields(self):
        value = self.conversation()
        value['messages'] = [{'content': 'silently incomplete'}]
        with self.assertRaises(ValueError):
            canonical_conversation(value)

    def test_graph_checks_dangling_edges_and_preserves_properties(self):
        graph = {'nodes': [{'key': 'c', 'labels': ['Conversation'], 'properties': {'session_id': 's'}}],
                 'edges': []}
        self.assertEqual(canonical_graph(graph), graph)
        graph['edges'].append({'source': 'c', 'target': 'missing', 'type': 'HAS_MESSAGE', 'properties': {}})
        with self.assertRaises(ValueError):
            canonical_graph(graph)

    def test_json_types_are_not_interchangeable(self):
        left, right = self.conversation(), self.conversation()
        left['metadata'] = {'flag': True}
        right['metadata'] = {'flag': 1}
        self.assertEqual(compare_conversations(left, right), ['conversation_fields'])
        left['metadata'] = {'number': 1}
        right['metadata'] = {'number': 1.0}
        self.assertEqual(compare_conversations(left, right), ['conversation_fields'])

    def test_full_message_comparison_reports_missing_fields_without_fabrication(self):
        from gm01_contract import compare_public_messages
        reference = {'id': 'a', 'conversation_id': 'c', 'role': 'user', 'content': 'hola',
                     'created_at': '2026-11-03T00:00:00Z', 'updated_at': None,
                     'tool_calls': None, 'embedding': None, 'metadata': {'flag': True}}
        rust = {'id': 'b', 'conversation_id': 'd', 'role': 'user', 'content': 'hola',
                'timestamp': '2026-11-03T01:00:00Z', 'embedding': None, 'metadata': {'flag': True}}
        self.assertEqual(compare_public_messages(reference, rust),
                         ['missing:tool_calls', 'missing:updated_at'])
        rust.update(tool_calls=None, updated_at=None)
        self.assertEqual(compare_public_messages(reference, rust), [])
        rust['metadata']['flag'] = 1
        self.assertIn('message_fields', compare_public_messages(reference, rust))

    def test_persisted_graph_normalizes_only_declared_generated_properties(self):
        from gm01_contract import normalize_persisted_graph
        graph = {'nodes': [{'key': 'c1', 'labels': ['Conversation'],
                           'properties': {'id': 'c1', 'created_at': '2026-11-03T00:00:00Z',
                                          'metadata': '{"id":"keep","flag":true}'}}], 'edges': []}
        result = normalize_persisted_graph(graph, {'c1': 'conversation'})
        self.assertEqual(result['nodes'][0]['properties']['metadata'], {'id': 'keep', 'flag': True})
        self.assertEqual(result['nodes'][0]['properties']['id'], 'conversation')
        graph['nodes'][0]['properties']['owner_scope'] = 'unexpected'
        self.assertEqual(normalize_persisted_graph(graph, {'c1': 'conversation'})['nodes'][0]
                         ['properties']['owner_scope'], 'unexpected')
        graph['edges'] = [{'source': 'c1', 'target': 'external', 'type': 'NEXT_MESSAGE', 'properties': {}}]
        with self.assertRaises(ValueError):
            normalize_persisted_graph(graph, {'c1': 'conversation'})

    def test_owner_scope_exception_is_exact_and_does_not_hide_other_properties(self):
        from gm01_contract import compare_scoped_graphs
        import copy
        left = {'nodes': [{'key': 'conversation', 'labels': ['Conversation'],
                           'properties': {'id': 'conversation'}}], 'edges': []}
        right = copy.deepcopy(left)
        right['nodes'][0]['properties']['owner_scope'] = 'unowned'
        self.assertEqual(compare_scoped_graphs(left, right), [])
        right['nodes'][0]['properties']['owner_scope'] = 'owned:other'
        self.assertIn('owner_scope', compare_scoped_graphs(left, right))
        del right['nodes'][0]['properties']['owner_scope']
        self.assertIn('owner_scope', compare_scoped_graphs(left, right))
        right['nodes'][0]['properties'].update(owner_scope='unowned', unexpected=True)
        self.assertEqual(compare_scoped_graphs(left, right), ['graph_properties'])
        left['nodes'][0]['properties']['owner_scope'] = 'unowned'
        self.assertIn('owner_scope', compare_scoped_graphs(left, right))

    def test_public_invariants_reject_duplicate_uuid_and_backwards_time(self):
        from gm01_contract import validate_public_invariants
        from uuid import uuid4
        from copy import deepcopy
        cid, mid = str(uuid4()), str(uuid4())
        value = {'id': cid, 'created_at': '2026-11-03T00:00:00Z', 'updated_at': None,
                 'messages': [{'id': mid, 'conversation_id': cid,
                               'timestamp': '2026-11-03T01:00:00Z', 'updated_at': None}]}
        self.assertEqual(validate_public_invariants(value), [])
        value['messages'].append(deepcopy(value['messages'][0]))
        self.assertIn('duplicate_message_id', validate_public_invariants(value))
        value['messages'][1]['id'] = str(uuid4())
        value['messages'][1]['timestamp'] = '2026-11-03T00:30:00Z'
        self.assertIn('message_time_order', validate_public_invariants(value))
        value['messages'][0]['conversation_id'] = str(uuid4())
        self.assertIn('conversation_reference', validate_public_invariants(value))
        value['id'] = 'invalid'
        self.assertIn('uuid', validate_public_invariants(value))

    def test_empty_conversation_preserves_all_public_fields(self):
        value = canonical_conversation(self.conversation())
        self.assertEqual(value['metadata'], {'id': 'keep-me'})
        self.assertEqual(value['messages'], [])
        self.assertEqual(set(value), set(self.conversation()))

    def test_generated_identity_and_time_may_differ_but_title_must_not(self):
        left = self.conversation()
        right = copy.deepcopy(left)
        right['id'] = 'conversation-b'
        right['created_at'] = '2026-11-03T10:00:00Z'
        self.assertEqual(compare_conversations(left, right), [])
        right['title'] = 'changed'
        self.assertEqual(compare_conversations(left, right), ['conversation_fields'])

    def test_unknown_or_missing_fields_are_not_silently_removed(self):
        value = self.conversation()
        value['unknown'] = True
        with self.assertRaises(ValueError):
            canonical_conversation(value)
        del value['unknown']
        del value['updated_at']
        with self.assertRaises(ValueError):
            canonical_conversation(value)

    def test_datetime_requires_zone_and_updated_at_cannot_precede_creation(self):
        value = self.conversation()
        value['created_at'] = '2026-11-02T10:00:00'
        with self.assertRaises(ValueError):
            canonical_conversation(value)
        value = self.conversation()
        value['updated_at'] = '2026-11-01T10:00:00Z'
        with self.assertRaises(ValueError):
            canonical_conversation(value)

    def test_nested_metadata_identifiers_and_timestamps_are_not_normalized(self):
        left = self.conversation()
        right = copy.deepcopy(left)
        right['metadata']['id'] = 'changed'
        self.assertEqual(compare_conversations(left, right), ['conversation_fields'])


if __name__ == '__main__':
    unittest.main()
