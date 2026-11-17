"""Escenarios GM-01: pruebas del protocolo de ciclo de vida."""
import unittest
from gm01_lifecycle import check_chain, classify_delete_error, capture_lifecycle
from unittest.mock import AsyncMock


class LifecycleTests(unittest.TestCase):
    def test_empty_creation_is_checked_before_any_deletion(self):
        import asyncio
        reference, rust = AsyncMock(), AsyncMock()
        reference.create.return_value = {'id': 'a', 'messages': []}
        rust.create.return_value = {'id': 'b', 'messages': ['unexpected']}
        with self.assertRaises(ValueError):
            asyncio.run(capture_lifecycle(reference, rust))
        reference.delete.assert_not_called()
        rust.delete.assert_not_called()

    def test_delete_bridge_boolean_is_not_a_transport_tuple(self):
        import asyncio
        from unittest.mock import Mock
        from gm01_lifecycle import LifecycleAdapter
        fixture = Mock(spec=['assert_live', '_request'])
        fixture._request.return_value = (200, {'deleted': True})
        adapter = LifecycleAdapter(Mock(), fixture=fixture)
        self.assertIs(asyncio.run(adapter.delete('message-id')), True)

    def test_adapter_records_safe_checkpoint_before_bridge_failure(self):
        import asyncio
        from unittest.mock import Mock
        from gm01_lifecycle import LifecycleAdapter
        checkpoint = []
        fixture = Mock(spec=['assert_live', 'call'])
        fixture.call.side_effect = RuntimeError('sensitive')
        adapter = LifecycleAdapter(Mock(), fixture=fixture, checkpoint=checkpoint.append)
        with self.assertRaises(RuntimeError):
            asyncio.run(adapter.create('session'))
        self.assertEqual(checkpoint, ['rust:create'])

    def test_restart_rust_replaces_process_without_closing_database(self):
        import asyncio
        from unittest.mock import Mock, patch
        from gm01_lifecycle import LifecycleAdapter
        database = Mock()
        fixture = Mock(spec=['close', 'artifact', 'private', 'start'])
        adapter = LifecycleAdapter(database, fixture=fixture)
        asyncio.run(adapter.reconnect())
        fixture.close.assert_called_once_with(preserve_database=True)
        fixture.start.assert_called_once()
        database.close.assert_not_called()

    def test_lifecycle_rejects_same_missing_append_in_both_backends(self):
        import asyncio
        from copy import deepcopy
        conversation = {'id': 'c', 'session_id': 's', 'title': None, 'messages': [],
                        'metadata': {}, 'created_at': '2026-11-03T00:00:00Z', 'updated_at': None}
        def backend():
            adapter = AsyncMock()
            adapter.create.return_value = deepcopy(conversation)
            adapter.get.return_value = deepcopy(conversation)
            count = iter(range(12))
            async def add(session, content, metadata):
                return {'id': str(next(count)), 'conversation_id': 'c', 'role': 'user',
                        'content': content, 'metadata': metadata}
            adapter.add.side_effect = add
            adapter.graph.return_value = {'nodes': [], 'edges': []}
            adapter.snapshot.return_value = {'members': [], 'first': [], 'last': [], 'next': []}
            adapter.delete.return_value = True
            adapter.clear.return_value = None
            adapter.invalid_role.return_value = 'invalid_input'
            adapter.remaining.return_value = {'conversations': 0, 'messages': 0}
            return adapter
        report = asyncio.run(capture_lifecycle(backend(), backend()))
        self.assertIn('first:0:read:message_count', report['errors'])
        self.assertIn('first:1:read:message_count', report['errors'])
        self.assertEqual(report['status'], 'incomplete')

    def test_missing_delete_requires_false_and_unchanged_state(self):
        import asyncio
        from gm01_lifecycle import capture_missing_delete
        adapter = AsyncMock()
        adapter.delete.return_value = False
        adapter.get.return_value = {'messages': []}
        adapter.snapshot.side_effect = [{'members': []}, {'members': ['unexpected']}]
        case = asyncio.run(capture_missing_delete(adapter, 's', 'invalid-id'))
        self.assertEqual(case['errors'], ['state_changed'])

    def test_clear_case_rejects_orphan_message_even_when_read_is_empty(self):
        import asyncio
        from gm01_lifecycle import capture_clear
        adapter = AsyncMock()
        adapter.clear.return_value = None
        adapter.get.return_value = {'messages': []}
        adapter.remaining.return_value = {'conversations': 0, 'messages': 1}
        case = asyncio.run(capture_clear(adapter, 's', ['m']))
        self.assertIn('remaining_nodes', case['errors'])
        self.assertIn('restart_remaining_nodes', case['errors'])

    def test_clear_session_requires_no_content_transport_response(self):
        import asyncio
        from unittest.mock import Mock
        from gm01_lifecycle import LifecycleAdapter
        fixture = Mock(spec=['assert_live', '_request'])
        fixture._request.return_value = (204, None)
        adapter = LifecycleAdapter(Mock(), fixture=fixture)
        self.assertIsNone(asyncio.run(adapter.clear('session')))
        fixture._request.assert_called_once_with('clear_session', {'session_id': 'session'})
        fixture._request.return_value = (500, {})
        with self.assertRaises(RuntimeError):
            asyncio.run(adapter.clear('session'))

    def test_public_reference_read_does_not_reconstruct_message_fields(self):
        import asyncio
        from unittest.mock import Mock
        from gm01_lifecycle import LifecycleAdapter
        raw = {'id': 'c', 'messages': [{'conversation_id': None, 'embedding': [0.5]}]}
        client = Mock()
        client.short_term.get_conversation = AsyncMock(return_value=Mock())
        client.short_term.get_conversation.return_value.model_dump.return_value = raw
        adapter = LifecycleAdapter(Mock(), reference=client)
        self.assertEqual(asyncio.run(adapter.get('s')), raw)

    def test_append_checks_expected_input_and_json_types(self):
        from gm01_lifecycle import check_append
        message = {'id': 'm', 'conversation_id': 'c', 'role': 'user',
                   'content': '¡Hola 世界!', 'metadata': {'flag': True}}
        self.assertEqual(check_append(message, 'c', '¡Hola 世界!', {'flag': True}), [])
        self.assertEqual(check_append(message, 'c', 'wrong', {'flag': True}), ['content'])
        self.assertEqual(check_append(message, 'c', '¡Hola 世界!', {'flag': 1}), ['metadata'])
        message['conversation_id'] = None
        self.assertEqual(check_append(message, 'c', '¡Hola 世界!', {'flag': True}), ['conversation_id'])

    def test_delete_oracle_rejects_survivor_mutation_and_orphan_target(self):
        from gm01_lifecycle import check_delete_effects
        from copy import deepcopy
        before = {'nodes': [{'key': 'c', 'labels': ['Conversation'], 'properties': {}},
                            {'key': 'a', 'labels': ['Message'], 'properties': {'content': 'A'}},
                            {'key': 'b', 'labels': ['Message'], 'properties': {'content': 'B'}}],
                  'edges': [{'source': 'c', 'target': 'a', 'type': 'HAS_MESSAGE', 'properties': {}},
                            {'source': 'c', 'target': 'b', 'type': 'HAS_MESSAGE', 'properties': {}},
                            {'source': 'c', 'target': 'a', 'type': 'FIRST_MESSAGE', 'properties': {}},
                            {'source': 'a', 'target': 'b', 'type': 'NEXT_MESSAGE', 'properties': {}}]}
        after = {'nodes': deepcopy([before['nodes'][0], before['nodes'][2]]),
                 'edges': [{'source': 'c', 'target': 'b', 'type': 'HAS_MESSAGE', 'properties': {}},
                           {'source': 'c', 'target': 'b', 'type': 'FIRST_MESSAGE', 'properties': {}}]}
        public = {'messages': [{'id': 'a', 'content': 'A'}, {'id': 'b', 'content': 'B'}]}
        read = {'messages': [{'id': 'b', 'content': 'B'}]}
        self.assertEqual(check_delete_effects(before, after, public, read, 'a'), [])
        after['nodes'][1]['properties']['content'] = 'changed'
        self.assertIn('graph', check_delete_effects(before, after, public, read, 'a'))
        after['nodes'][1]['properties']['content'] = 'B'
        after['nodes'].append(deepcopy(before['nodes'][1]))
        self.assertIn('graph', check_delete_effects(before, after, public, read, 'a'))
        read['messages'][0]['content'] = 'changed'
        self.assertIn('public', check_delete_effects(before, after, public, read, 'a'))

    def test_append_adapter_forwards_mixed_role(self):
        import asyncio
        from unittest.mock import Mock
        from gm01_lifecycle import LifecycleAdapter, check_append
        fixture = Mock(spec=['assert_live', 'call'])
        fixture.call.return_value = {'id': 'm', 'conversation_id': 'c', 'content': 'answer',
                                     'metadata': {}, 'role': 'assistant'}
        result = asyncio.run(LifecycleAdapter(Mock(), fixture=fixture).add(
            's', 'answer', {}, role='assistant'))
        self.assertEqual(fixture.call.call_args.kwargs['role'], 'assistant')
        self.assertEqual(check_append(result, 'c', 'answer', {}, role='assistant'), [])

    def test_interleaved_sessions_detects_cross_session_leak(self):
        import asyncio
        from uuid import uuid4
        from copy import deepcopy
        from gm01_lifecycle import capture_interleaved
        class Backend:
            def __init__(self, leak=True, reference=False):
                self.reference = reference
                self.leak = leak
                self.conversations = {}
                self.roles = []
            async def create(self, session):
                self.conversations[session] = {'id': str(uuid4()), 'session_id': session,
                    'created_at': '2026-11-03T00:00:00Z', 'updated_at': None,
                    'title': None, 'metadata': {}, 'messages': []}
                return deepcopy(self.conversations[session])
            async def add(self, session, content, metadata, *, role):
                message = {'id': str(uuid4()), 'conversation_id': self.conversations[session]['id'],
                    'role': role, 'content': content, 'metadata': metadata, 'embedding': None,
                    'tool_calls': None, 'updated_at': None, 'timestamp': '2026-11-03T01:00:00Z'}
                if self.reference:
                    message['created_at'] = message.pop('timestamp')
                self.roles.append(role)
                self.conversations[session]['messages'].append(message)
                return deepcopy(message)
            async def get(self, session):
                result = deepcopy(self.conversations[session])
                if self.leak and session.endswith('-b') and self.conversations['gm01-interleaved-a']['messages']:
                    result['messages'].append(deepcopy(self.conversations['gm01-interleaved-a']['messages'][0]))
                return result
            async def reconnect(self):
                pass
        left, right = Backend(), Backend()
        case = asyncio.run(capture_interleaved(left, right))
        self.assertTrue(any('message_count' in error for error in case['errors']))
        self.assertEqual(set(left.roles), {'system', 'user', 'assistant', 'tool'})
        valid = asyncio.run(capture_interleaved(Backend(leak=False, reference=True), Backend(leak=False)))
        self.assertEqual(valid['errors'], [])
        self.assertEqual(len(valid['checkpoints']), 7)

    def test_invalid_role_requires_input_error_and_unchanged_graph(self):
        import asyncio
        from gm01_lifecycle import capture_invalid_role
        adapter = AsyncMock()
        adapter.invalid_role.return_value = 'internal_error'
        adapter.get.return_value = {'messages': []}
        adapter.graph.return_value = {'nodes': [], 'edges': []}
        case = asyncio.run(capture_invalid_role(adapter, 's', []))
        self.assertIn('category', case['errors'])
        adapter.invalid_role.return_value = 'invalid_input'
        self.assertEqual(asyncio.run(capture_invalid_role(adapter, 's', []))['errors'], [])

    def test_chain_checks_all_edges_and_membership(self):
        graph = {'members': ['a', 'b', 'c'], 'first': ['a'], 'last': ['c'],
                 'next': [['a', 'b'], ['b', 'c']]}
        self.assertEqual(check_chain(graph, ['a', 'b', 'c']), [])
        graph['next'].append(['a', 'c'])
        self.assertEqual(check_chain(graph, ['a', 'b', 'c']), ['next'])

    def test_singleton_chain_and_empty_chain(self):
        self.assertEqual(check_chain({'members': ['a'], 'first': ['a'], 'last': ['a'], 'next': []}, ['a']), [])
        self.assertEqual(check_chain({'members': [], 'first': [], 'last': [], 'next': []}, []), [])

    def test_only_connected_node_constraint_is_approved(self):
        self.assertEqual(classify_delete_error('Neo.ClientError.Schema.ConstraintValidationFailed'),
                         'GM01-DEV-001')
        self.assertIsNone(classify_delete_error('Neo.ClientError.Security.Unauthorized'))
        self.assertIsNone(classify_delete_error('RuntimeError'))


if __name__ == '__main__':
    unittest.main()
