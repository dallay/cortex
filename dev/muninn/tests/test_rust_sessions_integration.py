"""Regresiones HTTP/grafo; solo en la fixture efímera Docker del spike."""

import os
from pathlib import Path
import unittest
from uuid import uuid4


@unittest.skipUnless(os.environ.get("MUNINN_SESSION_TESTS") == "1", "Requiere la fixture Docker efímera")
class SessionIntegrationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        import httpx
        from neo4j import AsyncGraphDatabase

        self.token = Path('/run/secrets/bridge_token').read_text().strip()
        password = Path('/run/secrets/neo4j_auth').read_text().strip().split('/', 1)[1]
        self.client = httpx.AsyncClient(base_url='http://bridge:8080', trust_env=False, timeout=12)
        self.driver = AsyncGraphDatabase.driver('bolt://neo4j:7687', auth=('neo4j', password))
        await self.call('setup', {})
        await self.call('clear_all_data', {})

    async def asyncTearDown(self):
        await self.call('clear_all_data', {})
        await self.driver.close()
        await self.client.aclose()

    async def call(self, method, body, status=200):
        response = await self.client.post('/' + method, json=body, headers={'Authorization': 'Bearer ' + self.token})
        self.assertEqual(response.status_code, status, f'{method}: estado inesperado')
        return response.json()

    async def add(self, content):
        return await self.call('add_message', {'session_id': 'append-after-delete', 'role': 'user', 'content': content})

    async def test_append_after_tail_and_all_deleted_repairs_actual_chain(self):
        first = await self.add('first')
        tail = await self.add('tail')
        self.assertTrue((await self.call('delete_message', {'message_id': tail['id']}))['deleted'])
        replacement = await self.add('replacement')
        rows, _, _ = await self.driver.execute_query(
            'MATCH (c:Conversation {scope:$scope,session_id:$session})-[:FIRST_MESSAGE]->(a:Message)-[:NEXT_MESSAGE]->(b:Message) RETURN a.id AS first,b.id AS tail',
            scope='muninn-rust-spike', session='append-after-delete', database_='neo4j')
        self.assertEqual([r.data() for r in rows], [{'first': first['id'], 'tail': replacement['id']}])
        for message in (first, replacement):
            await self.call('delete_message', {'message_id': message['id']})
        fresh = await self.add('fresh')
        rows, _, _ = await self.driver.execute_query(
            'MATCH (:Conversation {scope:$scope,session_id:$session})-[:FIRST_MESSAGE]->(m:Message) RETURN m.id AS id',
            scope='muninn-rust-spike', session='append-after-delete', database_='neo4j')
        self.assertEqual([r['id'] for r in rows], [fresh['id']])

    async def test_new_routes_reject_unauthorized_scope_and_invalid_identifiers(self):
        for method, body in (('list_sessions', {}), ('clear_session', {'session_id':'valid'}),
                             ('delete_message', {'message_id':str(uuid4())})):
            response = await self.client.post('/' + method, json=body)
            self.assertEqual(response.status_code, 401)
            await self.call(method, {**body, 'scope':'other'}, 400)
        await self.call('list_sessions', {'limit':1001}, 400)
        await self.call('delete_message', {'message_id':'not-a-uuid'}, 400)
        for session in ('', self.token):
            await self.call('clear_session', {'session_id':session}, 400)

    async def test_revoked_guard_and_foreign_nodes_cannot_be_treated_as_empty_success(self):
        message = await self.add('protected')
        await self.driver.execute_query(
            'MATCH (g:MuninnBridgeGuard {scope:$scope}) SET g.owner=$revoked',
            scope='muninn-rust-spike', revoked='revoked-session-test', database_='neo4j')
        try:
            for method, body in (('list_sessions', {}), ('clear_session', {'session_id':'append-after-delete'}),
                                 ('delete_message', {'message_id':message['id']})):
                await self.call(method, body, 409)
        finally:
            import hashlib
            await self.driver.execute_query('MATCH (g:MuninnBridgeGuard {scope:$scope}) SET g.owner=$owner',
                scope='muninn-rust-spike', owner=hashlib.sha256(self.token.encode()).hexdigest(), database_='neo4j')
        foreign_id = str(uuid4())
        await self.driver.execute_query('CREATE (c:Conversation {scope:$foreign,session_id:$session})-[:HAS_MESSAGE]->(:Message {scope:$foreign,id:$id})',
            foreign='muninn-session-test-foreign', session='append-after-delete', id=foreign_id, database_='neo4j')
        try:
            self.assertFalse((await self.call('delete_message', {'message_id':foreign_id}))['deleted'])
            await self.call('clear_session', {'session_id':'append-after-delete'})
            rows, _, _ = await self.driver.execute_query('MATCH (n {scope:$foreign}) RETURN count(n) AS total',
                foreign='muninn-session-test-foreign', database_='neo4j')
            self.assertEqual(rows[0]['total'], 2)
        finally:
            await self.driver.execute_query('MATCH (n {scope:$foreign}) DETACH DELETE n',
                foreign='muninn-session-test-foreign', database_='neo4j')


if __name__ == '__main__':
    unittest.main()
