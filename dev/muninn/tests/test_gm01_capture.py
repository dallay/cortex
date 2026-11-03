"""Captura GM-01 sin servicios ni dependencias externas."""

import copy
from datetime import datetime, timezone
from types import SimpleNamespace
import unittest
from uuid import UUID

from gm01_differential import CaptureError, compare


class ReferenceDouble:
    def __init__(self):
        self.short_term = self
        self.calls = []
        self.conversations = {}

    async def add_message(self, session, role, content, **options):
        self.calls.append((session, role, content, copy.deepcopy(options)))
        if session not in self.conversations:
            self.conversations[session] = SimpleNamespace(
                id=UUID(int=len(self.conversations) + 1), session_id=session, messages=[])
        conversation = self.conversations[session]
        message = SimpleNamespace(
            id=UUID(int=10 + len(self.calls)), conversation_id=conversation.id,
            role=SimpleNamespace(value=role), content=content,
            metadata=copy.deepcopy(options['metadata']),
            created_at=datetime(2026, 11, 1, 12, 0, len(self.calls), tzinfo=timezone.utc))
        conversation.messages.append(message)
        return message

    async def get_conversation(self, session):
        return self.conversations[session]


class CaptureTests(unittest.IsolatedAsyncioTestCase):
    async def test_readback_must_match_written_scenario(self):
        from gm01_capture import ReferenceAdapter, capture_gm01

        def mutate(conversation, change):
            if change == 'session':
                conversation.session_id += '-wrong'
            elif change == 'drop-duplicate':
                conversation.messages.pop()
            elif change == 'reorder':
                conversation.messages[0], conversation.messages[1] = conversation.messages[1], conversation.messages[0]
            elif change == 'role':
                conversation.messages[0].role.value = 'tool'
            elif change == 'content':
                conversation.messages[0].content += 'changed'
            elif change == 'metadata':
                conversation.messages[0].metadata['nested']['tags'][1] = True

        for session in ('a', 'b'):
            for change in ('session', 'drop-duplicate', 'reorder', 'role', 'content', 'metadata'):
                if session == 'b' and change in ('drop-duplicate', 'reorder'):
                    continue

                class ChangedRead(ReferenceDouble):
                    async def get_conversation(self, requested):
                        result = await super().get_conversation(requested)
                        if requested == session:
                            mutate(result, change)
                        return result

                with self.subTest(session=session, change=change), self.assertRaises(CaptureError):
                    await capture_gm01(ReferenceAdapter(ChangedRead()), ('a', 'b'))

    async def test_incomplete_contractual_bridge_fixture_is_not_a_capture(self):
        from gm01_capture import RustBridgeAdapter, capture_gm01
        from dev.muninn.tests.test_gm01_differential import capture
        source = capture('rust')
        client = BridgeDouble({c['session_id']: c for c in source['conversations']})

        class ReadOnlyFixtureAdapter:
            async def add_message(self, *args):
                pass

            async def get_conversation(self, session):
                return await RustBridgeAdapter(client).get_conversation(session)

        with self.assertRaises(CaptureError):
            await capture_gm01(ReadOnlyFixtureAdapter(), ('sesion-0', 'sesion-1'))

    async def test_invalid_sessions_fail_before_writes(self):
        from gm01_capture import ReferenceAdapter, capture_gm01
        for sessions in (('same', 'same'), ('one',), ('a', 'b', 'c'), ('', 'b'), (None, 'b'), 'ab'):
            client = ReferenceDouble()
            with self.subTest(sessions=sessions), self.assertRaises(ValueError):
                await capture_gm01(ReferenceAdapter(client), sessions)
            self.assertEqual(client.calls, [])

    async def test_invalid_returned_capture_is_not_emitted(self):
        from gm01_capture import ReferenceAdapter, capture_gm01

        class BrokenRead(ReferenceDouble):
            async def get_conversation(self, session):
                result = await super().get_conversation(session)
                result.messages[0].metadata = None
                return result

        with self.assertRaises(CaptureError):
            await capture_gm01(ReferenceAdapter(BrokenRead()), ('a', 'b'))

    async def test_backend_error_propagates_without_fallback(self):
        from gm01_capture import ReferenceAdapter, capture_gm01
        failure = RuntimeError('backend refused')

        class BrokenReference(ReferenceDouble):
            async def add_message(self, *args, **kwargs):
                raise failure

        for adapter in (ReferenceAdapter(BrokenReference()),):
            with self.assertRaises(RuntimeError) as caught:
                await capture_gm01(adapter, ('a', 'b'))
            self.assertIs(caught.exception, failure)

    async def test_missing_reference_field_is_not_invented(self):
        from gm01_capture import ReferenceAdapter, capture_gm01

        class MissingField(ReferenceDouble):
            async def get_conversation(self, session):
                result = await super().get_conversation(session)
                del result.messages[0].conversation_id
                return result

        with self.assertRaises(AttributeError):
            await capture_gm01(ReferenceAdapter(MissingField()), ('a', 'b'))

    async def test_reference_scenario_and_complete_capture(self):
        from gm01_capture import ReferenceAdapter, capture_gm01
        client = ReferenceDouble()
        result = await capture_gm01(ReferenceAdapter(client), ('test-a', 'test-b'))
        self.assertEqual(compare(result, result)['comparison'], 'pass')
        self.assertEqual([c['session_id'] for c in result['conversations']], ['test-a', 'test-b'])
        self.assertEqual([call[1] for call in client.calls],
                         ['system', 'user', 'assistant', 'user', 'user', 'user', 'assistant'])
        first = result['conversations'][0]
        self.assertEqual([m['content'] for m in first['messages']],
                         ['Instrucción GM-01', 'Hola, Muñinn 🧠\nsegunda línea', '',
                          'x' * 10001, 'duplicado', 'duplicado'])
        for actual, source in zip(first['messages'], client.conversations['test-a'].messages):
            self.assertEqual(actual, {'id': str(source.id), 'conversation_id': str(source.conversation_id),
                                     'role': source.role.value, 'content': source.content,
                                     'metadata': source.metadata, 'timestamp': source.created_at.isoformat()})
        for call in client.calls:
            self.assertEqual(call[3], {
                'metadata': {'id': 'conservar', 'timestamp': 'conservar', 'nested': {'tags': ['ñ', 1]}},
                'extract_entities': False, 'extract_relations': False,
                'generate_embedding': False, 'extraction_mode': 'skip'})


class BridgeDouble:
    def __init__(self, conversations):
        self.conversations = conversations
        self.calls = []

    def call(self, method, **arguments):
        self.calls.append((method, copy.deepcopy(arguments)))
        if method == 'get_conversation':
            return copy.deepcopy(self.conversations[arguments['session_id']])
        return {'id': 'returned-add-id'}


class RustCaptureTests(unittest.IsolatedAsyncioTestCase):
    async def test_writes_block_before_any_client_call(self):
        from gm01_capture import RustBridgeAdapter, capture_gm01
        client = BridgeDouble({})
        adapter = RustBridgeAdapter(client)
        with self.assertRaisesRegex(CaptureError, 'embeddings'):
            await adapter.add_message('a', 'user', 'ñ', {})
        self.assertEqual(client.calls, [])
        with self.assertRaisesRegex(CaptureError, 'embeddings'):
            await capture_gm01(adapter, ('a', 'b'))
        self.assertEqual(client.calls, [])

    async def test_current_wire_fails_without_inventing_conversation_id(self):
        from gm01_capture import RustBridgeAdapter
        conversation = {'id': 'rust-c', 'session_id': 'test-a', 'title': None,
                        'created_at': '2026-11-01T12:00:00+00:00', 'updated_at': None,
                        'messages': [{'id': 'rust-m', 'role': 'user', 'content': 'ñ',
                                      'timestamp': '2026-11-01T12:00:00+00:00',
                                      'metadata': {}, 'embedding': None}]}
        client = BridgeDouble({'test-a': conversation})
        with self.assertRaisesRegex(CaptureError, 'conversation_id'):
            await RustBridgeAdapter(client).get_conversation('test-a')
        self.assertNotIn('conversation_id', conversation['messages'][0])
        self.assertEqual(client.calls, [('get_conversation', {'session_id': 'test-a'})])

    async def test_complete_bridge_read_preserves_payload_and_order_only(self):
        from gm01_capture import RustBridgeAdapter
        from dev.muninn.tests.test_gm01_differential import capture
        source = capture('rust')
        client = BridgeDouble({c['session_id']: c for c in source['conversations']})
        adapter = RustBridgeAdapter(client)
        result = [await adapter.get_conversation(c['session_id']) for c in source['conversations']]
        self.assertEqual(result, source['conversations'])
        self.assertEqual([method for method, _ in client.calls], ['get_conversation', 'get_conversation'])


if __name__ == '__main__':
    unittest.main()
