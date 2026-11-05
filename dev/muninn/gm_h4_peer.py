"""Internal H4 peer, launched ONLY by the source-bound Dory runner.

One peer runs per captured side inside the H4 oracle image. It binds no port,
opens no network connection except its own Neo4j alias, downloads nothing and
writes exactly one capture document plus one runtime identity. The reference
side drives the pinned `agent-memory` client; the product side starts the
source-bound bridge on container loopback.
"""
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import sysconfig

from gm08_peer import _post, consume_binary, require_empty, start_bridge, stop_bridge
from gm_h4_capture import capture_gm02, capture_gm04, frozen_fixture
from gm_h4_contract import SCHEMA, SIDES
from gm_h4_extract import capture_gm03
from reference import verify_source_tree

MODEL_RELATIVE = Path('en_core_web_sm') / 'en_core_web_sm-3.8.0'


def validate_side(value):
    if value not in SIDES:
        raise ValueError('unknown capture side')
    return value


def capture_document(side, gm02, gm03, gm04):
    return {'schema': SCHEMA, 'side': validate_side(side), 'gm02': gm02, 'gm03': gm03,
            'gm04': gm04}


def model_path():
    """The pinned model installed beside the interpreter; never a download."""
    path = Path(sysconfig.get_path('purelib')) / MODEL_RELATIVE
    if not path.is_dir():
        raise ValueError('installed spaCy model is absent')
    return str(path)


def reference_entity(entity):
    return {'id': str(entity.id), 'name': entity.name, 'type': entity.type,
            'subtype': entity.subtype, 'canonical_name': entity.canonical_name}


def reference_deduplication(result):
    matched = getattr(result, 'matched_entity_id', None)
    return {'is_duplicate': bool(result.is_duplicate), 'action': result.action,
            'matched_entity_id': None if matched is None else str(matched),
            'similarity_score': float(result.similarity_score),
            'match_type': result.match_type}


class ReferenceAdapter:
    """Drive the pinned client; the runner never projects product data here."""

    def __init__(self, client, driver):
        self.client, self.driver = client, driver

    def query(self, statement, parameters):
        with self.driver.session() as session:
            return session.run(statement, **parameters).data()

    async def call(self, operation, **arguments):
        if operation == 'add_message':
            from neo4j_agent_memory.schema.models import EntityRef
            mentions = [EntityRef(**item) for item in arguments['explicit_mentions']]
            extract = arguments['extract_entities']
            message = await self.client.short_term.add_message(
                arguments['session_id'], arguments['role'], arguments['content'],
                extraction_mode=arguments['extraction_mode'], explicit_mentions=mentions,
                extract_entities=True if extract is None else extract,
                extract_relations=arguments['extract_relations'],
                generate_embedding=arguments['generate_embedding'])
            return {'id': str(message.id), 'content': message.content}
        if operation == 'add_entity':
            entity, deduplication = await self.client.long_term.add_entity(
                arguments['name'], arguments['entity_type'],
                resolve=arguments['resolve'], deduplicate=arguments['deduplicate'],
                generate_embedding=True, geocode=False, enrich=False)
            return {'entity': reference_entity(entity),
                    'deduplication': reference_deduplication(deduplication)}
        raise ValueError('unsupported operation')


class ProductAdapter:
    """Drive the source-bound bridge wire; no native product object is read."""

    def __init__(self, url, driver):
        self.url, self.driver = url, driver

    def query(self, statement, parameters):
        with self.driver.session() as session:
            return session.run(statement, **parameters).data()

    async def call(self, operation, **arguments):
        return await asyncio.to_thread(_post, f'{self.url}/{operation}', arguments)


def reference_client(uri, password, model):
    from neo4j_agent_memory import MemoryClient
    from neo4j_agent_memory.config.settings import MemorySettings
    from tck.fixtures.mocks import MockEmbedder
    settings = MemorySettings(
        backend='bolt',
        neo4j={'uri': uri, 'username': 'neo4j', 'password': password},
        extraction={'extractor_type': 'spacy', 'spacy_model': model},
        resolution={'strategy': 'none'},
        enrichment={'enabled': False},
        geocoding={'enabled': False},
    )
    return MemoryClient(settings, embedder=MockEmbedder())


def gm03_section(side, texts, model):
    """The two stages over the frozen texts, each in its own process."""
    if side == 'product':
        import spacy
        import spacy_service
        pipeline = spacy.load(model)

        def extract(text):
            return spacy_service.extract({'text': text}, pipeline)['entities']
        return capture_gm03(texts, extract, side)
    from neo4j_agent_memory.extraction.spacy_extractor import SpacyEntityExtractor
    extractor = SpacyEntityExtractor(model=model)

    def extract(text):
        result = asyncio.run(extractor.extract(text))
        return [entity.model_dump(mode='json') for entity in result.entities]
    return capture_gm03(texts, extract, side)


def main():
    side = validate_side(os.environ['GM_H4_SIDE'])
    sources = Path('/references')
    integrity = json.loads(Path(__file__).with_name('source-integrity.json').read_text())
    verify_source_tree(sources, {key: integrity[key]
                                 for key in ('agent-memory', 'agent-memory-tck')})
    sys.path[:0] = [str(sources / 'agent-memory/src'), str(sources / 'agent-memory-tck')]
    uri, password = os.environ['NAM_NEO4J__URI'], os.environ['NAM_NEO4J__PASSWORD']
    model = model_path()
    from neo4j import GraphDatabase
    driver = GraphDatabase.driver(uri, auth=('neo4j', password))
    product = None
    try:
        with driver.session() as session:
            require_empty(session.run('MATCH (n) RETURN count(n) AS count').data())
        if side == 'product':
            binary = Path('/target/muninn')
            consume_binary('/built/muninn', binary, os.environ['GM_H4_BINARY_SHA256'])
            product, url = start_bridge(binary, uri, password)
            adapter = ProductAdapter(url, driver)
        else:
            client = reference_client(uri, password, model)
            asyncio.run(client.connect())
            adapter = ReferenceAdapter(client, driver)

        async def capture():
            gm02 = await capture_gm02(adapter)
            gm04 = await capture_gm04(adapter)
            return gm02, gm04

        try:
            gm02, gm04 = asyncio.run(capture())
        finally:
            if side == 'reference':
                asyncio.run(adapter.client.close())
        gm03 = gm03_section(side, frozen_fixture()['gm03']['texts'], model)
        document = capture_document(side, gm02, gm03, gm04)
        components = adapter.query('CALL dbms.components() YIELD versions, edition '
                                   'RETURN versions, edition', {})
        Path('/out/' + side + '.json').write_text(
            json.dumps(document, indent=2, allow_nan=False) + '\n')
        Path('/out/' + side + '-runtime.json').write_text(json.dumps({
            'side': side,
            'python': sys.version,
            'spacy': importlib.metadata.version('spacy'),
            'model': importlib.metadata.version('en_core_web_sm'),
            'model_path': Path(model).name,
            'neo4j': components,
            'embedding': 'tck MockEmbedder SHA256 1536' if side == 'reference'
            else 'hash 1536 (provider=hash)',
            'extraction': 'pinned SpacyEntityExtractor' if side == 'reference'
            else 'dev/muninn/spacy_service.py',
        }, indent=2) + '\n')
        return 0
    finally:
        if product is not None:
            stop_bridge(product)
        driver.close()


if __name__ == '__main__':
    raise SystemExit(main())
