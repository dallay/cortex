"""Fixture controlado; no sustituye los tests ni corrige resultados nativos."""

import pytest_asyncio

from tck_runner import guarded_reset


_configuration = None


def configure(profile, token, password):
    global _configuration
    _configuration = (profile, token, password)


@pytest_asyncio.fixture(scope="session", loop_scope="session")
async def adapter():
    from neo4j_agent_memory import MemoryClient
    from neo4j_agent_memory.config.settings import MemorySettings
    from tck.fixtures.mocks import MockEmbedder
    from tck.reference.adapter import ReferenceAdapter

    if _configuration is None:
        raise ValueError("El adaptador solo puede usarse desde el runner aislado")
    profile, token, password = _configuration

    class ControlledAdapter(ReferenceAdapter):
        async def setup(self):
            settings = MemorySettings(
                backend="bolt", neo4j={"uri": "bolt://neo4j:7687", "username": "neo4j", "password": password},
                extraction={"extractor_type": "none"}, resolution={"strategy": "none"},
                enrichment={"enabled": False}, geocoding={"enabled": False},
            )
            self._client = MemoryClient(settings, embedder=MockEmbedder())
            await self._client.connect()
            await self.clear_all_data()

        async def clear_all_data(self):
            if self._client is None or self._client._client is None:
                raise ValueError("No hay conexión a la base sintética protegida")
            graph = self._client._client
            await guarded_reset(graph.execute_read, graph.execute_write, token)

        async def delete_message(self, message_id):
            if profile == "native-delete":
                return await self._client.short_term.delete_message(message_id)
            return await super().delete_message(message_id)

    instance = ControlledAdapter(neo4j_uri="bolt://neo4j:7687", neo4j_password=password)
    try:
        await instance.setup()
        yield instance
    finally:
        await instance.teardown()
