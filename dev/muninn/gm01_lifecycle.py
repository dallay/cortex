"""Verificación GM-01 de ciclo de vida sobre bases exclusivamente efímeras."""
from copy import deepcopy


def check_chain(graph, expected):
    errors = []
    checks = {'members': sorted(expected), 'first': expected[:1],
              'last': expected[-1:],
              'next': sorted([a, b] for a, b in zip(expected, expected[1:]))}
    for key, value in checks.items():
        actual = sorted(graph[key])
        if actual != value:
            errors.append(key)
    return errors


def check_append(message, conversation_id, content, metadata, *, role='user'):
    """Contrasta valores observados con entradas, no con el otro backend."""
    import json
    expected = {'conversation_id': conversation_id, 'role': role,
                'content': content, 'metadata': metadata}
    errors = []
    for key, value in expected.items():
        if json.dumps(message.get(key), sort_keys=True, allow_nan=False) != json.dumps(
                value, sort_keys=True, allow_nan=False):
            errors.append(key)
    if not isinstance(message.get('id'), str) or not message['id']:
        errors.append('id')
    return errors


def check_delete_effects(before, after, public_before, public_after, target):
    """Deriva el estado esperado sin reutilizar los datos observados tras borrar."""
    import json
    from gm01_contract import canonical_graph
    expected = deepcopy(before)
    expected['nodes'] = [node for node in expected['nodes'] if node['key'] != target]
    incoming = [edge for edge in before['edges'] if edge['target'] == target]
    outgoing = [edge for edge in before['edges'] if edge['source'] == target]
    expected['edges'] = [edge for edge in expected['edges']
                         if target not in (edge['source'], edge['target'])]
    successors = [edge['target'] for edge in outgoing if edge['type'] == 'NEXT_MESSAGE']
    for edge in incoming:
        if edge['type'] in ('NEXT_MESSAGE', 'FIRST_MESSAGE'):
            for successor in successors:
                expected['edges'].append({'source': edge['source'], 'target': successor,
                                          'type': edge['type'], 'properties': {}})
    expected_public = deepcopy(public_before)
    expected_public['messages'] = [message for message in expected_public['messages']
                                   if message['id'] != target]
    encode = lambda value: json.dumps(value, sort_keys=True, allow_nan=False, ensure_ascii=False)
    errors = []
    if encode(canonical_graph(expected)) != encode(canonical_graph(after)):
        errors.append('graph')
    if encode(expected_public) != encode(public_after):
        errors.append('public')
    return errors


def classify_delete_error(code):
    return 'GM01-DEV-001' if code == 'Neo.ClientError.Schema.ConstraintValidationFailed' else None


async def capture_invalid_role(adapter, session, message_ids):
    before = (await adapter.get(session), await adapter.graph(session, message_ids))
    category = await adapter.invalid_role(session)
    after = (await adapter.get(session), await adapter.graph(session, message_ids))
    errors = []
    if category != 'invalid_input':
        errors.append('category')
    if before != after:
        errors.append('state_changed')
    return {'id': 'invalid-role', 'category': category, 'unchanged': before == after,
            'errors': errors}


async def capture_missing_delete(adapter, session, message_id):
    before = (await adapter.get(session), await adapter.snapshot(session))
    result = await adapter.delete(message_id)
    after = (await adapter.get(session), await adapter.snapshot(session))
    errors = []
    if result is not False:
        errors.append('return')
    if before != after:
        errors.append('state_changed')
    return {'id': 'delete-missing', 'message_id': message_id,
            'result': result, 'unchanged': before == after, 'errors': errors}


async def capture_clear(adapter, session, message_ids):
    """Comprueba borrado completo e idempotente, incluidos posibles huérfanos."""
    case = {'id': 'clear-session', 'errors': []}
    for prefix in ('', 'repeat_', 'restart_'):
        if prefix == 'restart_':
            await adapter.reconnect()
        else:
            result = await adapter.clear(session)
            if result is not None:
                case['errors'].append(prefix + 'return')
        read = await adapter.get(session)
        remaining = await adapter.remaining(session, message_ids)
        case[prefix + 'read'] = read
        case[prefix + 'remaining'] = remaining
        if read.get('messages') != []:
            case['errors'].append(prefix + 'messages')
        if remaining != {'conversations': 0, 'messages': 0}:
            case['errors'].append(prefix + 'remaining_nodes')
    return case


async def capture_interleaved(reference, rust):
    """FIX-CONV-01: comprueba ambas sesiones después de cada escritura."""
    from gm01_contract import validate_public_invariants, compare_public_messages
    sessions = ('gm01-interleaved-a', 'gm01-interleaved-b')
    adapters = (reference, rust)
    created = [[await adapter.create(session) for session in sessions] for adapter in adapters]
    expected = [[[], []], [[], []]]
    case = {'id': 'interleaved-sessions', 'errors': [], 'checkpoints': []}
    sequence = ((0, 'system', 'Sistema'), (1, 'user', '¡Hola 世界!'),
                (0, 'user', 'duplicado'), (1, 'assistant', 'Respuesta 🧠'),
                (0, 'assistant', 'duplicado'), (1, 'tool', 'resultado'))
    async def checkpoint(label):
        reads = []
        for side, adapter in enumerate(adapters):
            pair = []
            for slot, session in enumerate(sessions):
                read = await adapter.get(session)
                errors = validate_public_invariants(read)
                if read['id'] != created[side][slot]['id'] or read['session_id'] != session:
                    errors.append('session_identity')
                if len(read['messages']) != len(expected[side][slot]):
                    errors.append('message_count')
                for message, entry in zip(read['messages'], expected[side][slot]):
                    role, content, metadata, mid = entry
                    errors.extend(check_append(message, read['id'], content, metadata, role=role))
                    if message['id'] != mid:
                        errors.append('message_identity')
                case['errors'].extend(f'{label}:{side}:{slot}:{error}' for error in errors)
                pair.append(read)
            reads.append(pair)
        for slot in range(2):
            for left, right in zip(reads[0][slot]['messages'], reads[1][slot]['messages']):
                case['errors'].extend(f'{label}:{slot}:public:{error}' for error in
                                      compare_public_messages(left, right))
        case['checkpoints'].append({'id': label, 'reads': reads})
    for index, (slot, role, content) in enumerate(sequence):
        metadata = {'index': index, 'nested': {'flag': True}}
        returns = []
        for side, adapter in enumerate(adapters):
            message = await adapter.add(sessions[slot], content, metadata, role=role)
            case['errors'].extend(f'append:{side}:{index}:{error}' for error in check_append(
                message, created[side][slot]['id'], content, metadata, role=role))
            expected[side][slot].append((role, content, metadata, message['id']))
            returns.append(message)
        case['errors'].extend(f'append:{index}:public:{error}' for error in
                              compare_public_messages(*returns))
        await checkpoint(f'append-{index}')
    for adapter in adapters:
        await adapter.reconnect()
    await checkpoint('reconnect')
    return case


def finalize_lifecycle(report):
    """Deriva cobertura de casos efectivamente ejecutados; no acepta omisiones."""
    cases = report['cases']
    ids = {case['id'] for case in cases}
    pending = []
    for requirement, prefix, count in (
        ('full_graph_properties', 'graph-', 3),
        ('reconnect_restart', 'restart-live', 6),
        ('conversation_delete', 'clear-session', 6),
        ('complete_public_returns', 'append-', 6),
        ('errors', 'invalid-role', 6),
    ):
        matching = [case for case in cases if case['id'].startswith(prefix)]
        if len(matching) != count or any(case.get('errors') for case in matching):
            pending.append(requirement)
    required_ids = {'interleaved-sessions', *(f'{prefix}-{suffix}'
        for prefix in ('create', 'graph', 'delete-effects', 'delete')
        for suffix in ('first', 'middle', 'last'))}
    if not required_ids.issubset(ids):
        pending.append('scenario_coverage')
    for case in cases:
        identifier = case['id']
        required = ()
        if identifier == 'restart-live':
            required = ('before_public', 'after_public', 'before_graph', 'after_graph', 'backend', 'session')
        elif identifier == 'clear-session':
            required = ('read', 'remaining', 'repeat_read', 'repeat_remaining', 'restart_read', 'restart_remaining', 'backend', 'session')
        elif identifier.startswith('graph-'):
            required = ('reference', 'rust', 'canonical', 'equal')
        elif identifier.startswith('append-'):
            required = ('returns', 'read', 'backend')
        elif identifier == 'interleaved-sessions':
            required = ('checkpoints',)
        if any(key not in case for key in required):
            pending.append('case_evidence:' + identifier)
        if identifier == 'interleaved-sessions' and not case.get('checkpoints'):
            pending.append('interleaved_checkpoints')
    report['pending'] = sorted(set(pending))
    report['status'] = 'pass' if not pending and not report['errors'] else 'incomplete'
    report['gm01_acceptance'] = 'pass_with_authorized_deviations' if report['status'] == 'pass' else 'not_established'


async def capture_live_restart(adapter, session, message_ids):
    """Compara datos públicos y grafo exactos dentro de cada backend al reiniciar."""
    before_public = await adapter.get(session)
    before_graph = await adapter.graph(session, message_ids)
    await adapter.reconnect()
    after_public = await adapter.get(session)
    after_graph = await adapter.graph(session, message_ids)
    errors = []
    if before_public != after_public:
        errors.append('public_changed')
    if before_graph != after_graph:
        errors.append('graph_changed')
    return {'id': 'restart-live', 'before_public': before_public,
            'after_public': after_public, 'before_graph': before_graph,
            'after_graph': after_graph, 'errors': errors}


async def capture_lifecycle(reference, rust):
    """Ejecuta operaciones idénticas; registra divergencias, nunca las oculta."""
    from gm01_contract import compare_conversations
    report = {'suite': 'GM-01', 'status': 'incomplete', 'cases': [],
              'deviations': [], 'errors': [], 'h1_acceptance': 'not_established'}
    for suffix, position in (('first', 0), ('middle', 1), ('last', 3)):
        session = 'gm01-lifecycle-' + suffix
        pair = []
        for adapter in (reference, rust):
            created = await adapter.create(session)
            if created.get('messages') != []:
                raise ValueError('Creación de conversación no vacía')
            again = await adapter.create(session)
            if created['id'] != again['id'] or created != again:
                raise ValueError('Creación no idempotente')
            pair.append(created)
        case = {'id': 'create-' + suffix, 'reference': pair[0], 'rust': pair[1]}
        try:
            errors = compare_conversations(*pair)
        except ValueError:
            errors = ['public_fields']
        case['errors'] = errors
        report['cases'].append(case)
        report['errors'].extend('create-' + suffix + ':' + e for e in errors)
        ids = []
        inputs = [('¡Hola 世界! 🧠', {'order': 0, 'nested': {'flag': True, 'values': [1, 1.0, None]}}),
                  ('duplicado', {'order': 1}), ('duplicado', {'order': 2}),
                  ('último\nfin', {'order': 3})]
        for side, adapter in enumerate((reference, rust)):
            messages, returns = [], []
            for index, (content, metadata) in enumerate(inputs):
                message = await adapter.add(session, content, metadata)
                errors = check_append(message, pair[side]['id'], content, metadata)
                report['errors'].extend(f'{suffix}:{side}:append-{index}:{e}' for e in errors)
                messages.append(message['id'])
                returns.append(message)
            observed = await adapter.get(session)
            from gm01_contract import validate_public_invariants
            read_errors = validate_public_invariants(observed)
            returned_conversation = deepcopy(pair[side])
            returned_conversation['messages'] = returns
            read_errors.extend('return:' + error for error in
                               validate_public_invariants(returned_conversation))
            if len(observed['messages']) != len(inputs):
                read_errors.append('message_count')
            for index, (message, (content, metadata)) in enumerate(zip(observed['messages'], inputs)):
                read_errors.extend(f'{index}:{e}' for e in check_append(
                    message, pair[side]['id'], content, metadata))
                if message['id'] != messages[index]:
                    read_errors.append(f'{index}:identity')
                from gm01_contract import compare_append_read
                # append supplies no timestamp: upstream returns its client clock,
                # but Neo4j persists server-clock datetime() (spec §10.2).
                read_errors.extend(f'{index}:append_read:{error}'
                    for error in compare_append_read(
                        returns[index], message, generated_time_window_seconds=1.0))
            report['errors'].extend(f'{suffix}:{side}:read:{e}' for e in read_errors)
            report['cases'].append({'id': 'append-' + suffix,
                                    'backend': 'reference' if side == 0 else 'rust',
                                    'returns': returns, 'read': observed, 'errors': read_errors})
            ids.append(messages)
        from gm01_contract import compare_public_messages, validate_complete_message
        append_cases = report['cases'][-2:]
        for side, append_case in enumerate(append_cases):
            for operation in ('returns', 'read'):
                values = append_case[operation]
                if operation == 'read':
                    values = values['messages']
                for index, message in enumerate(values):
                    report['errors'].extend(f'{suffix}:{side}:{operation}:{index}:complete:{e}'
                        for e in validate_complete_message(message, pair[side]['id']))
        for operation in ('returns', 'read'):
            left = append_cases[0][operation]
            right = append_cases[1][operation]
            if operation == 'read':
                left, right = left['messages'], right['messages']
            if len(left) != len(right):
                report['errors'].append(suffix + ':public:message_count')
            for index, (reference_message, rust_message) in enumerate(zip(left, right)):
                differences = compare_public_messages(reference_message, rust_message)
                report['errors'].extend(f'{suffix}:public:{operation}:{index}:{e}' for e in differences)
        for side, adapter in enumerate((reference, rust)):
            invalid_case = await capture_invalid_role(adapter, session, ids[side])
            invalid_case['backend'] = 'reference' if side == 0 else 'rust'
            report['cases'].append(invalid_case)
            report['errors'].extend(f'{suffix}:{side}:invalid_role:{error}' for error in invalid_case['errors'])
            for missing in ('invalid-id', '00000000-0000-0000-0000-000000000000'):
                missing_case = await capture_missing_delete(adapter, session, missing)
                missing_case['backend'] = 'reference' if side == 0 else 'rust'
                report['cases'].append(missing_case)
                report['errors'].extend(f'{suffix}:{side}:missing:{e}' for e in missing_case['errors'])
        for side, adapter in enumerate((reference, rust)):
            restart = await capture_live_restart(adapter, session, ids[side])
            restart['backend'] = 'reference' if side == 0 else 'rust'
            restart['session'] = session
            report['cases'].append(restart)
            report['errors'].extend(f'{suffix}:{side}:restart:{e}' for e in restart['errors'])
        public_before = [await adapter.get(session) for adapter in (reference, rust)]
        from gm01_contract import normalize_persisted_graph
        full_before = [await adapter.graph(session, ids[side])
                       for side, adapter in enumerate((reference, rust))]
        from gm01_contract import validate_expected_graph
        for side, graph in enumerate(full_before):
            report['errors'].extend(f'{suffix}:{side}:expected_graph:{e}'
                for e in validate_expected_graph(graph, public_before[side]))
        normalized = [normalize_persisted_graph(graph, {
            pair[side]['id']: 'conversation',
            **{mid: f'message-{index}' for index, mid in enumerate(ids[side])}})
            for side, graph in enumerate(full_before)]
        from gm01_contract import compare_scoped_graphs
        graph_errors = compare_scoped_graphs(*normalized)
        report['cases'].append({'id': 'graph-' + suffix, 'reference': full_before[0],
                                'rust': full_before[1], 'canonical': normalized,
                                'equal': not graph_errors, 'errors': graph_errors})
        report['errors'].extend(suffix + ':' + error for error in graph_errors)
        if not graph_errors:
            report['deviations'].append({'id': 'GM01-DEV-002', 'case': suffix,
                                         'reference_owner_scope_absent': True,
                                         'rust_owner_scope': 'unowned',
                                         'remaining_graph_equal': True})
        before = [await adapter.snapshot(session) for adapter in (reference, rust)]
        for side, snapshot in enumerate(before):
            errors = check_chain(snapshot, ids[side])
            report['errors'].extend(suffix + ':before:' + e for e in errors)
        reference_error = None
        try:
            reference_deleted = await reference.delete(ids[0][position])
        except Exception as error:
            code = getattr(error, 'code', None)
            reference_error = code if isinstance(code, str) and code.startswith('Neo.') else type(error).__name__
            reference_deleted = None
        rust_deleted = await rust.delete(ids[1][position])
        after = [await adapter.snapshot(session) for adapter in (reference, rust)]
        public_after = [await adapter.get(session) for adapter in (reference, rust)]
        full_after = [await adapter.graph(session, ids[side])
                      for side, adapter in enumerate((reference, rust))]
        remaining = ids[1][:position] + ids[1][position + 1:]
        rust_errors = check_chain(after[1], remaining)
        effect_errors = check_delete_effects(full_before[1], full_after[1],
                                             public_before[1], public_after[1], ids[1][position])
        rust_errors.extend('effects_' + error for error in effect_errors)
        report['cases'].append({'id': 'delete-effects-' + suffix,
                                'before_graph': full_before[1], 'after_graph': full_after[1],
                                'before_public': public_before[1], 'after_public': public_after[1],
                                'target': ids[1][position], 'errors': effect_errors})
        if rust_deleted is not True:
            rust_errors.append('delete_result')
        if reference_error:
            deviation = classify_delete_error(reference_error)
            # La excepción solo es aplicable si la transacción upstream no cambió el grafo.
            if (deviation and before[0] == after[0]
                    and public_before[0] == public_after[0]
                    and full_before[0] == full_after[0] and not rust_errors):
                report['deviations'].append({'id': deviation, 'case': suffix,
                                             'reference_error_code': reference_error,
                                             'reference_unchanged': True,
                                             'rust_chain_valid': True})
            else:
                report['errors'].append(suffix + ':unapproved_reference_error')
        elif reference_deleted is not True or check_chain(after[0], ids[0][:position] + ids[0][position + 1:]):
            report['errors'].append(suffix + ':reference_delete')
        report['errors'].extend(suffix + ':rust_delete:' + e for e in rust_errors)
        report['cases'].append({'id': 'delete-' + suffix, 'before': before,
                                'after': after, 'rust_errors': rust_errors})
        for side, adapter in enumerate((reference, rust)):
            await adapter.reconnect()
            persisted = await adapter.snapshot(session)
            public_persisted = await adapter.get(session)
            full_persisted = await adapter.graph(session, ids[side])
            if full_persisted != full_after[side]:
                report['errors'].append(suffix + ':restart_graph_properties_changed')
            if public_persisted != public_after[side]:
                report['errors'].append(suffix + ':restart_public_changed')
            report['cases'].append({'id': 'reconnect-public-' + suffix,
                                    'backend': 'reference' if side == 0 else 'rust',
                                    'before': public_after[side], 'after': public_persisted,
                                    'unchanged': public_persisted == public_after[side]})
            if persisted != after[side]:
                report['errors'].append(suffix + ':restart_graph_changed')
            report['cases'].append({'id': 'reconnect-' + suffix,
                                    'backend': 'reference' if side == 0 else 'rust',
                                    'graph_unchanged': persisted == after[side]})
        for side, adapter in enumerate((reference, rust)):
            clear_case = await capture_clear(adapter, session, ids[side])
            clear_case['backend'] = 'reference' if side == 0 else 'rust'
            clear_case['session'] = session
            report['cases'].append(clear_case)
            report['errors'].extend(f'{suffix}:{side}:clear:{e}' for e in clear_case['errors'])
    # Las etapas aún no implementadas bloquean la aceptación, incluso con borrado correcto.
    finalize_lifecycle(report)
    return report


class LifecycleAdapter:
    """Acceso limitado a bases efímeras ya poseídas por el runner."""
    def __init__(self, database, *, reference=None, fixture=None, checkpoint=None):
        if (reference is None) == (fixture is None):
            raise ValueError('Se requiere exactamente un backend')
        self.database, self.reference, self.fixture = database, reference, fixture
        self.checkpoint = checkpoint

    def mark(self, operation):
        if self.checkpoint is not None:
            self.checkpoint(('reference' if self.reference is not None else 'rust') + ':' + operation)

    async def create(self, session):
        self.mark('create')
        if self.reference is not None:
            result = await self.reference.short_term.create_conversation(session)
            return result.model_dump(mode='json')
        self.fixture.assert_live()
        return self.fixture.call('create_session_conversation', session_id=session)

    async def get(self, session):
        self.mark('get')
        if self.reference is not None:
            result = await self.reference.short_term.get_conversation(session)
            return result.model_dump(mode='json')
        self.fixture.assert_live()
        return self.fixture.call('get_conversation', session_id=session)

    async def add(self, session, content, metadata, *, role='user'):
        self.mark('add')
        if self.reference is not None:
            result = await self.reference.short_term.add_message(
                session, role, content, metadata=deepcopy(metadata),
                extract_entities=False, extract_relations=False,
                generate_embedding=False, extraction_mode='skip')
            return result.model_dump(mode='json')
        self.fixture.assert_live()
        return self.fixture.call('add_message', session_id=session, role=role,
                                 content=content, metadata=deepcopy(metadata),
                                 generate_embedding=False)

    async def invalid_role(self, session):
        self.mark('invalid_role')
        if self.reference is not None:
            try:
                await self.add(session, 'invalid role fixture', {}, role='gm01-invalid-role')
            except ValueError:
                return 'invalid_input'
            return 'unexpected_success'
        self.fixture.assert_live()
        status, _ = self.fixture._request('add_message', {
            'session_id': session, 'role': 'gm01-invalid-role',
            'content': 'invalid role fixture', 'metadata': {}, 'generate_embedding': False})
        return 'invalid_input' if status == 400 else 'unexpected_status'

    async def delete(self, message_id):
        self.mark('delete')
        if self.reference is not None:
            return await self.reference.short_term.delete_message(message_id)
        self.fixture.assert_live()
        status, payload = self.fixture._request('delete_message', {'message_id': message_id})
        if (status != 200 or not isinstance(payload, dict)
                or set(payload) != {'deleted'} or not isinstance(payload['deleted'], bool)):
            raise RuntimeError('Respuesta de borrado inválida')
        return payload['deleted']

    async def graph(self, session, message_ids):
        self.mark('graph')
        from neo4j import GraphDatabase
        if not self.database.active:
            raise ValueError('Base efímera cerrada')
        # Incluye relaciones entrantes y salientes para detectar enlaces ajenos al fixture.
        node_query = '''MATCH (n) WHERE (n:Conversation AND n.session_id = $session)
OR (n:Message AND n.id IN $ids)
RETURN n.id AS key, labels(n) AS labels, properties(n) AS properties LIMIT 101'''
        edge_query = '''MATCH (a)-[r]->(b)
WHERE (a:Conversation AND a.session_id = $session) OR a.id IN $ids
OR (b:Conversation AND b.session_id = $session) OR b.id IN $ids
RETURN a.id AS source, b.id AS target, type(r) AS type, properties(r) AS properties LIMIT 201'''
        def plain(value):
            if isinstance(value, dict):
                return {key: plain(child) for key, child in value.items()}
            if isinstance(value, list):
                return [plain(child) for child in value]
            if hasattr(value, 'iso_format'):
                return value.iso_format()
            return value
        with GraphDatabase.driver(self.database.uri,
                                  auth=(self.database.username, self.database.password)) as driver:
            with driver.session() as connection:
                nodes = [plain(dict(row)) for row in connection.run(node_query, session=session, ids=message_ids)]
                edges = [plain(dict(row)) for row in connection.run(edge_query, session=session, ids=message_ids)]
        if len(nodes) > 100 or len(edges) > 200:
            raise ValueError('Grafo excede los límites del fixture')
        from gm01_contract import canonical_graph
        return canonical_graph({'nodes': nodes, 'edges': edges})

    async def remaining(self, session, message_ids):
        self.mark('remaining')
        from neo4j import GraphDatabase
        if not self.database.active:
            raise ValueError('Base efímera cerrada')
        query = '''OPTIONAL MATCH (c:Conversation {session_id: $session})
WITH count(c) AS conversations
OPTIONAL MATCH (m:Message) WHERE m.id IN $ids
RETURN conversations, count(m) AS messages'''
        with GraphDatabase.driver(self.database.uri,
                                  auth=(self.database.username, self.database.password)) as driver:
            with driver.session() as connection:
                return dict(connection.run(query, session=session, ids=message_ids).single(strict=True))

    async def clear(self, session):
        self.mark('clear_session')
        if self.reference is not None:
            return await self.reference.short_term.clear_session(session)
        self.fixture.assert_live()
        status, _ = self.fixture._request('clear_session', {'session_id': session})
        if status != 204:
            raise RuntimeError('Respuesta de borrado de sesión inválida')
        return None

    async def reconnect(self):
        self.mark('reconnect')
        if self.reference is not None:
            await self.reference.close()
            await self.reference.connect()
        else:
            self.fixture.close(preserve_database=True)
            self.fixture.start()

    async def snapshot(self, session):
        self.mark('snapshot')
        from neo4j import GraphDatabase
        if not self.database.active:
            raise ValueError('Base efímera cerrada')
        # Acotada al fixture creado y con límite fijo; nunca escribir Cypher desde este método.
        query = '''MATCH (c:Conversation {session_id: $session})
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(m:Message)
WITH c, collect(m.id) AS members
OPTIONAL MATCH (c)-[:FIRST_MESSAGE]->(f:Message)
WITH c, members, collect(f.id) AS first
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(tail:Message)
WHERE NOT (tail)-[:NEXT_MESSAGE]->()
WITH c, members, first, collect(tail.id) AS last
OPTIONAL MATCH (c)-[:HAS_MESSAGE]->(a:Message)-[:NEXT_MESSAGE]->(b:Message)
RETURN members, first, last, collect([a.id,b.id]) AS next LIMIT 2'''
        with GraphDatabase.driver(self.database.uri,
                                  auth=(self.database.username, self.database.password)) as driver:
            with driver.session() as connection:
                rows = list(connection.run(query, session=session))
        if len(rows) != 1:
            raise ValueError('Conversación inexistente o duplicada en grafo')
        result = dict(rows[0])
        result['next'] = [edge for edge in result['next'] if None not in edge]
        return {key: sorted(value) for key, value in result.items()}
