"""Internal H4 peer, launched ONLY by the source-bound Dory runner.

One peer runs per captured side inside the H4 oracle image. It binds no port,
opens no network connection except its own Neo4j alias, downloads nothing and
writes exactly one capture document plus one runtime identity. The reference
side drives the pinned `agent-memory` client; the product side starts the
source-bound bridge on container loopback. Every provider answer is a recorded
fixture served locally: no Wikipedia, Diffbot or Nominatim endpoint is
reachable from this process.
"""
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import sysconfig
import time

from gm08_peer import _post, consume_binary, require_empty, start_bridge, stop_bridge
from gm_h4_capture import (ENTITY_STATE, capture_gm02, capture_gm04, capture_gm06,
                           frozen_fixture)
from gm_h4_contract import SCHEMA, SIDES
from gm_h4_extract import capture_gm03
from gm_h4_search import capture_gm07
from reference import verify_source_tree

MODEL_RELATIVE = Path('en_core_web_sm') / 'en_core_web_sm-3.8.0'


def validate_side(value):
    if value not in SIDES:
        raise ValueError('unknown capture side')
    return value


def capture_document(side, gm02, gm03, gm04, gm06, gm07):
    return {'schema': SCHEMA, 'side': validate_side(side), 'gm02': gm02, 'gm03': gm03,
            'gm04': gm04, 'gm06': gm06, 'gm07': gm07}


class RecordedProviders:
    """The frozen provider answers, served locally and recorded per request.

    An unmatched request fails closed: a live provider call, an unexpected URL
    or a missing fixture can never be answered with a default body.
    """

    def __init__(self, records, clock=None):
        self.records = records
        self.clock = clock or time.monotonic
        self.log = []

    def answer(self, path):
        for key, record in self.records.items():
            if record['path'] == path:
                self.log.append({'provider': record['provider'], 'fixture': key,
                                 'at': self.clock()})
                return record['status'], record['body']
        raise RuntimeError('recorded provider fixture is absent for this request')

    def requests(self):
        return [dict(item) for item in self.log]


def serve_recorded_providers(providers):
    """Bind the pinned adapters to the recorded answers; no socket is opened."""
    import httpx
    original = httpx.AsyncClient

    def handle(request):
        status, body = providers.answer(request.url.path)
        return httpx.Response(status, json=body)

    class RecordedClient(original):
        def __init__(self, *arguments, **options):
            options['transport'] = httpx.MockTransport(handle)
            super().__init__(*arguments, **options)

    httpx.AsyncClient = RecordedClient
    return original


class RecordingProvider:
    """Record the outcome of every enrichment the pinned chain produced."""

    def __init__(self, provider):
        self.provider = provider
        self.outcomes = {}

    @property
    def name(self):
        return self.provider.name

    @property
    def supported_entity_types(self):
        return self.provider.supported_entity_types

    def supports_entity_type(self, entity_type):
        return self.provider.supports_entity_type(entity_type)

    async def enrich(self, entity_name, entity_type, **options):
        result = await self.provider.enrich(entity_name, entity_type, **options)
        self.outcomes[entity_name] = {'status': result.status.value, 'failure': None}
        return result

    def last(self, entity_name):
        outcome = self.outcomes.get(entity_name)
        if outcome is None:
            raise RuntimeError('no recorded enrichment outcome for this entity')
        return outcome


def enrichment_chain(settings):
    """The pinned factory decides the chain; only the transport is recorded.

    The reference chain is `CachedEnrichmentProvider(WikimediaProvider)`, exactly
    what `create_enrichment_service` builds with the pinned defaults.
    """
    from neo4j_agent_memory.enrichment.factory import create_enrichment_service
    service = create_enrichment_service(settings.enrichment)
    if service is None:
        raise ValueError('the pinned enrichment service is disabled')
    return RecordingProvider(service)


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


def _hit(item, identifier, score):
    """One search hit: identity plus the score the server itself reported."""
    return {'id': str(identifier), 'score': float(score)}


class ReferenceAdapter:
    """Drive the pinned client; the runner never projects product data here."""

    def __init__(self, client, driver, providers):
        self.client, self.driver = client, driver
        self.providers, self.side = providers, 'reference'

    def provider_requests(self):
        return self.providers.requests()

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
                generate_embedding=arguments['generate_embedding'],
                user_identifier=arguments.get('user_identifier'))
            return {'id': str(message.id), 'content': message.content}
        if operation == 'add_entity':
            entity, deduplication = await self.client.long_term.add_entity(
                arguments['name'], arguments['entity_type'],
                resolve=arguments['resolve'], deduplicate=arguments['deduplicate'],
                generate_embedding=True, geocode=False, enrich=False)
            return {'entity': reference_entity(entity),
                    'deduplication': reference_deduplication(deduplication)}
        if operation == 'add_relationship':
            from uuid import UUID
            relationship = await self.client.long_term.add_relationship(
                UUID(arguments['source_id']), UUID(arguments['target_id']),
                arguments['relationship_type'], confidence=arguments['confidence'])
            return {'id': str(relationship.id)}
        if operation == 'enrich_entity':
            entity, _ = await self.client.long_term.add_entity(
                arguments['name'], arguments['entity_type'], resolve=False,
                deduplicate=False, generate_embedding=True, geocode=False, enrich=True)
            await self._settle(entity.id)
            outcome = self.client._enrichment_provider.last(arguments['name'])
            return {'entity': {'id': str(entity.id)}, **outcome}
        if operation == 'geocode_entity':
            entity, _ = await self.client.long_term.add_entity(
                arguments['name'], arguments['entity_type'], resolve=False,
                deduplicate=False, generate_embedding=True, geocode=True, enrich=False)
            rows = self.query(ENTITY_STATE, {'id': str(entity.id)})
            stored = bool(rows) and rows[0]['latitude'] is not None
            return {'entity': {'id': str(entity.id)},
                    'status': 'success' if stored else 'not_found', 'failure': None}
        if operation == 'search_entities':
            found = await self.client.long_term.search_entities(
                arguments['query'], limit=arguments['limit'],
                threshold=arguments['threshold'])
            return [{'id': str(entity.id), 'name': entity.name, 'type': entity.type,
                     'score': entity.metadata['similarity']} for entity in found]
        if operation == 'search_messages':
            found = await self.client.short_term.search_messages(
                arguments['query'], limit=arguments['limit'],
                threshold=arguments['threshold'])
            return [_hit(item, item.id, item.metadata['similarity']) for item in found]
        if operation == 'get_related_entities':
            from uuid import UUID
            related = await self.client.long_term.get_related_entities(
                UUID(arguments['entity_id']), depth=arguments['depth'])
            return [{'id': str(entity.id), 'name': entity.name, 'type': entity.type}
                    for entity, _ in related]
        if operation == 'get_context':
            return await self.client.get_context(arguments['query'],
                                                 include_reasoning=False,
                                                 max_items=arguments['max_items'])
        raise ValueError('unsupported operation')

    async def _settle(self, entity_id, timeout=120.0):
        """Wait until the pinned background worker finished this entity."""
        service = self.client._enrichment_service
        if service is None:
            raise RuntimeError('the pinned background enrichment worker is absent')
        deadline = time.monotonic() + timeout
        while service.is_pending(entity_id):
            if time.monotonic() > deadline:
                raise RuntimeError('pinned enrichment did not settle')
            await asyncio.sleep(0.05)


class ProductAdapter:
    """Drive the source-bound bridge wire; no native product object is read."""

    def __init__(self, url, driver):
        self.url, self.driver = url, driver
        self.providers, self.side = RecordedProviders({}), 'product'

    def provider_requests(self):
        return self.providers.requests()

    def query(self, statement, parameters):
        with self.driver.session() as session:
            return session.run(statement, **parameters).data()

    async def _call(self, operation, **arguments):
        try:
            return await asyncio.to_thread(_post, f'{self.url}/{operation}', arguments)
        except LookupError as error:
            raise RuntimeError(f'bridge exposes no {operation} method') from error

    async def call(self, operation, **arguments):
        if operation == 'add_entity':
            return await self._call('add_entity', **arguments)
        if operation in ('add_message', 'add_relationship'):
            return await self._call(operation, **arguments)
        if operation == 'enrich_entity':
            created = await self._call('add_entity', name=arguments['name'],
                                       entity_type=arguments['entity_type'],
                                       resolve=False, deduplicate=False)
            identifier = created['entity']['id']
            outcome = await self._call('enrich_entity', entity_id=str(identifier))
            return {'entity': {'id': str(identifier)}, 'status': outcome['status'],
                    'failure': outcome['failure']}
        if operation == 'geocode_entity':
            created = await self._call('add_entity', name=arguments['name'],
                                       entity_type=arguments['entity_type'],
                                       resolve=False, deduplicate=False)
            identifier = created['entity']['id']
            outcome = await self._call('geocode_entity', entity_id=str(identifier))
            return {'entity': {'id': str(identifier)}, 'status': outcome['status'],
                    'failure': outcome.get('failure')}
        if operation == 'search_entities':
            found = await self._call('search_entities', query=arguments['query'],
                                     limit=arguments['limit'])
            return [{'id': str(item['id']), 'name': item['name'], 'type': item['type'],
                     'score': item['metadata']['similarity']} for item in found]
        if operation == 'search_messages':
            found = await self._call('search_messages', query=arguments['query'],
                                     limit=arguments['limit'],
                                     threshold=arguments['threshold'],
                                     user_identifier=arguments.get('user_identifier'))
            return [_hit(item, item['id'], item['metadata']['similarity']) for item in found]
        if operation == 'get_related_entities':
            found = await self._call('get_related_entities',
                                     entity_id=arguments['entity_id'],
                                     depth=arguments['depth'])
            return [{'id': str(item['id']), 'name': item['name'], 'type': item['type']}
                    for item in found]
        if operation == 'get_context':
            return await self._call('get_context', query=arguments['query'],
                                    max_items=arguments['max_items'],
                                    user_identifier=arguments.get('user_identifier'))
        raise ValueError('unsupported operation')


async def reference_client(uri, password, model, providers):
    from neo4j_agent_memory import MemoryClient
    from neo4j_agent_memory.config.settings import MemorySettings
    from neo4j_agent_memory.services.geocoder import create_geocoder
    from tck.fixtures.mocks import MockEmbedder
    settings = MemorySettings(
        backend='bolt',
        neo4j={'uri': uri, 'username': 'neo4j', 'password': password},
        extraction={'extractor_type': 'spacy', 'spacy_model': model},
        resolution={'strategy': 'none'},
        enrichment={'enabled': True, 'providers': ['wikimedia'], 'cache_results': True},
        geocoding={'enabled': True, 'provider': 'nominatim', 'cache_results': True,
                   'rate_limit_per_second': 1.0},
    )
    geocoder = create_geocoder(provider='nominatim', cache_results=True, rate_limit=1.0,
                               user_agent=settings.geocoding.user_agent)
    client = MemoryClient(settings, embedder=MockEmbedder(), geocoder=geocoder,
                          enrichment_provider=enrichment_chain(settings))
    await client.connect()
    return client


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
    original = None
    try:
        with driver.session() as session:
            require_empty(session.run('MATCH (n) RETURN count(n) AS count').data())
        if side == 'product':
            binary = Path('/target/muninn')
            consume_binary('/built/muninn', binary, os.environ['GM_H4_BINARY_SHA256'])
            product, url = start_bridge(binary, uri, password)
            adapter = ProductAdapter(url, driver)
        else:
            providers = RecordedProviders(frozen_fixture()['gm06']['providers'])
            original = serve_recorded_providers(providers)
            client = asyncio.run(reference_client(uri, password, model, providers))
            adapter = ReferenceAdapter(client, driver, providers)

        async def capture():
            gm04 = await capture_gm04(adapter)
            gm07 = await capture_gm07(adapter)
            gm02 = await capture_gm02(adapter)
            gm06 = await capture_gm06(adapter)
            return gm02, gm04, gm06, gm07

        try:
            gm02, gm04, gm06, gm07 = asyncio.run(capture())
        finally:
            if side == 'reference':
                asyncio.run(adapter.client.close())
        gm03 = gm03_section(side, frozen_fixture()['gm03']['texts'], model)
        document = capture_document(side, gm02, gm03, gm04, gm06, gm07)
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
            'providers': ('recorded local fixtures' if side == 'reference'
                          else 'none: the product binary registers no enrichment stage'),
        }, indent=2) + '\n')
        return 0
    finally:
        if original is not None:
            import httpx
            httpx.AsyncClient = original
        if product is not None:
            stop_bridge(product)
        driver.close()


if __name__ == '__main__':
    raise SystemExit(main())
