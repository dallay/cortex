"""Adaptadores GM-05: oráculo Python fijado y bridge Muninn.

Las respuestas API y el snapshot persistido son observaciones independientes.
Los formatos se normalizan sin completar respuestas desde el grafo.
Una operación ausente se declara con `ContractAbsent`; un campo ausente en una
respuesta se anota en `gaps` y la captura continúa.
"""

from datetime import datetime
import json
from copy import deepcopy

from gm05_capture import ContractAbsent
from gm05_contract import (COMPARABLE_FIELDS, GENERATED_MARKS, KNOWN_FIELDS,
                           OPTIONAL_DEFAULTS, alias_value)

_ABSENT = object()
# Nombres del wire que el contrato llama de otro modo (TCK b6c72c36 exige los del wire).
WIRE_NAMES = {'relationship': {'relationship_type': 'type', 'properties': 'attributes'}}

LABELS = ('Entity', 'Preference', 'Fact')
NODE_FILTER = ' OR '.join(f'n:{label}' for label in LABELS)
SOURCE_FILTER = ' OR '.join(f'a:{label}' for label in LABELS)
OTHER_FILTER = ' OR '.join(f'b:{label}' for label in LABELS)


def _iso(value):
    if value is None:
        return None
    if isinstance(value, datetime):
        return value.isoformat()
    formatter = getattr(value, 'iso_format', None)
    if callable(formatter):
        return formatter()
    return str(value)


def _properties(properties):
    """Convierte tipos temporales del servidor; conserva el resto tal cual."""
    return {key: _iso(value) if isinstance(value, datetime)
            or callable(getattr(value, 'iso_format', None)) else value
            for key, value in properties.items()}


def _metadata(value):
    if value is None:
        return {}
    if isinstance(value, str):
        try:
            return json.loads(value)
        except ValueError:
            return {}
    return dict(value)


def _wire_names(value, category):
    """Traduce los nombres del wire a los del contrato sin inventar campos."""
    aliases = WIRE_NAMES.get(category)
    if not aliases or not isinstance(value, dict):
        return value
    renamed = dict(value)
    for wire, name in aliases.items():
        if wire in renamed:
            renamed[name] = alias_value(renamed, wire, name)
            del renamed[wire]
    return renamed


def _time(value):
    if value is None or isinstance(value, datetime):
        return value
    return datetime.fromisoformat(str(value).replace('Z', '+00:00'))


def _read(value, field):
    return value.get(field) if isinstance(value, dict) else getattr(value, field, None)


def _response_fields(value):
    """Nombres de campo que la respuesta expone realmente.

    Admite mapas, modelos Pydantic v1/v2 y objetos con atributos de instancia o de
    clase. Un atributo de clase público también es un campo de la respuesta: el
    modelo del oráculo puede declararlos allí y no en `__dict__`.
    """
    if isinstance(value, dict):
        return set(value)
    for attribute in ('model_fields', '__fields__'):
        declared = getattr(type(value), attribute, None)
        if declared:
            return set(declared)
    fields = {name for name in vars(value) if not name.startswith('_')}
    for klass in type(value).__mro__:
        for name, descriptor in vars(klass).items():
            if name.startswith('_'):
                continue
            if callable(descriptor) or isinstance(descriptor,
                                                  (property, classmethod, staticmethod)):
                continue
            fields.add(name)
    return fields


def _observe(value, category, gaps, operation):
    """Observa sólo la API: no completa valores ausentes con persistencia.

    Un campo desconocido se registra como brecha explícita; un campo declarado
    como no comparable se descarta a propósito y sólo se conserva si es una
    marca generada, para validarla contra la ventana de la captura.
    """
    if value is None:
        gaps.append(f'{operation}:response_absent')
        return {}
    value = _wire_names(value, category)
    fields = _response_fields(value)
    record = {}
    for field in sorted(fields - KNOWN_FIELDS[category]):
        gaps.append(f'{operation}.{field}:unknown_field')
    identifier = _read(value, 'id') if 'id' in fields else None
    if identifier is None:
        gaps.append(f'{operation}.id:field_absent')
    else:
        record['id'] = str(identifier)
    for field in sorted(COMPARABLE_FIELDS[category]):
        if field not in fields:
            default = OPTIONAL_DEFAULTS.get((category, field), _ABSENT)
            if default is _ABSENT:
                gaps.append(f'{operation}.{field}:field_absent')
            else:
                record[field] = deepcopy(default)
            continue
        item = _read(value, field)
        if field in ('source_id', 'target_id') and item is not None:
            item = str(item)
        elif field in ('metadata', 'attributes'):
            item = _metadata(item)
        elif field in ('created_at', 'updated_at', 'valid_from', 'valid_until'):
            item = _iso(item)
        record[field] = item
    for field in sorted(GENERATED_MARKS[category] - COMPARABLE_FIELDS[category]):
        if field in fields:
            item = _read(value, field)
            if item is not None:
                record[field] = _iso(item)
    return record


class GraphReader:
    """Proyección de solo lectura sobre el Neo4j propio de cada lado."""

    def __init__(self, query):
        self.query = query

    def node(self, label, identifier):
        rows = self.query(f'MATCH (n:{label} {{id: $id}}) RETURN properties(n) AS properties',
                          {'id': identifier})
        if not rows:
            raise ContractAbsent('node_absent')
        return rows[0]['properties']

    def applies_to(self, identifier):
        return [row['id'] for row in self.query(
            'MATCH (:Preference {id: $id})-[:APPLIES_TO]->(e:Entity) RETURN e.id AS id',
            {'id': identifier})]

    def preference(self, identifier):
        properties = self.node('Preference', identifier)
        record = {'id': str(properties['id']),
                  'category': properties.get('category'),
                  'preference': properties.get('preference'),
                  'context': properties.get('context'),
                  'confidence': properties.get('confidence'),
                  'metadata': _metadata(properties.get('metadata')),
                  'linked_entities': self.applies_to(identifier)}
        for field in ('valid_from', 'valid_until'):
            if properties.get(field) is not None:
                record[field] = _iso(properties[field])
        return record

    def fact(self, identifier):
        properties = self.node('Fact', identifier)
        record = {'id': str(properties['id']),
                  'subject': properties.get('subject'),
                  'predicate': properties.get('predicate'),
                  'object': properties.get('object'),
                  'confidence': properties.get('confidence'),
                  'metadata': _metadata(properties.get('metadata'))}
        for field in ('valid_from', 'valid_until'):
            if properties.get(field) is not None:
                record[field] = _iso(properties[field])
        return record

    def relationship(self, identifier):
        rows = self.query(
            'MATCH (a)-[r:RELATED_TO {id: $id}]->(b) '
            'RETURN a.id AS source, b.id AS target, properties(r) AS properties',
            {'id': identifier})
        if not rows:
            raise ContractAbsent('edge_absent')
        row = rows[0]
        properties = row['properties']
        attributes = properties.get('properties', properties.get('attributes'))
        record = {'id': str(properties.get('id', identifier)),
                  'source_id': str(row['source']),
                  'target_id': str(row['target']),
                  'type': alias_value(properties, 'relation_type', 'type'),
                  'description': properties.get('description'),
                  'confidence': properties.get('confidence'),
                  'attributes': _metadata(attributes)}
        for field in ('valid_from', 'valid_until'):
            if properties.get(field) is not None:
                record[field] = _iso(properties[field])
        return record

    def snapshot(self):
        nodes = self.query(
            f'MATCH (n) WHERE {NODE_FILTER} '
            'RETURN labels(n) AS labels, properties(n) AS properties', {})
        edges = self.query(
            f'MATCH (a)-[r]->(b) WHERE ({SOURCE_FILTER}) AND ({OTHER_FILTER}) '
            'RETURN a.id AS source, b.id AS target, type(r) AS type, properties(r) AS properties',
            {})
        return {'nodes': [{'key': str(node['properties'].get('id')),
                           'labels': [label for label in node['labels'] if label in LABELS],
                           'properties': _properties(node['properties'])} for node in nodes],
                'edges': [{'source': str(edge['source']), 'target': str(edge['target']),
                           'type': edge['type'],
                           'properties': {key: _iso(value) for key, value in
                                          edge['properties'].items() if key != 'embedding'}}
                          for edge in edges]}


class ReferenceAdapter:
    """Adapta la API del oráculo fijado; el llamador controla su ciclo de vida.

    `entity_ref` construye el tipo de referencia del oráculo para `applies_to`,
    de modo que este módulo no importe el paquete de referencia.
    """

    def __init__(self, client, graph, entity_ref=None, to_uuid=None):
        self.client = client
        self.graph = graph
        self.entity_ref = entity_ref
        self.to_uuid = to_uuid
        self.gaps = []

    async def call(self, operation, **arguments):
        handler = getattr(self, '_' + operation, None)
        if handler is None:
            raise ContractAbsent('contract_absent')
        return await handler(**arguments)

    async def snapshot(self):
        return self.graph.snapshot()

    async def _add_entity(self, name, type):
        entity, _ = await self.client.long_term.add_entity(
            name, type, resolve=False, deduplicate=False, geocode=False, enrich=False)
        return _observe(entity, 'entity', self.gaps, 'add_entity')

    async def _add_preference(self, category, preference, context, confidence,
                              user_identifier, applies_to=None):
        references = None
        if applies_to:
            if self.entity_ref is None:
                raise ContractAbsent('entity_ref_unavailable')
            references = [self.entity_ref(identifier) for identifier in applies_to]
        result = await self.client.long_term.add_preference(
            category, preference, context=context, confidence=confidence,
            user_identifier=user_identifier, applies_to=references)
        return _observe(result, 'preference', self.gaps, 'add_preference')

    async def _add_fact(self, subject, predicate, obj, confidence, valid_from, valid_until):
        result = await self.client.long_term.add_fact(
            subject, predicate, obj, confidence=confidence,
            valid_from=_time(valid_from), valid_until=_time(valid_until))
        return _observe(result, 'fact', self.gaps, 'add_fact')

    async def _add_relationship(self, source_id, target_id, type, description, confidence,
                                valid_from, valid_until, attributes):
        source = self.to_uuid(source_id) if self.to_uuid else source_id
        target = self.to_uuid(target_id) if self.to_uuid else target_id
        result = await self.client.long_term.add_relationship(
            source, target, type, description=description, confidence=confidence,
            valid_from=_time(valid_from), valid_until=_time(valid_until),
            attributes=attributes)
        return _observe(result, 'relationship', self.gaps, 'add_relationship')

    async def _supersede_preference(self, old_id, new_id):
        await self.client.long_term.supersede_preference(old_id, new_id)
        return None

    async def _get_preferences_for(self, user_identifier, active_only=True):
        found = await self.client.long_term.get_preferences_for(
            user_identifier, active_only=active_only)
        return [_observe(item, 'preference', self.gaps, 'get_preferences_for') for item in found]

    async def _get_facts_about(self, subject, limit=10):
        found = await self.client.long_term.get_facts_about(subject, limit=limit)
        return [_observe(item, 'fact', self.gaps, 'get_facts_about') for item in found]

    async def _get_related_entities(self, entity_id):
        found = await self.client.long_term.get_related_entities(entity_id)
        return [_observe(item[0] if isinstance(item, tuple) else item, 'entity',
                         self.gaps, 'get_related_entities') for item in found]


class BridgeAdapter:
    """Adapta el bridge HTTP de Muninn; una operación no publicada es una brecha."""

    ARGUMENTS = {
        'add_entity': ('name', 'entity_type'),
        'add_preference': ('category', 'preference', 'context', 'confidence',
                           'user_identifier', 'applies_to'),
        'add_fact': ('subject', 'predicate', 'obj', 'confidence', 'valid_from', 'valid_until'),
        'add_relationship': ('source_id', 'target_id', 'relationship_type', 'description',
                             'confidence', 'valid_from', 'valid_until', 'properties'),
        'get_facts_about': ('subject', 'limit'),
        'supersede_preference': ('old_id', 'new_id'),
        'get_preferences_for': ('user_identifier', 'active_only'),
        'get_related_entities': ('entity_id',),
    }

    def __init__(self, post, graph):
        self.post = post
        self.graph = graph
        self.gaps = []

    async def call(self, operation, **arguments):
        if operation not in self.ARGUMENTS:
            raise ContractAbsent('contract_absent')
        payload = self._payload(operation, arguments)
        try:
            response = await self.post(operation, payload)
        except LookupError as error:
            raise ContractAbsent('contract_absent') from error
        return self._project(operation, response)

    async def snapshot(self):
        return self.graph.snapshot()

    def _payload(self, operation, arguments):
        names = self.ARGUMENTS[operation]
        if operation == 'add_entity':
            return {'name': arguments['name'], 'entity_type': arguments['type']}
        if operation == 'add_relationship':
            payload = {'source_id': arguments['source_id'],
                       'target_id': arguments['target_id'],
                       'relationship_type': arguments['type'],
                       'properties': dict(arguments.get('attributes') or {})}
            for name in ('description', 'confidence', 'valid_from', 'valid_until'):
                if arguments.get(name) is not None:
                    payload[name] = arguments[name]
            return payload
        return {name: arguments[name] for name in names
                if name in arguments and arguments[name] is not None}

    def _project(self, operation, response):
        if operation == 'supersede_preference':
            return None
        category = {'add_entity': 'entity', 'add_preference': 'preference',
                    'get_preferences_for': 'preference', 'add_fact': 'fact',
                    'get_facts_about': 'fact', 'add_relationship': 'relationship',
                    'get_related_entities': 'entity'}[operation]
        if operation.startswith('get_'):
            return [_observe(item, category, self.gaps, operation) for item in response]
        return _observe(response, category, self.gaps, operation)
