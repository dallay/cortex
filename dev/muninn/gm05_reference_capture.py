"""Captura GM-05 contra el oráculo fijado; nunca escribe en el producto.

Se ejecuta con el intérprete del oráculo (dependencias fijadas por hash) y las
fuentes verificadas de `agent-memory` y `agent-memory-tck`. El resultado es un
documento JSON canónico que el runner compara con la captura de Muninn.
"""

import asyncio
import json
import os
from pathlib import Path
import sys



def _required(name):
    value = os.environ.get(name)
    if not value:
        raise SystemExit(f'Falta la variable {name}')
    return value


def main():
    sources = Path(_required('GM05_SOURCES')).resolve()
    uri = _required('GM05_URI')
    password = _required('GM05_PASSWORD')
    output = Path(_required('GM05_OUTPUT')).resolve()
    sys.path[:0] = [str(sources / 'agent-memory/src'), str(sources / 'agent-memory-tck')]

    from neo4j import GraphDatabase
    from neo4j_agent_memory import MemoryClient
    from neo4j_agent_memory.config.settings import MemorySettings
    from neo4j_agent_memory.schema.models import EntityRef
    from tck.fixtures.mocks import MockEmbedder

    from gm05_adapters import GraphReader, ReferenceAdapter
    from gm05_capture import capture_gm05

    driver = GraphDatabase.driver(uri, auth=('neo4j', password))

    def query(cypher, parameters):
        with driver.session() as session:
            return session.run(cypher, **parameters).data()

    async def run():
        count = query('MATCH (n) RETURN count(n) AS count', {})[0]['count']
        if count != 0:
            raise SystemExit(f'La base del oráculo no está vacía ({count} nodos); no se mide')
        settings = MemorySettings(
            backend='bolt',
            neo4j={'uri': uri, 'username': 'neo4j', 'password': password},
            extraction={'extractor_type': 'none'}, resolution={'strategy': 'none'},
            enrichment={'enabled': False}, geocoding={'enabled': False},
        )
        client = MemoryClient(settings, embedder=MockEmbedder())
        await client.connect()
        try:
            adapter = ReferenceAdapter(client, GraphReader(query),
                                       entity_ref=lambda identifier: EntityRef(id=str(identifier)))
            document = await capture_gm05(adapter)
        finally:
            await client.close()
        components = query('CALL dbms.components() YIELD versions, edition '
                           'RETURN versions, edition', {})
        return document, components

    try:
        document, components = asyncio.run(run())
        payload = {
            'side': 'reference',
            'neo4j': components,
            'embedder': 'tck MockEmbedder (SHA256, 1536)',
            'capture': document,
        }
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(payload, ensure_ascii=False, indent=2, default=str) + '\n')
    finally:
        driver.close()
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
