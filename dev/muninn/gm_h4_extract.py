"""GM-03 spaCy stage capture: one server side, frozen texts, no model runtime.

The pinned stage is `neo4j_agent_memory.extraction.spacy_extractor` and the
Muninn stage is `dev/muninn/spacy_service.extract`. Both run in the same oracle
image against the same installed `en_core_web_sm-3.8.0` model, in separate
processes; this module only projects their payloads and never loads a model,
opens a socket or normalizes a span.

Projection, declared before measurement:

* compared — `name`, `type`, `subtype`, `start_pos`, `end_pos`, `confidence`,
  `extractor`;
* reference-only — `context`, `attributes` and `id`, which the pinned
  `ExtractedEntity` exposes and the Muninn stage does not emit. They are
  accepted and dropped, never compared.

An unknown or absent field on either side fails; there is no defaulting.
"""
from copy import deepcopy

from gm_h4_contract import ContractError, GM03_EXCLUDED, GM03_FIELDS, SIDES

REFERENCE_FIELDS = frozenset(GM03_FIELDS + GM03_EXCLUDED)
PRODUCT_FIELDS = frozenset(GM03_FIELDS)


def validate_side(value):
    if value not in SIDES:
        raise ValueError('unknown capture side')
    return value


def _project(value, allowed, path):
    from gm_h4_contract import fields
    if not isinstance(value, dict):
        raise ContractError(path + ': object required')
    fields(value, GM03_FIELDS, allowed, path)
    return {name: deepcopy(value[name]) for name in GM03_FIELDS}


def project_reference(value, path='entity'):
    """Project one pinned `ExtractedEntity` dump onto the compared fields."""
    return _project(value, REFERENCE_FIELDS, path)


def project_product(value, path='entity'):
    """Project one Muninn sidecar extraction item onto the compared fields."""
    return _project(value, PRODUCT_FIELDS, path)


PROJECTORS = {'reference': project_reference, 'product': project_product}


def capture_gm03(texts, extract, side):
    """Capture the stage entity stream for `side` over the frozen texts.

    `texts` are the frozen `{'key', 'text'}` inputs and `extract(text)` returns
    the side's own payload list. Document order is preserved exactly as the
    stage emitted it.
    """
    project = PROJECTORS[validate_side(side)]
    frozen = []
    for index, item in enumerate(texts if isinstance(texts, list) else []):
        if not isinstance(item, dict) or set(item) != {'key', 'text'}:
            raise ContractError(f'texts[{index}]: exactly key and text are required')
        if not isinstance(item['key'], str) or not item['key']:
            raise ContractError(f'texts[{index}].key: non-empty string required')
        if not isinstance(item['text'], str):
            raise ContractError(f'texts[{index}].text: string required')
        frozen.append({'key': item['key'], 'text': item['text']})
    if len({item['key'] for item in frozen}) != len(frozen):
        raise ContractError('texts: duplicate frozen text key')
    entities = []
    for item in frozen:
        produced = extract(item['text'])
        if not isinstance(produced, list):
            raise ContractError(f'gm03.{item["key"]}: list required from the stage')
        for value in produced:
            entities.append({'text': item['key'], **project(value, f'gm03.{item["key"]}')})
    return {'texts': frozen, 'entities': entities}
