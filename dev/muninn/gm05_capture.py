"""Captura GM-05: el mismo escenario fijo contra el oráculo y contra Muninn.

Una operación ausente no aborta la captura: se registra en `gaps` con la causa y
la sección dependiente queda vacía. Así la primera medición sirve aunque el
producto todavía no tenga la superficie completa.
"""

from copy import deepcopy

USER = 'gm05-user'
PREFERENCE_ONE = {'category': 'gm05-food', 'preference': 'prefiere cafe sin azucar',
                  'context': 'manana', 'confidence': 0.8}
PREFERENCE_TWO = {'category': 'gm05-food', 'preference': 'prefiere te verde',
                  'context': 'tarde', 'confidence': 0.6}
FACT_ONE = {'subject': 'gm05-ana', 'predicate': 'vive_en', 'obj': 'Madrid', 'confidence': 0.9,
            'valid_from': '2026-02-01T00:00:00+00:00', 'valid_until': None}
FACT_DUPLICATE = {**FACT_ONE, 'confidence': 0.7}
FACT_TWO = {'subject': 'gm05-ana', 'predicate': 'nacio_en', 'obj': 'Sevilla', 'confidence': 0.6,
            'valid_from': None, 'valid_until': None}
FACT_TEMPORAL = {'subject': 'gm05-ana', 'predicate': 'trabajo_en', 'obj': 'ACME',
                 'confidence': 0.95, 'valid_from': '2026-01-01T00:00:00+00:00',
                 'valid_until': '2026-06-01T00:00:00+00:00'}
RELATIONSHIP = {'type': 'WORKS_AT', 'description': 'trabaja en', 'confidence': 0.75,
                'valid_from': '2026-03-01T00:00:00+00:00', 'valid_until': None,
                'attributes': {'role': 'cto'}}
SOURCE_ENTITY = ('gm05-ana', 'PERSON')
TARGET_ENTITY = ('gm05-acme', 'ORGANIZATION')


class ContractAbsent(RuntimeError):
    """La operación o el campo no existe en este lado de la comparación."""

    def __init__(self, reason='contract_absent'):
        super().__init__(reason)
        self.reason = reason


async def _attempt(adapter, operation, fallback, gaps, **arguments):
    try:
        return await adapter.call(operation, **arguments)
    except ContractAbsent as error:
        gaps.append(f'{operation}:{error.reason}')
        return fallback


async def _snapshot(adapter, gaps):
    try:
        return await adapter.snapshot()
    except ContractAbsent as error:
        gaps.append(f'snapshot:{error.reason}')
        return None


def _validity_view(items):
    return [(item['id'], item.get('valid_until')) for item in items or ()]


def _deduplication_flags(one, duplicate, fact, fact_duplicate):
    return {
        'preference_same_id': bool(one and duplicate and one['id'] == duplicate['id']),
        'preference_flag': bool(duplicate and (duplicate.get('metadata') or {}).get('deduplicated') is True),
        'fact_same_id': bool(fact and fact_duplicate and fact['id'] == fact_duplicate['id']),
        'fact_flag': bool(fact_duplicate and (fact_duplicate.get('metadata') or {}).get('deduplicated') is True),
    }


_MISSING = object()


async def capture_gm05(adapter):
    """Ejecuta el escenario fijo y devuelve la captura canónica con sus brechas."""
    gaps = []
    source = await _attempt(adapter, 'add_entity', None, gaps,
                            name=SOURCE_ENTITY[0], type=SOURCE_ENTITY[1])
    target = await _attempt(adapter, 'add_entity', None, gaps,
                            name=TARGET_ENTITY[0], type=TARGET_ENTITY[1])
    applies_to = [source['id']] if source else None

    one = await _attempt(adapter, 'add_preference', None, gaps, **PREFERENCE_ONE,
                         user_identifier=USER, applies_to=applies_to)
    duplicate = await _attempt(adapter, 'add_preference', None, gaps, **PREFERENCE_ONE,
                               user_identifier=USER, applies_to=applies_to)
    two = await _attempt(adapter, 'add_preference', None, gaps, **PREFERENCE_TWO,
                         user_identifier=USER)
    preferences = [item for item in (one, two) if item]

    fact = await _attempt(adapter, 'add_fact', None, gaps, **FACT_ONE)
    fact_duplicate = await _attempt(adapter, 'add_fact', None, gaps, **FACT_DUPLICATE)
    fact_two = await _attempt(adapter, 'add_fact', None, gaps, **FACT_TWO)
    fact_temporal = await _attempt(adapter, 'add_fact', None, gaps, **FACT_TEMPORAL)
    facts = [item for item in (fact, fact_two, fact_temporal) if item]

    facts_about = await _attempt(adapter, 'get_facts_about', None, gaps,
                                 subject=FACT_ONE['subject'], limit=10)

    relationship = None
    related = None
    if source and target:
        relationship = await _attempt(adapter, 'add_relationship', None, gaps,
                                      source_id=source['id'], target_id=target['id'],
                                      **RELATIONSHIP)
        related = await _attempt(adapter, 'get_related_entities', None, gaps,
                                 entity_id=source['id'])

    supersession = None
    if one and two:
        first = await _attempt(adapter, 'supersede_preference', _MISSING, gaps,
                               old_id=one['id'], new_id=two['id'])
        before = await _attempt(adapter, 'get_preferences_for', None, gaps,
                                user_identifier=USER, active_only=False)
        second = await _attempt(adapter, 'supersede_preference', _MISSING, gaps,
                                old_id=one['id'], new_id=two['id'])
        after = await _attempt(adapter, 'get_preferences_for', None, gaps,
                               user_identifier=USER, active_only=False)
        active = await _attempt(adapter, 'get_preferences_for', None, gaps,
                                user_identifier=USER, active_only=True)
        graph = await _snapshot(adapter, gaps)
        if (first is not _MISSING and second is not _MISSING and before is not None
                and after is not None and active is not None and graph is not None):
            superseded = next((node for node in graph['nodes']
                               if node['key'] == one['id']
                               and 'Preference' in node['labels']), None)
            supersession = {
                'edge': any(edge['type'] == 'SUPERSEDED_BY' for edge in graph['edges']),
                # La respuesta de preferencias del oráculo no expone `valid_until`
                # (gm05_contract: campo no comparable), así que el hecho se mide donde
                # ambos lados lo persisten: el nodo del grafo.
                'old_valid_until_set': bool(superseded
                                            and superseded['properties'].get('valid_until')),
                'repeat_is_noop': _validity_view(before) == _validity_view(after),
                'active_only': [item['id'] for item in active],
                'history': [item['id'] for item in after],
            }
    else:
        graph = await _snapshot(adapter, gaps)

    gaps.extend(getattr(adapter, 'gaps', ()))
    return {
        'preferences': deepcopy(preferences),
        'preferences_for': deepcopy(after if one and two and after is not None else []),
        'facts': deepcopy(facts),
        'facts_about': deepcopy(facts_about or []),
        'relationships': [deepcopy(relationship)] if relationship else [],
        'related': deepcopy(related or []),
        'deduplication': _deduplication_flags(one, duplicate, fact, fact_duplicate),
        'supersession': supersession,
        'graph': deepcopy(graph or {'nodes': [], 'edges': []}),
        'gaps': sorted(set(gaps)),
    }
