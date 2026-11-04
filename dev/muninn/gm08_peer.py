"""Internal GM08 peer, launched ONLY by the source-bound Dory runner.

This is not an external-binary capture command. The runner binds expected binary,
source and helper hashes, separate empty databases and an internal-only network.
"""
import asyncio
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from uuid import UUID

from gm05_adapters import GraphReader, _properties
from gm08_capture import capture_gm08
from reference import verify_source_tree

SIDES = ('reference', 'product')


def _post(url, payload):
    """Minimal loopback JSON POST: the pinned oracle image has no httpx."""
    request = urllib.request.Request(url, data=json.dumps(payload).encode(),
                                     headers={'content-type': 'application/json'})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = response.read()
            return json.loads(body) if body else None
    except urllib.error.HTTPError as error:
        if error.code == 404:
            raise LookupError('contract_absent') from error
        raise RuntimeError(f'bridge error {error.code}') from error


def start_bridge(binary, uri, password):
    """Start the product bridge on container loopback; same lifecycle as the GM05 fixture."""
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    url = f'http://127.0.0.1:{port}'
    env = {**os.environ, 'NAM_NEO4J__URI': uri, 'NAM_NEO4J__PASSWORD': password,
           'NAM_EMBEDDING__PROVIDER': 'hash', 'NAM_EMBEDDING__DIMENSIONS': '1536'}
    log = open('/out/product-bridge.log', 'w')
    process = subprocess.Popen(
        [str(binary), 'bridge', '--host', '127.0.0.1', '--port', str(port),
         '--allow-destructive-clear'], env=env, stdout=log, stderr=log)
    for _ in range(120):
        if process.poll() is not None:
            raise RuntimeError('source-bound bridge exited before readiness')
        try:
            _post(url + '/setup', {})
            return process, url
        except (urllib.error.URLError, OSError, RuntimeError):
            time.sleep(.5)
    raise RuntimeError('source-bound bridge readiness exhausted')


def stop_bridge(process):
    process.terminate()
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def validate_side(value):
    """Reject anything except the two exact capture sides."""
    if value not in SIDES:
        raise ValueError('unknown capture side')
    return value


def consume_binary(source, destination, expected):
    """Copy the bound build artifact and verify the consumed bytes.

    A mismatch removes the copy so no unverified executable is left runnable.
    """
    destination = Path(destination)
    destination.write_bytes(Path(source).read_bytes())
    try:
        destination.chmod(0o700)
        actual = hashlib.sha256(destination.read_bytes()).hexdigest()
        if actual != expected:
            raise ValueError('consumed binary hash mismatch')
    except BaseException:
        destination.unlink(missing_ok=True)
        raise
    return actual


def require_empty(rows):
    """A capture may only start on a database that is provably empty."""
    if not rows or rows[0].get('count') != 0:
        raise ValueError('fresh database is not empty')
    return rows[0]['count']


def serialize(value):
    if hasattr(value, 'model_dump'):
        return value.model_dump(mode='json')
    if isinstance(value, dict):
        return {key: serialize(item) for key, item in value.items()}
    if isinstance(value, (tuple, list)):
        return [serialize(item) for item in value]
    if isinstance(value, (datetime, UUID)):
        return str(value)
    return value


TRACE_WIRE = ('id', 'session_id', 'task', 'outcome', 'success', 'started_at', 'completed_at')
STEP_WIRE = ('id', 'trace_id', 'step_number', 'thought', 'action', 'observation')
CALL_WIRE = ('id', 'tool_name', 'arguments', 'result', 'status', 'duration_ms', 'error')
STATS_WIRE = ('name', 'total_calls', 'successful_calls', 'failed_calls', 'success_rate',
              'avg_duration_ms')
LIST_OPERATIONS = ('list_traces', 'get_similar_traces')


def _pick(value, names):
    return {name: value.get(name) for name in names}


def wire_tool_call(call):
    return _pick(call, CALL_WIRE)


def wire_step(step):
    value = _pick(step, STEP_WIRE)
    value['tool_calls'] = [wire_tool_call(call) for call in step.get('tool_calls') or ()]
    return value


def wire_trace(trace, with_steps=True):
    value = _pick(trace, TRACE_WIRE)
    value['steps'] = [wire_step(step) for step in trace.get('steps') or ()] if with_steps else []
    return value


def project_wire(operation, result):
    """Project a native reference result onto the exact product bridge wire shape.

    `crates/muninn/bridge/src/wire.rs` is the measured public surface. Native-only
    fields (MemoryEntry timestamps, embeddings, similarity metadata) are not part of
    it; unexposed native capabilities are covered by the Rust execution target instead.
    """
    if operation in LIST_OPERATIONS:
        return [wire_trace(trace, with_steps=False) for trace in result or ()]
    if operation == 'get_tool_stats':
        return [_pick(stats, STATS_WIRE) for stats in result or ()]
    if operation == 'record_tool_call':
        return wire_tool_call(result)
    if operation == 'add_step':
        return wire_step(result)
    return wire_trace(result, with_steps=True)


# The measured graph is the execution-memory surface. Both servers also keep
# schema/ontology bookkeeping (product `:MuninnSchema`, reference `:Ontology`/
# `:OntologyVersion`); those labels are excluded symmetrically before comparison.
EXECUTION_LABELS = ('Conversation', 'Message', 'Entity', 'ReasoningTrace',
                    'ReasoningStep', 'ToolCall', 'Tool')
ANY_LABEL = ' OR '.join(f'n:{label}' for label in EXECUTION_LABELS)
FROM_LABEL = ' OR '.join(f'a:{label}' for label in EXECUTION_LABELS)
TO_LABEL = ' OR '.join(f'b:{label}' for label in EXECUTION_LABELS)


class ExecutionGraph(GraphReader):
    def snapshot(self):
        nodes = self.query(f'MATCH (n) WHERE {ANY_LABEL} '
                           'RETURN labels(n) AS labels, properties(n) AS properties', {})
        edges = self.query(f'MATCH (a)-[r]->(b) WHERE ({FROM_LABEL}) AND ({TO_LABEL}) '
                           'RETURN coalesce(a.id, "Tool:" + a.name) AS source, '
                           'coalesce(b.id, "Tool:" + b.name) AS target, type(r) AS type, '
                           'properties(r) AS properties', {})
        return {
            'nodes': [{'key': node['properties'].get('id', 'Tool:' + node['properties'].get('name', '')),
                       'labels': node['labels'], 'properties': _properties(node['properties'])} for node in nodes],
            'edges': [{**edge, 'properties': _properties(edge['properties'])} for edge in edges],
        }


class Adapter:
    def __init__(self, side, uri, password):
        self.side, self.uri, self.password = side, uri, password
        self.driver = None
        self.client = None
        self.url = os.environ.get('GM08_BRIDGE_URL', 'http://127.0.0.1:3001')

    async def connect(self):
        from neo4j import GraphDatabase
        self.driver = GraphDatabase.driver(self.uri, auth=('neo4j', self.password))
        self.driver.verify_connectivity()
        if self.side == 'reference':
            from neo4j_agent_memory import MemoryClient
            from neo4j_agent_memory.config.settings import MemorySettings
            from tck.fixtures.mocks import MockEmbedder
            settings = MemorySettings(
                backend='bolt', neo4j={'uri': self.uri, 'username': 'neo4j', 'password': self.password},
                extraction={'extractor_type': 'none'}, resolution={'strategy': 'none'},
                enrichment={'enabled': False}, geocoding={'enabled': False})
            self.client = MemoryClient(settings, embedder=MockEmbedder())
            await self.client.connect()

    def query(self, statement, parameters):
        with self.driver.session() as session:
            return session.run(statement, **parameters).data()

    async def seed(self, fixture):
        count = self.query(f'MATCH (n) WHERE {ANY_LABEL} RETURN count(n) AS count', {})[0]['count']
        if count:
            raise ValueError('GM08 requires an empty execution-memory graph')
        self.query(
            "CREATE (c:Conversation {id: $conversation, session_id: $session, owner_scope: 'unowned'}) "
            "CREATE (m:Message {id: $message, content: $name, role: 'user', timestamp: datetime($timestamp)}) "
            "CREATE (e:Entity {id: $entity, name: $name, type: 'OBJECT'}) "
            "CREATE (c)-[:HAS_MESSAGE]->(m) CREATE (m)-[:MENTIONS]->(e)",
            {'conversation': fixture['conversation_id'], 'session': fixture['session_id'],
             'message': fixture['message_id'], 'name': fixture['entity_name'],
             'timestamp': fixture['timestamp'], 'entity': fixture['entity_id']})

    async def call(self, operation, **arguments):
        if self.side == 'product':
            return await asyncio.to_thread(_post, f'{self.url}/{operation}', arguments)
        from neo4j_agent_memory.core.memory import ToolCallStatus
        from neo4j_agent_memory.schema.models import EntityRef, TraceOutcome
        for field in ('trace_id', 'step_id'):
            if field in arguments:
                arguments[field] = UUID(arguments[field])
        if operation == 'record_tool_call':
            arguments['status'] = ToolCallStatus(arguments['status'])
            arguments['touched_entities'] = [EntityRef(**ref) for ref in arguments['touched_entities']]
        if operation == 'complete_trace' and isinstance(arguments['outcome'], dict):
            arguments['outcome'] = TraceOutcome(**arguments['outcome'])
        if operation == 'list_traces':
            for field in ('since', 'until'):
                if field in arguments:
                    arguments[field] = datetime.fromisoformat(arguments[field].replace('Z', '+00:00'))
        result = await getattr(self.client.reasoning, operation)(**arguments)
        return project_wire(operation, serialize(result))

    async def snapshot(self):
        self.query('CALL db.awaitIndexes(60)', {})
        return ExecutionGraph(self.query).snapshot()

    async def reconnect(self):
        await self.close()
        await self.connect()

    async def close(self):
        if self.client is not None:
            await self.client.close()
            self.client = None
        if self.driver is not None:
            self.driver.close()
            self.driver = None


def main():
    side = validate_side(os.environ['GM08_SIDE'])
    sources = Path('/references')
    integrity = json.loads(Path(__file__).with_name('source-integrity.json').read_text())
    verify_source_tree(sources, {key: integrity[key] for key in ('agent-memory', 'agent-memory-tck')})
    sys.path[:0] = [str(sources / 'agent-memory/src'), str(sources / 'agent-memory-tck')]
    uri, password = os.environ['NAM_NEO4J__URI'], os.environ['NAM_NEO4J__PASSWORD']
    from neo4j import GraphDatabase
    with GraphDatabase.driver(uri, auth=('neo4j', password)) as driver:
        with driver.session() as session:
            require_empty(session.run('MATCH (n) RETURN count(n) AS count').data())
    product = None
    try:
        if side == 'product':
            binary = Path('/target/muninn')
            consume_binary('/built/muninn', binary, os.environ['GM08_BINARY_SHA256'])
            product, url = start_bridge(binary, uri, password)
            os.environ['GM08_BRIDGE_URL'] = url
        async def capture():
            adapter = Adapter(side, uri, password)
            await adapter.connect()
            try:
                result = await capture_gm08(adapter)
                components = adapter.query('CALL dbms.components() YIELD versions, edition RETURN versions, edition', {})
                return result, components
            finally:
                await adapter.close()
        result, components = asyncio.run(capture())
        Path('/out/' + side + '.json').write_text(json.dumps(result, indent=2, allow_nan=False) + '\n')
        Path('/out/' + side + '-runtime.json').write_text(json.dumps({'python': sys.version, 'neo4j': components, 'provider': 'tck MockEmbedder SHA256 1536' if side == 'reference' else 'hash1536'}, indent=2) + '\n')
        return 0
    finally:
        if product is not None:
            stop_bridge(product)


if __name__ == '__main__':
    raise SystemExit(main())
