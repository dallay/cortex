"""Contratos GM-01 completos: normalización limitada a campos generados."""
from datetime import datetime
from copy import deepcopy

CONVERSATION_FIELDS = {'id', 'session_id', 'title', 'created_at', 'updated_at',
                       'metadata', 'messages'}


def _time(value):
    if not isinstance(value, str):
        raise ValueError('Timestamp inválido')
    parsed = datetime.fromisoformat(value.replace('Z', '+00:00'))
    if parsed.utcoffset() is None:
        raise ValueError('Timestamp sin zona')
    return parsed


def canonical_conversation(value):
    if not isinstance(value, dict) or set(value) != CONVERSATION_FIELDS:
        raise ValueError('Campos de conversación incompletos o desconocidos')
    for key in ('id', 'session_id'):
        if not isinstance(value[key], str) or not value[key].strip():
            raise ValueError('Identidad inválida')
    if value['title'] is not None and not isinstance(value['title'], str):
        raise ValueError('Título inválido')
    if not isinstance(value['metadata'], dict) or not isinstance(value['messages'], list):
        raise ValueError('Contenido de conversación inválido')
    created = _time(value['created_at'])
    updated = value['updated_at']
    if updated is not None and _time(updated) < created:
        raise ValueError('Actualización anterior a creación')
    result = deepcopy(value)
    result['id'] = '<conversation>'
    result['created_at'] = '<generated-time>'
    result['updated_at'] = None if updated is None else '<generated-time>'
    seen, previous = set(), None
    for index, message in enumerate(result['messages']):
        fields = {'id', 'conversation_id', 'role', 'content', 'timestamp', 'embedding', 'metadata'}
        if not isinstance(message, dict) or set(message) != fields:
            raise ValueError('Campos de mensaje incompletos o desconocidos')
        mid = message['id']
        if not isinstance(mid, str) or not mid or mid in seen:
            raise ValueError('Identidad de mensaje inválida')
        seen.add(mid)
        if message['conversation_id'] != value['id']:
            raise ValueError('Referencia de conversación inválida')
        timestamp = _time(message['timestamp'])
        if previous is not None and timestamp < previous:
            raise ValueError('Orden temporal inválido')
        previous = timestamp
        if message['role'] not in {'system', 'user', 'assistant', 'tool'}:
            raise ValueError('Rol inválido')
        if not isinstance(message['content'], str) or not isinstance(message['metadata'], dict):
            raise ValueError('Contenido de mensaje inválido')
        message['id'] = f'<message-{index}>'
        message['conversation_id'] = '<conversation>'
        message['timestamp'] = '<generated-time>'
    return result


def compare_conversations(left, right):
    import json
    def encoded(value):
        return json.dumps(canonical_conversation(value), sort_keys=True,
                          ensure_ascii=False, allow_nan=False, separators=(',', ':'))
    return [] if encoded(left) == encoded(right) else ['conversation_fields']


def validate_public_invariants(conversation):
    """Valida identidades y tiempos reales antes de cualquier normalización."""
    from uuid import UUID
    errors, seen, previous = [], set(), None
    def identity(value):
        try:
            UUID(value)
        except (ValueError, TypeError, AttributeError):
            errors.append('uuid')
    identity(conversation.get('id'))
    try:
        created = _time(conversation.get('created_at'))
        if conversation.get('updated_at') is not None and _time(conversation['updated_at']) < created:
            errors.append('conversation_time_order')
    except (ValueError, TypeError):
        errors.append('conversation_time')
    for message in conversation['messages']:
        mid = message.get('id')
        identity(mid)
        if mid in seen:
            errors.append('duplicate_message_id')
        seen.add(mid)
        if message.get('conversation_id') != conversation['id']:
            errors.append('conversation_reference')
        try:
            timestamp = _time(message.get('created_at', message.get('timestamp')))
            if previous is not None and timestamp < previous:
                errors.append('message_time_order')
            previous = timestamp
            if message.get('updated_at') is not None and _time(message['updated_at']) < timestamp:
                errors.append('message_update_order')
        except (ValueError, TypeError):
            errors.append('message_time')
    return errors


def validate_expected_graph(graph, public):
    """Comprueba nodos, propiedades y enlaces contra el modelo público validado."""
    import json
    errors = []
    nodes = {node['key']: node for node in graph['nodes']}
    cid = public['id']
    messages = public['messages']
    if set(nodes) != {cid, *(message['id'] for message in messages)}:
        errors.append('nodes')
    conversation = nodes.get(cid, {})
    properties = conversation.get('properties', {})
    if (conversation.get('labels') != ['Conversation'] or properties.get('id') != cid
            or properties.get('session_id') != public['session_id']):
        errors.append('conversation_properties')
    for key in ('created_at', 'updated_at'):
        try:
            if _time(properties.get(key)) != _time(public.get(key)):
                errors.append('conversation_' + key)
        except (ValueError, TypeError):
            errors.append('conversation_' + key)
    expected_edges = []
    ids = [message['id'] for message in messages]
    for message in messages:
        node = nodes.get(message['id'], {})
        properties = node.get('properties', {})
        if node.get('labels') != ['Message']:
            errors.append('message_label')
        for key in ('id', 'content', 'role'):
            if properties.get(key) != message[key]:
                errors.append('message_' + key)
        try:
            metadata = properties.get('metadata', '{}')
            if isinstance(metadata, str):
                metadata = json.loads(metadata)
            if metadata != message['metadata']:
                errors.append('message_metadata')
            if _time(properties.get('timestamp')) != _time(message.get('timestamp', message.get('created_at'))):
                errors.append('message_timestamp')
            if properties.get('embedding') != message.get('embedding'):
                errors.append('message_embedding')
        except (ValueError, TypeError):
            errors.append('message_properties')
        expected_edges.append((cid, message['id'], 'HAS_MESSAGE'))
    if ids:
        expected_edges.append((cid, ids[0], 'FIRST_MESSAGE'))
    expected_edges.extend((a, b, 'NEXT_MESSAGE') for a, b in zip(ids, ids[1:]))
    edges = [(edge['source'], edge['target'], edge['type']) for edge in graph['edges']]
    if sorted(edges) != sorted(expected_edges) or any(edge['properties'] for edge in graph['edges']):
        errors.append('edges')
    return errors


def compare_append_read(returned, observed, *, generated_time_window_seconds=0.0):
    """Exacto por defecto; ventana explícita solo para tiempos no suministrados."""
    from datetime import timedelta
    window = timedelta(seconds=generated_time_window_seconds)
    left, right = deepcopy(returned), deepcopy(observed)
    for value in (left, right):
        if 'timestamp' in value and 'created_at' not in value:
            value['created_at'] = value.pop('timestamp')
        for key in ('created_at', 'updated_at'):
            if value.get(key) is not None:
                value[key] = _time(value[key])
    errors = []
    for key in sorted(set(left) | set(right)):
        if key not in left or key not in right:
            errors.append(key)
        elif (key in ('created_at', 'updated_at')
              and left[key] is not None and right[key] is not None):
            if abs(left[key] - right[key]) > window:
                errors.append(key)
        elif left[key] != right[key]:
            errors.append(key)
    return errors


def validate_complete_message(message, conversation_id):
    """Valida el modelo público real contra campos obligatorios independientes."""
    from uuid import UUID
    expected = {'id', 'conversation_id', 'role', 'content', 'metadata',
                'embedding', 'created_at', 'updated_at', 'tool_calls'}
    value = deepcopy(message)
    if 'timestamp' in value and 'created_at' not in value:
        value['created_at'] = value.pop('timestamp')
    errors = []
    if set(value) != expected:
        errors.append('fields')
    for key in ('id', 'conversation_id'):
        try:
            UUID(value.get(key))
        except (ValueError, TypeError, AttributeError):
            errors.append(key)
    if value.get('conversation_id') != conversation_id:
        errors.append('conversation_reference')
    try:
        created = _time(value.get('created_at'))
        if value.get('updated_at') is not None and _time(value['updated_at']) < created:
            errors.append('time_order')
    except (ValueError, TypeError):
        errors.append('time')
    if (not isinstance(value.get('content'), str)
            or value.get('role') not in ('user', 'assistant', 'system', 'tool')
            or not isinstance(value.get('metadata'), dict)
            or value.get('embedding') is not None or value.get('tool_calls') is not None):
        errors.append('values')
    return errors


def compare_public_messages(reference, rust):
    """Compara el modelo Python real con Rust; solo timestamp es un alias."""
    import json
    left, right = deepcopy(reference), deepcopy(rust)
    if 'timestamp' in right and 'created_at' not in right:
        right['created_at'] = right.pop('timestamp')
    errors = ['missing:' + key for key in sorted(set(left) - set(right))]
    errors.extend('unexpected:' + key for key in sorted(set(right) - set(left)))
    for value in (left, right):
        for key in ('created_at', 'updated_at'):
            if key in value and value[key] is not None:
                _time(value[key])
                value[key] = '<generated-time>'
        for key in ('id', 'conversation_id'):
            if key in value and value[key] is not None:
                value[key] = '<' + key + '>'
    common = set(left) & set(right)
    encode = lambda value: json.dumps({key: value[key] for key in common},
                                     sort_keys=True, allow_nan=False, ensure_ascii=False)
    if encode(left) != encode(right):
        errors.append('message_fields')
    return errors


def compare_scoped_graphs(reference, rust):
    """GM01-DEV-002: valida la extensión de aislamiento antes de compararla."""
    import json
    left, right = canonical_graph(reference), canonical_graph(rust)
    conversations = [[node for node in graph['nodes'] if node['key'] == 'conversation'
                      and node['labels'] == ['Conversation']] for graph in (left, right)]
    if any(len(nodes) != 1 for nodes in conversations):
        return ['owner_scope']
    reference_properties = conversations[0][0]['properties']
    rust_properties = conversations[1][0]['properties']
    if ('owner_scope' in reference_properties
            or rust_properties.get('owner_scope') != 'unowned'):
        return ['owner_scope']
    del rust_properties['owner_scope']
    encode = lambda value: json.dumps(value, sort_keys=True, allow_nan=False, ensure_ascii=False)
    return [] if encode(left) == encode(right) else ['graph_properties']


def normalize_persisted_graph(graph, identities):
    """Conserva propiedades no generadas, incluso extensiones de infraestructura."""
    import json
    result = canonical_graph(graph)
    for node in result['nodes']:
        node['key'] = identities[node['key']]
        node['labels'] = sorted(node['labels'])
        properties = node['properties']
        if 'id' in properties:
            properties['id'] = identities[properties['id']]
        for key in ('timestamp', 'created_at', 'updated_at'):
            if key in properties and properties[key] is not None:
                _time(properties[key])
                properties[key] = '<generated-time>'
        if isinstance(properties.get('metadata'), str):
            properties['metadata'] = json.loads(properties['metadata'])
    for edge in result['edges']:
        edge['source'] = identities[edge['source']]
        edge['target'] = identities[edge['target']]
    return canonical_graph(result)


def canonical_graph(value):
    if not isinstance(value, dict) or set(value) != {'nodes', 'edges'}:
        raise ValueError('Grafo inválido')
    nodes, edges = value['nodes'], value['edges']
    if not isinstance(nodes, list) or not isinstance(edges, list):
        raise ValueError('Colecciones de grafo inválidas')
    keys = set()
    for node in nodes:
        if set(node) != {'key', 'labels', 'properties'} or node['key'] in keys:
            raise ValueError('Nodo inválido o repetido')
        keys.add(node['key'])
    for edge in edges:
        if set(edge) != {'source', 'target', 'type', 'properties'}:
            raise ValueError('Relación inválida')
        if edge['source'] not in keys or edge['target'] not in keys:
            raise ValueError('Enlace colgante')
    return {'nodes': sorted(deepcopy(nodes), key=lambda n: n['key']),
            'edges': sorted(deepcopy(edges), key=lambda e: (e['source'], e['type'], e['target']))}
