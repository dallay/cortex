"""El grafo debe corresponder a entradas conocidas, no solo al otro backend."""
import unittest


class ExpectedGraphTests(unittest.TestCase):
    def test_missing_properties_and_links_reject(self):
        from gm01_contract import validate_expected_graph
        graph = {'nodes': [{'key': 'c', 'labels': ['Conversation'], 'properties': {'id': 'c'}}], 'edges': []}
        public = {'id': 'c', 'session_id': 's', 'messages': [{'id': 'm', 'role': 'user', 'content': 'hello', 'metadata': {}}]}
        self.assertTrue(validate_expected_graph(graph, public))

    def test_valid_but_different_timestamp_rejects(self):
        from gm01_contract import validate_expected_graph
        public = {'id': 'c', 'session_id': 's', 'created_at': '2026-11-03T00:00:00Z',
                  'updated_at': '2026-11-03T00:00:00Z', 'messages': []}
        graph = {'nodes': [{'key': 'c', 'labels': ['Conversation'], 'properties': {
            'id': 'c', 'session_id': 's', 'created_at': '2026-11-04T00:00:00Z',
            'updated_at': '2026-11-03T00:00:00Z'}}], 'edges': []}
        self.assertIn('conversation_created_at', validate_expected_graph(graph, public))
