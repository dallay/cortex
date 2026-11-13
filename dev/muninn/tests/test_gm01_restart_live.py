"""Regresiones de persistencia con conversaciones no vacías."""
import asyncio
from copy import deepcopy
import unittest
from unittest.mock import AsyncMock


class RestartLiveTests(unittest.TestCase):
    def test_restart_detects_lost_message_properties(self):
        from gm01_lifecycle import capture_live_restart
        adapter = AsyncMock()
        before = {'id': 'c', 'messages': [{'id': 'm', 'metadata': {'keep': True}}]}
        after = deepcopy(before)
        after['messages'][0]['metadata'] = {}
        adapter.get.side_effect = [before, after]
        adapter.graph.return_value = {'nodes': [], 'edges': []}
        result = asyncio.run(capture_live_restart(adapter, 'session', ['m']))
        self.assertIn('public_changed', result['errors'])
        adapter.reconnect.assert_awaited_once()

    def test_restart_detects_lost_graph_links(self):
        from gm01_lifecycle import capture_live_restart
        adapter = AsyncMock()
        adapter.get.return_value = {'id': 'c', 'messages': [{'id': 'm'}]}
        adapter.graph.side_effect = [{'nodes': ['c', 'm'], 'edges': ['has']},
                                     {'nodes': ['c', 'm'], 'edges': []}]
        result = asyncio.run(capture_live_restart(adapter, 'session', ['m']))
        self.assertIn('graph_changed', result['errors'])
