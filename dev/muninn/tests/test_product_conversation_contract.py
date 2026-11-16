"""Pruebas suplementarias del contrato Platinum de Muninn; no son TCK normativo."""
import os
import unittest
from uuid import UUID


@unittest.skipUnless(
    os.environ.get('MUNINN_PRODUCT_CONVERSATION_CONTRACT_TEST') == '1',
    'requiere fixture Docker nueva y binario de producto explícito',
)
class ProductConversationContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from product_chain_fixture import ProductFixture

        binary = os.environ['MUNINN_PRODUCT_BINARY']
        cls.fixture = ProductFixture(binary)
        cls.fixture.__enter__()

    @classmethod
    def tearDownClass(cls):
        cls.fixture.__exit__(None, None, None)

    def test_create_conversation_returns_fresh_uuid_and_requested_user_id(self):
        first = self.fixture.call('create_conversation', user_id='alice')
        second = self.fixture.call('create_conversation', user_id='alice')

        first_id = UUID(first['id'])
        second_id = UUID(second['id'])
        self.assertNotEqual(first_id, second_id)
        self.assertEqual(first['user_id'], 'alice')
        self.assertEqual(second['user_id'], 'alice')
