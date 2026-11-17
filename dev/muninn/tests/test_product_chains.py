"""Cadenas físicas del producto actual en una fixture Docker nueva, opt-in."""
import os
from pathlib import Path
import unittest


@unittest.skipUnless(os.environ.get('MUNINN_PRODUCT_CHAIN_TEST') == '1',
                     'requiere fixture Docker nueva y binario de producto explícito')
class ProductChains(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from product_chain_fixture import ProductFixture
        binary = os.environ['MUNINN_PRODUCT_BINARY']
        cls.fixture = ProductFixture(binary)
        cls.fixture.__enter__()

    @classmethod
    def tearDownClass(cls):
        cls.fixture.__exit__(None, None, None)

    def add(self, session, owner, content):
        return self.fixture.call('add_message', session_id=session, user_identifier=owner,
                                 role='user', content=content)

    def assert_chain(self, session, owner, ids):
        result = self.fixture.call('get_conversation', session_id=session, user_identifier=owner)
        self.assertEqual([m['id'] for m in result['messages']], ids)
        conversation = result['id']
        first = self.fixture.query('MATCH (c:Conversation {id:$id})-[:FIRST_MESSAGE]->(m) RETURN m.id AS id', id=conversation)
        self.assertEqual([r['id'] for r in first], ids[:1])
        edges = self.fixture.query('MATCH (c:Conversation {id:$id})-[:HAS_MESSAGE]->(a)-[:NEXT_MESSAGE]->(b) RETURN a.id AS a,b.id AS b', id=conversation)
        self.assertEqual({(r['a'], r['b']) for r in edges}, set(zip(ids, ids[1:])))
        members = self.fixture.query('MATCH (c:Conversation {id:$id})-[:HAS_MESSAGE]->(m) RETURN m.id AS id', id=conversation)
        self.assertEqual({r['id'] for r in members}, set(ids))

    def test_delete_positions_append_restart_and_scopes(self):
        for position in ('first', 'middle', 'last', 'single'):
            with self.subTest(position=position):
                session = 'product-chain-' + position
                owner = 'chain-owner'
                count = 1 if position == 'single' else 3
                ids = [self.add(session, owner, f'{position}-{i}')['id'] for i in range(count)]
                other = self.add(session, 'other-owner', 'must remain')['id']
                target = ids[{'first': 0, 'middle': 1, 'last': 2, 'single': 0}[position]]
                # Wrong owner must neither expose nor physically delete the message.
                self.fixture.call('delete_message', message_id=target, user_identifier='other-owner')
                self.assert_chain(session, owner, ids)
                self.fixture.call('delete_message', message_id=target, user_identifier=owner)
                self.assertEqual(self.fixture.query('MATCH (m:Message {id:$id}) RETURN count(m) AS count', id=target)[0]['count'], 0)
                remaining = [item for item in ids if item != target]
                self.assert_chain(session, owner, remaining)
                remaining.append(self.add(session, owner, 'appended')['id'])
                self.assert_chain(session, owner, remaining)
                self.fixture.stop_bridge()
                self.fixture.start_bridge()
                self.assert_chain(session, owner, remaining)
                self.assertEqual(self.fixture.query(
                    'MATCH (m:Message {id:$id}) RETURN count(m) AS count', id=target,
                )[0]['count'], 0)
                self.assert_chain(session, 'other-owner', [other])
        # Anonymous scope is independent of a named owner for the same session.
        anon = self.add('product-anonymous', None, 'anonymous')['id']
        named = self.add('product-anonymous', 'named', 'named')['id']
        self.fixture.call('clear_session', session_id='product-anonymous', user_identifier=None)
        self.assertEqual(self.fixture.query('MATCH (m:Message {id:$id}) RETURN count(m) AS count', id=anon)[0]['count'], 0)
        self.assert_chain('product-anonymous', 'named', [named])
