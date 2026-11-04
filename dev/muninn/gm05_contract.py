"""Contrato de comparación GM-05: preferencias, hechos, relaciones y supersesión.

Reglas de normalización, todas explícitas:

- cada sección del documento corresponde a una operación; una operación ausente se
  reporta como brecha y nunca se confunde con una lista vacía;
- los identificadores los genera cada servidor: se validan (presentes y únicos por
  sección) y se usan sólo como token estable de diagnóstico (`<categoría-sha256>`),
  derivado del identificador real y no del orden de aparición; nunca se comparan
  entre servidores;
- cada elemento se compara por identidad canónica (`IDENTITY_FIELDS`) y no por la
  posición que ocupó en la respuesta; cada sección se ordena por esa identidad
  antes de comparar, de modo que el orden de la respuesta no se compara;
- las referencias (extremos de una relación y listas de supersesión) se resuelven
  a la etiqueta de identidad canónica del elemento cuando el documento la permite,
  y a un token estable en caso contrario;
- las marcas generadas por el servidor (`created_at`, `updated_at` y la validez de
  preferencia) se normalizan a `<generated-time>` mediante un marcador de presencia:
  su valor no se compara y su presencia se valida contra la ventana de la captura
  cuando el llamador la provee;
- las cotas de validez suministradas por el llamador (hechos y relaciones) se
  comparan exactas después de normalizar la zona horaria;
- ningún campo desconocido se ignora: bloquea la comparación;
- `compare_gm05` no lanza: una anomalía produce veredicto `fail` con su causa.

La única desviación autorizada (`GM05-DEV-001`) describe una asimetría de
persistencia observada, no una tolerancia: se aplica sólo cuando la asimetría se
cumple exactamente. Si no se observa, la comparación estricta decide el resultado.
"""

from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json

from gm05_capture import RELATIONSHIP

SECTIONS = frozenset({
    'preferences', 'preferences_for', 'facts', 'facts_about', 'relationships', 'related',
    'deduplication', 'supersession', 'graph', 'gaps',
})
LIST_SECTIONS = {
    'preferences': 'preference', 'preferences_for': 'preference',
    'facts': 'fact', 'facts_about': 'fact',
    'relationships': 'relationship', 'related': 'entity',
}

# Campos que ambos servidores deben producir iguales. El `id` no está aquí porque
# cada servidor genera el suyo; su presencia y unicidad se validan aparte.
COMPARABLE_FIELDS = {
    'preference': frozenset({'category', 'preference', 'context', 'confidence', 'metadata'}),
    'fact': frozenset({'subject', 'predicate', 'object', 'confidence', 'metadata'}),
    'relationship': frozenset({'source_id', 'target_id', 'type', 'description',
                               'confidence', 'attributes', 'valid_from', 'valid_until'}),
    'entity': frozenset({'name', 'type', 'description'}),
}

# Campos que la respuesta puede traer y que no se comparan entre servidores.
# Cada entrada justifica la decisión con la línea del wire o del modelo del oráculo.
NON_COMPARABLE_FIELDS = {
    'preference': {
        'id': 'identificador generado por cada servidor (core/memory.py:45; wire.rs:181); '
              'se valida su presencia y unicidad pero no se compara',
        'created_at': 'marca generada por el servidor (core/memory.py:46 MemoryEntry.created_at); '
                      'el wire de preferencia no la emite (wire.rs:179-191)',
        'updated_at': 'marca generada por el servidor (core/memory.py:47); instante distinto por servidor',
        'embedding': 'vector calculado por el servidor (core/memory.py:48; wire.rs:189)',
        'linked_entities': 'artefacto del modelo del oráculo siempre vacío en la ruta de creación '
                           '(long_term.py:403); el vínculo se verifica en el grafo (APPLIES_TO)',
        'valid_from': 'el oráculo no la expone en la respuesta (long_term.py:396-403) y Muninn la '
                      'genera al crear (long_term.rs:577); se valida contra la ventana y se '
                      'compara en el grafo',
        'valid_until': 'el oráculo no la expone en la respuesta; Muninn la fija al superseder '
                       '(long_term.rs:605); se valida contra la ventana y se compara en el grafo',
    },
    'fact': {
        'id': 'identificador generado por cada servidor (core/memory.py:45; wire.rs:195)',
        'created_at': 'marca generada por el servidor (core/memory.py:46); wire.rs:193-204 no la emite',
        'updated_at': 'marca generada por el servidor (core/memory.py:47)',
        'embedding': 'vector calculado por el servidor (core/memory.py:48; wire.rs:203)',
        'source_id': 'mensaje o documento de origen (long_term.py:413); el escenario no lo fija',
        'valid_from': 'el oráculo la persiste en el grafo pero su `get_facts_about` no la '
                      'expone: la respuesta vuelve a generar el instante de lectura '
                      '(long_term.py:1145-1195 escribe `str(datetime)` y la relectura no lo '
                      'convierte a temporal). Medido con `probe-oracle-validity.py`: el grafo '
                      'guarda 2026-01-01 y la API devuelve ahora. Muninn devuelve el valor '
                      'recibido, así que la API no es una superficie diferencial válida; se '
                      'compara en el grafo',
        'valid_until': 'mismo motivo que `valid_from`: el oráculo la persiste como texto en '
                       'el grafo y no la expone por la API; se compara en el grafo',
    },
    'relationship': {
        'id': 'identificador generado por cada servidor (core/memory.py:45; wire.rs:209)',
        'created_at': 'marca generada por el servidor (core/memory.py:46); wire.rs:207-219 no la emite',
        'updated_at': 'marca generada por el servidor (core/memory.py:47)',
        'embedding': 'vector calculado por el servidor (core/memory.py:48)',
        'metadata': 'campo heredado de MemoryEntry (core/memory.py:49) que el escenario no fija '
                    'y el wire de relaciones no emite',
    },
    'entity': {
        'id': 'identificador generado por cada servidor (core/memory.py:45; wire.rs:165)',
        'created_at': 'marca generada por el servidor (core/memory.py:46); wire.rs:172 la emite '
                      'pero es un instante distinto por servidor',
        'subtype': 'clasificación adicional del oráculo (long_term.py:349) que el escenario no fija',
        'canonical_name': 'nombre resuelto por el oráculo (long_term.py:347); el escenario crea sin resolver',
        'embedding': 'vector calculado por el servidor (core/memory.py:48; wire.rs:170)',
        'updated_at': 'marca generada por el servidor (core/memory.py:47; wire.rs:173)',
        'confidence': 'confianza de extracción del oráculo (long_term.py:353; wire.rs:174), no del escenario',
        'metadata': 'campo heredado de MemoryEntry (core/memory.py:49) que el escenario no fija',
        'aliases': 'alias de resolución del oráculo (long_term.py:354)',
        'attributes': 'atributos libres de entidad (long_term.py:355) que el escenario no fija',
        'source_id': 'mensaje o documento de origen (long_term.py:358)',
    },
}

# Marcas que cada servidor genera al escribir: no se comparan entre servidores.
# Su presencia se valida contra la ventana de la captura cuando el llamador la provee.
GENERATED_MARKS = {
    'preference': frozenset({'created_at', 'updated_at', 'valid_from', 'valid_until'}),
    'fact': frozenset({'created_at', 'updated_at'}),
    'relationship': frozenset({'created_at', 'updated_at'}),
    'entity': frozenset({'created_at', 'updated_at'}),
}

# Campos que identifican un elemento dentro de su sección. Se usan para ordenar y
# comparar colecciones por identidad canónica y no por posición.
IDENTITY_FIELDS = {
    'preference': ('category', 'preference'),
    'fact': ('subject', 'predicate', 'object'),
    'relationship': ('type', 'source_id', 'target_id'),
    'entity': ('name', 'type'),
}

# Campos comparables que pueden faltar y se normalizan de forma simétrica: uno
# ausente y otro vacío son la misma observación (GM05-010).
OPTIONAL_DEFAULTS = {('relationship', 'attributes'): {}}

KNOWN_FIELDS = {category: COMPARABLE_FIELDS[category] | frozenset(NON_COMPARABLE_FIELDS[category])
                for category in COMPARABLE_FIELDS}
DEDUPLICATION_BOOLEANS = ('preference_same_id', 'preference_flag', 'fact_same_id', 'fact_flag')
DEDUPLICATION_FIELDS = frozenset(DEDUPLICATION_BOOLEANS)
SUPERSESSION_BOOLEANS = ('edge', 'old_valid_until_set', 'repeat_is_noop')
SUPERSESSION_FIELDS = frozenset({*SUPERSESSION_BOOLEANS, 'active_only', 'history'})
NODE_FIELDS = frozenset({'key', 'labels', 'properties'})
EDGE_FIELDS = frozenset({'source', 'target', 'type', 'properties'})
LABEL_CATEGORIES = {'Preference': 'preference', 'Fact': 'fact', 'Entity': 'entity'}

# Superficie observable del grafo, derivada de las consultas reales:
# `crates/muninn/neo4j/src/queries/long_term.rs:157-180`, `:267-300`, `:562-575`
# y `dev/muninn/.cache/sources/agent-memory/src/neo4j_agent_memory/graph/queries.py:307-330`,
# `:387-400`, `:416-430`, `:434-460`.
GRAPH_COMPARABLE = {
    'entity': frozenset({'name', 'type', 'description'}),
    'preference': frozenset({'category', 'preference'}),
    'fact': frozenset({'subject', 'predicate', 'object', 'confidence', 'valid_from', 'valid_until'}),
}
GRAPH_GENERATED = frozenset({'created_at', 'updated_at'})
GRAPH_PREFERENCE_VALIDITY = frozenset({'valid_from', 'valid_until'})
GRAPH_IGNORED = {
    'entity': frozenset({'id', 'subtype', 'canonical_name', 'embedding', 'confidence', 'metadata',
                         'aliases', 'name_key', 'surface_keys', 'location', 'name_keys'}),
    'preference': frozenset({'id', 'context', 'confidence', 'metadata', 'embedding'}),
    'fact': frozenset({'id', 'metadata', 'embedding', 'source_id'}),
}
GRAPH_EDGE_COMPARABLE = frozenset({'description', 'confidence', 'valid_from', 'valid_until'})
GRAPH_EDGE_IGNORED = frozenset({'id', 'support', 'derived', 'extractor', 'source_message_ids',
                               'evidence', 'metadata', 'embedding', 'created_at', 'updated_at'})

AUTHORIZED_DEVIATIONS = frozenset({'GM05-DEV-001'})


class ContractError(ValueError):
    """La captura no satisface el contrato GM-05."""


_ABSENT = object()


def _time(value, path):
    if not isinstance(value, str):
        raise ContractError(f'{path}: timestamp inválido')
    try:
        parsed = datetime.fromisoformat(value.replace('Z', '+00:00'))
    except ValueError as error:
        raise ContractError(f'{path}: timestamp inválido') from error
    if parsed.utcoffset() is None:
        raise ContractError(f'{path}: timestamp sin zona')
    return parsed.astimezone(timezone.utc)


def _bound(value, name):
    if value is None:
        return None
    if isinstance(value, datetime):
        if value.utcoffset() is None:
            raise ContractError(f'window.{name}: timestamp sin zona')
        return value.astimezone(timezone.utc)
    return _time(value, f'window.{name}')


def _window(window):
    """Normaliza la ventana de la captura; `None` desactiva la validación."""
    if window is None:
        return None
    if not isinstance(window, (tuple, list)) or len(window) != 2:
        raise ContractError('window: ventana inválida')
    start, end = _bound(window[0], 'inicio'), _bound(window[1], 'fin')
    if start is not None and end is not None and end < start:
        raise ContractError('window: ventana invertida')
    return start, end


def _check_window(moment, path, window):
    if window is None:
        return
    start, end = window
    if start is not None and moment < start:
        raise ContractError(f'{path}: fuera de la ventana')
    if end is not None and moment > end:
        raise ContractError(f'{path}: fuera de la ventana')


class _Identities:
    """Tokens estables por categoría, derivados del identificador real.

    El token no se compara entre servidores: cada servidor genera identificadores
    distintos para el mismo elemento. Sirve para diagnósticos y como respaldo de
    referencias que el documento no permite resolver. Derivarlo del identificador
    real evita que un campo ausente desplace los diagnósticos del otro lado.
    """

    def __init__(self):
        self._seen = {}

    def token(self, category, value):
        digest = hashlib.sha256(value.encode('utf-8')).hexdigest()[:12]
        return f'<{category}-{digest}>'

    def declare(self, category, value, path, scope):
        if not isinstance(value, str) or not value.strip():
            raise ContractError(f'{path}: identidad inválida')
        seen = self._seen.setdefault((category, scope), set())
        if value in seen:
            raise ContractError(f'{path}: duplicate {category} id')
        seen.add(value)
        return self.token(category, value)


def _fields(value, allowed, path):
    """Un campo desconocido bloquea; uno ausente se reporta como diferencia localizada."""
    if not isinstance(value, dict):
        raise ContractError(f'{path}: objeto inválido')
    unknown = set(value) - set(allowed)
    if unknown:
        raise ContractError(f'{path}.{sorted(unknown)[0]}: campo desconocido')


def _generated_time(value, path, window):
    if value is None:
        return None
    _check_window(_time(value, path), path, window)
    return '<generated-time>'


def _validity(value, path):
    return None if value is None else _time(value, path).isoformat()


def _object(value, path):
    """Normaliza un objeto libre: admite mapa o JSON serializado."""
    if value is None:
        return {}
    if isinstance(value, str):
        try:
            value = json.loads(value)
        except ValueError as error:
            raise ContractError(f'{path}: objeto inválido') from error
    if not isinstance(value, dict):
        raise ContractError(f'{path}: objeto inválido')
    return deepcopy(value)


def _interval(item, path):
    start, end = item.get('valid_from'), item.get('valid_until')
    if start is not None and end is not None:
        if _time(end, path + '.valid_until') < _time(start, path + '.valid_from'):
            raise ContractError(f'{path}: interval invertido')


def _entity_label(name, entity_type):
    return f'<entity:{name!r}:{entity_type!r}>'


def _preference_label(category, preference):
    return f'<preference:{category!r}:{preference!r}>'


def _fact_label(subject, predicate, obj):
    return f'<fact:{subject!r}:{predicate!r}:{obj!r}>'


def _add_entity_label(labels, item):
    if not isinstance(item, dict) or item.get('id') is None:
        return
    labels[str(item['id'])] = _entity_label(item.get('name'), item.get('type'))


def _entity_labels(document):
    """Etiquetas canónicas de entidad: sección `related` y nodos del grafo.

    Se usa la identidad (nombre, tipo) y no el identificador porque cada servidor
    genera el suyo. El grafo del oráculo añade `:Entity:<TYPE>` de forma dinámica
    (`graph/queries.py:302-308`), así que sólo la etiqueta base identifica la
    categoría.
    """
    labels = {}
    if not isinstance(document, dict):
        return labels
    for item in document.get('related') or ():
        _add_entity_label(labels, item)
    graph = document.get('graph')
    nodes = graph.get('nodes') if isinstance(graph, dict) else None
    for node in nodes or ():
        if not isinstance(node, dict):
            continue
        raw_labels = node.get('labels') or ()
        if not raw_labels or raw_labels[0] != 'Entity':
            continue
        _add_entity_label(labels, node.get('properties'))
    return labels


def _preference_labels(document):
    labels = {}
    if not isinstance(document, dict):
        return labels
    for section in ('preferences', 'preferences_for'):
        for item in document.get(section) or ():
            if isinstance(item, dict) and item.get('id') is not None:
                labels.setdefault(str(item['id']),
                                  _preference_label(item.get('category'), item.get('preference')))
    return labels


def _reference(value, path, identities, labels, category='entity'):
    """Etiqueta de identidad canónica; token estable si el documento no la resuelve."""
    if not isinstance(value, str) or not value.strip():
        raise ContractError(f'{path}: identidad inválida')
    return labels.get(value) or identities.token(category, value)


def comparable_key(value):
    """Stable, type-preserving ordering of normalized JSON observations."""
    return json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(',', ':'),
                      allow_nan=False)


def alias_value(value, first, second):
    """Read an alias pair without concealing contradictory values (including null)."""
    if first in value and second in value:
        if comparable_key(value[first]) != comparable_key(value[second]):
            raise ContractError(f'{first}/{second}: conflicting aliases')
    return value[first] if first in value else value.get(second)


def _scalar(value):
    return '' if value is None else str(value)


def _identity_key(category, item):
    return tuple(_scalar(item.get(field)) for field in IDENTITY_FIELDS[category])


def _item(category, raw, path, scope, identities, labels, window):
    _fields(raw, KNOWN_FIELDS[category], path)
    if raw.get('id') is None:
        raise ContractError(f'{path}.id: identidad ausente')
    identities.declare(category, raw['id'], path + '.id', scope)
    item = {}
    for field in sorted(COMPARABLE_FIELDS[category]):
        if raw.get(field) is None:
            default = OPTIONAL_DEFAULTS.get((category, field), _ABSENT)
            if default is not _ABSENT:
                item[field] = deepcopy(default)
            continue
        value = raw[field]
        if field in GENERATED_MARKS[category]:
            item[field] = _generated_time(value, f'{path}.{field}', window)
        elif field in ('valid_from', 'valid_until'):
            item[field] = _validity(value, f'{path}.{field}')
        elif field in ('source_id', 'target_id'):
            item[field] = _reference(value, f'{path}.{field}', identities, labels)
        elif field in ('metadata', 'attributes'):
            item[field] = _object(value, f'{path}.{field}')
        else:
            item[field] = value
    if category == 'entity':
        for field in ('name', 'type'):
            if not isinstance(item.get(field), str) or not item[field].strip():
                raise ContractError(f'{path}.{field}: texto requerido')
        if item.get('description') is not None and not isinstance(item['description'], str):
            raise ContractError(f'{path}.description: texto requerido')
    if category in ('fact', 'relationship'):
        _interval(item, path)
    for field in sorted(GENERATED_MARKS[category] - COMPARABLE_FIELDS[category]):
        if raw.get(field) is not None:
            _generated_time(raw[field], f'{path}.{field}', window)
    return item


def _items(values, section, identities, labels, window):
    """Valida una sección de resultados; cada sección es una operación distinta."""
    category = LIST_SECTIONS[section]
    if not isinstance(values, list):
        raise ContractError(f'{section}: colección inválida')
    items = [_item(category, raw, f'{section}[{index}]', section, identities, labels, window)
             for index, raw in enumerate(values)]
    items.sort(key=lambda item: (_identity_key(category, item), comparable_key(item)))
    return items


def _deduplication(value):
    if value is None:
        return None
    _fields(value, DEDUPLICATION_FIELDS, 'deduplication')
    for field in DEDUPLICATION_BOOLEANS:
        if field not in value:
            raise ContractError(f'deduplication.{field}: campo ausente')
        if not isinstance(value[field], bool):
            raise ContractError(f'deduplication.{field}: booleano requerido')
    return deepcopy(value)


def _identity_list(values, identities, path, labels):
    if not isinstance(values, list):
        raise ContractError(f'{path}: lista inválida')
    return sorted(_reference(value, f'{path}[{index}]', identities, labels, 'preference')
                  for index, value in enumerate(values))


def _supersession(value, identities, preference_labels):
    if value is None:
        return None
    _fields(value, SUPERSESSION_FIELDS, 'supersession')
    result = deepcopy(value)
    for field in SUPERSESSION_BOOLEANS:
        if field not in result:
            raise ContractError(f'supersession.{field}: campo ausente')
        if not isinstance(result[field], bool):
            raise ContractError(f'supersession.{field}: booleano requerido')
    for field in ('active_only', 'history'):
        if field not in result:
            raise ContractError(f'supersession.{field}: campo ausente')
        result[field] = _identity_list(result[field], identities, f'supersession.{field}',
                                       preference_labels)
    return result


def _node_identity(category, properties):
    if category == 'entity':
        return _entity_label(properties.get('name'), properties.get('type'))
    if category == 'preference':
        return _preference_label(properties.get('category'), properties.get('preference'))
    return _fact_label(properties.get('subject'), properties.get('predicate'),
                       properties.get('object'))


def _node_properties(category, properties, path, window):
    """Compara solo los campos declarados; un campo no declarado bloquea.

    Las etiquetas dinámicas del oráculo (`:Entity:<TYPE>`) no se comparan: la
    categoría la fija la etiqueta base y el subtipo vive en `properties.type`, que
    sí es comparable.
    """
    known = GRAPH_COMPARABLE[category] | GRAPH_IGNORED[category] | GRAPH_GENERATED
    if category == 'preference':
        known |= GRAPH_PREFERENCE_VALIDITY
    unknown = set(properties) - known
    if unknown:
        raise ContractError(f'{path}.{sorted(unknown)[0]}: campo desconocido')
    result = {}
    for field in sorted(GRAPH_COMPARABLE[category]):
        if properties.get(field) is not None:
            value = properties[field]
            result[field] = (_validity(value, f'{path}.{field}')
                             if field in ('valid_from', 'valid_until') else value)
    for field in sorted(GRAPH_GENERATED):
        if properties.get(field) is not None:
            _generated_time(properties[field], f'{path}.{field}', window)
    if category == 'preference':
        for field in sorted(GRAPH_PREFERENCE_VALIDITY):
            if properties.get(field) is None:
                result[field] = None
            else:
                _generated_time(properties[field], f'{path}.{field}', window)
                result[field] = '<generated-time>'
    return result


def _edge_properties(properties, path):
    if not isinstance(properties, dict):
        raise ContractError(f'{path}: objeto inválido')
    known = GRAPH_EDGE_COMPARABLE | GRAPH_EDGE_IGNORED | {'type', 'relation_type', 'properties'}
    unknown = set(properties) - known
    if unknown:
        raise ContractError(f'{path}.{sorted(unknown)[0]}: campo desconocido')
    result = {}
    for field in sorted(GRAPH_EDGE_COMPARABLE):
        if properties.get(field) is not None:
            value = properties[field]
            result[field] = (_validity(value, f'{path}.{field}')
                             if field in ('valid_from', 'valid_until') else value)
    relation = alias_value(properties, 'relation_type', 'type')
    if relation is not None:
        result['relation'] = relation
    if 'properties' in properties:
        result['properties'] = _object(properties['properties'], path + '.properties')
    return result


def _graph(value, identities, window, labels):
    if not isinstance(value, dict) or set(value) != {'nodes', 'edges'}:
        raise ContractError('graph: grafo inválido')
    nodes = []
    node_labels = dict(labels)
    present = set()
    for index, raw in enumerate(value['nodes']):
        path = f'graph.nodes[{index}]'
        if not isinstance(raw, dict) or set(raw) != NODE_FIELDS:
            raise ContractError(f'{path}: nodo inválido')
        if not isinstance(raw['labels'], list) or not raw['labels']:
            raise ContractError(f'{path}.labels: etiquetas inválidas')
        category = LABEL_CATEGORIES.get(raw['labels'][0])
        if category is None:
            raise ContractError(f'{path}.labels: etiqueta desconocida')
        if not isinstance(raw['properties'], dict):
            raise ContractError(f'{path}.properties: objeto inválido')
        identifier = raw['properties'].get('id')
        if identifier is None:
            raise ContractError(f'{path}.properties.id: identidad ausente')
        identity = _node_identity(category, raw['properties'])
        node_labels[str(identifier)] = identity
        present.add(str(identifier))
        nodes.append({'key': identity, 'labels': [raw['labels'][0]],
                      'properties': _node_properties(category, raw['properties'],
                                                     f'{path}.properties', window)})
    keys = [node['key'] for node in nodes]
    if len(set(keys)) != len(keys):
        raise ContractError('graph.nodes: nodo repetido')
    edges = []
    for index, raw in enumerate(value['edges']):
        path = f'graph.edges[{index}]'
        if not isinstance(raw, dict) or set(raw) != EDGE_FIELDS:
            raise ContractError(f'{path}: relación inválida')
        if not isinstance(raw['type'], str) or not raw['type'].strip():
            raise ContractError(f'{path}.type: tipo inválido')
        if str(raw['source']) not in present or str(raw['target']) not in present:
            raise ContractError(f'{path}: enlace colgante')
        edges.append({'source': _reference(raw['source'], f'{path}.source', identities, node_labels),
                      'target': _reference(raw['target'], f'{path}.target', identities, node_labels),
                      'type': raw['type'],
                      'properties': _edge_properties(raw['properties'], f'{path}.properties')})
    nodes.sort(key=lambda node: node['key'])
    edges.sort(key=lambda edge: (edge['source'], edge['type'], edge['target'],
                                 comparable_key(edge)))
    return {'nodes': nodes, 'edges': edges}


def canonical_gm05(document, window=None):
    """Valida y normaliza una captura GM-05 completa."""
    if not isinstance(document, dict):
        raise ContractError('documento inválido')
    unknown = set(document) - SECTIONS
    if unknown:
        raise ContractError('secciones desconocidas: ' + ','.join(sorted(unknown)))
    missing = SECTIONS - set(document)
    if missing:
        raise ContractError('secciones ausentes: ' + ','.join(sorted(missing)))
    bounds = _window(window)
    identities = _Identities()
    labels = _entity_labels(document)
    preference_labels = _preference_labels(document)
    result = {section: _items(document[section], section, identities, labels, bounds)
              for section in LIST_SECTIONS}
    result['deduplication'] = _deduplication(document['deduplication'])
    result['supersession'] = _supersession(document['supersession'], identities,
                                          preference_labels)
    result['graph'] = _graph(document['graph'], identities, bounds, labels)
    gaps = document['gaps']
    if not isinstance(gaps, list) or any(not isinstance(gap, str) or not gap for gap in gaps):
        raise ContractError('gaps: brechas inválidas')
    result['gaps'] = sorted(set(gaps))
    return result


def _join(path, key):
    return f'{path}.{key}' if path else key


def _diff(left, right, path, errors):
    if type(left) is not type(right):
        errors.append(f'{path}: tipo distinto')
        return
    if isinstance(left, dict):
        for key in sorted(set(left) | set(right)):
            if key not in left:
                errors.append(f'{_join(path, key)}: falta en referencia')
            elif key not in right:
                errors.append(f'{_join(path, key)}: falta en muninn')
            else:
                _diff(left[key], right[key], _join(path, key), errors)
        return
    if isinstance(left, list):
        if len(left) != len(right):
            errors.append(f'{path}: longitud distinta')
        for index in range(min(len(left), len(right))):
            _diff(left[index], right[index], f'{path}[{index}]', errors)
        return
    if left != right:
        errors.append(f'{path}: valor distinto')


def _apply_gm05_dev_001(reference, muninn):
    """Clasifica GM05-DEV-001 como aplicada, ausente o corrupta.

    El resultado es `(status, problems)`. La ausencia de asimetría no es un error:
    en ese caso no se modifica el snapshot y la comparación estricta decide.
    """
    expected = RELATIONSHIP['attributes']
    for side, document in (('referencia', reference), ('muninn', muninn)):
        for item in document['relationships']:
            if item.get('attributes') != expected:
                return 'invalid', [
                    f'gm05_dev_001: {side} no reemite los attributes del escenario']

    endpoints = {(item['source_id'], item['target_id']) for item in muninn['relationships']}
    muninn_blob_edges = []
    for edge in muninn['graph']['edges']:
        blob = edge['properties'].get('properties')
        if blob is None:
            continue
        if edge['type'] != 'RELATED_TO' or (edge['source'], edge['target']) not in endpoints:
            return 'invalid', ['gm05_dev_001: la arista con blob no corresponde a la relación']
        if blob != expected:
            return 'invalid', ['gm05_dev_001: el blob no coincide con los attributes del escenario']
        muninn_blob_edges.append(edge)

    reference_persists = any('properties' in edge['properties']
                             for edge in reference['graph']['edges'])
    if reference_persists or not muninn_blob_edges:
        return 'absent', []
    if len(muninn_blob_edges) != len(endpoints):
        return 'absent', []

    for edge in muninn_blob_edges:
        del edge['properties']['properties']
    return 'applied', []


def compare_gm05(reference, muninn, deviations=(), window=None):
    """Compara dos capturas GM-05; nunca lanza.

    Una anomalía inesperada produce veredicto `fail` con `error: <tipo>: <mensaje>`
    para que el runner siempre pueda publicar un resultado. Las brechas bloquean
    pero no ocultan el resto de las diferencias.
    """
    try:
        return _compare(reference, muninn, deviations, window)
    except Exception as error:  # noqa: BLE001 - el contrato nunca propaga excepciones
        return {'comparison': 'fail', 'deviations': [],
                'errors': [f'error: {type(error).__name__}: {error}']}


def _compare(reference, muninn, deviations, window):
    unknown = sorted(set(deviations) - AUTHORIZED_DEVIATIONS)
    if unknown:
        return {'comparison': 'fail', 'deviations': [],
                'errors': [f'{name.lower().replace("-", "_")}: desviación desconocida'
                           for name in unknown]}
    errors = []
    for side, document in (('referencia', reference), ('muninn', muninn)):
        if not isinstance(document, dict):
            return {'comparison': 'fail', 'deviations': [],
                    'errors': [f'{side}: documento inválido']}
        declared = document.get('gaps')
        if declared is None:
            continue
        if not isinstance(declared, list):
            return {'comparison': 'fail', 'deviations': [],
                    'errors': [f'{side}: gaps inválidos']}
        errors.extend(f'gap:{gap}' for gap in declared)
    canonical = {}
    structural = []
    for side, document in (('referencia', reference), ('muninn', muninn)):
        try:
            canonical[side] = canonical_gm05(document, window)
        except ContractError as error:
            structural.append(f'{side}: {error}')
    if structural:
        return {'comparison': 'fail', 'deviations': [], 'errors': errors + structural}
    left, right = canonical['referencia'], canonical['muninn']
    left.pop('gaps'), right.pop('gaps')
    applied = []
    if 'GM05-DEV-001' in deviations:
        status, problems = _apply_gm05_dev_001(left, right)
        if status == 'invalid':
            errors.extend(problems)
        elif status == 'applied':
            applied.append('GM05-DEV-001')
    _diff(left, right, '', errors)
    errors = sorted(set(errors))
    return {'comparison': 'fail' if errors else 'pass',
            'deviations': applied, 'errors': errors}
