"""Validación de preparación C1; no tokeniza, carga pesos ni acepta encoders."""
import hashlib
import io
import json
from pathlib import Path
import unicodedata

from export_minilm_c1 import read_corpus, RunError

ROOT = Path(__file__).resolve().parents[2]
CORPUS = Path('dev/muninn/fixtures/minilm-c1-v1-draft.json')
LABELS = Path('dev/muninn/fixtures/minilm-c1-v1-draft-labels.json')
C0 = Path('dev/muninn/fixtures/minilm-c0-v1.json')


def check_disjoint(corpus, baseline):
    current = {text for batch in corpus['batches'] for text in batch['texts']}
    prior = {text for batch in baseline['batches'] for text in batch['texts']}
    exact = current & prior
    nfc = {unicodedata.normalize('NFC', text) for text in current} & {
        unicodedata.normalize('NFC', text) for text in prior}
    if exact - {''} or nfc - {''}:
        raise ValueError('solapamiento no autorizado entre C0 y C1')
    return sorted(exact), sorted(nfc)


# Required declarations describe planned coverage, not verified semantic sufficiency.
COVERAGE = {'spanish', 'english', 'paraphrase_candidates', 'lexical_negatives',
            'unicode', 'empty', 'rust', 'cypher', 'technical_decisions',
            'boundary_candidates', 'prefix_tail', 'batch_reordering'}
SEMANTIC_BATCHES = {'semantic_es', 'semantic_en', 'lexical_es', 'rust', 'cypher', 'decision'}
BOUNDARY_TARGETS = [127, 128, 129, 255, 256, 257]


def require_fields(value, fields):
    if type(value) is not dict or set(value) != set(fields):
        raise ValueError('esquema sidecar inválido')


def validate_labels(labels, corpus, batches):
    require_fields(labels, {'schema_version', 'corpus_id', 'lifecycle', 'label_origin',
                            'token_lengths_status', 'boundary_targets', 'boundary_batch',
                            'triplets', 'coverage', 'empty_overlap_exception', 'limits'})
    if (type(labels['schema_version']) is not int or labels['schema_version'] != 1
            or type(labels['corpus_id']) is not str
            or labels['corpus_id'] != corpus['corpus_id']
            or labels['lifecycle'] != 'draft_not_frozen'
            or labels['label_origin'] != 'agent_generated_hypotheses_not_human_annotations'
            or labels['token_lengths_status'] != 'not_verified'
            or labels['empty_overlap_exception'] != 'user_authorized_2026-10-30_only_empty_string'):
        raise ValueError('etiquetas no son hipótesis de borrador')
    limits = labels['limits']
    if (type(limits) is not list or not limits
            or any(type(item) is not str or not item.strip() for item in limits)):
        raise ValueError('límites inválidos')
    coverage = labels['coverage']
    if (type(coverage) is not list or any(type(item) is not str for item in coverage)
            or len(coverage) != len(COVERAGE) or set(coverage) != COVERAGE):
        raise ValueError('declaraciones de cobertura inválidas')
    triplets = labels['triplets']
    if type(triplets) is not list or not triplets:
        raise ValueError('triplets vacíos o inválidos')
    seen = set()
    covered = set()
    for triplet in triplets:
        require_fields(triplet, {'batch', 'query', 'positive', 'negative'})
        batch = triplet['batch']
        if type(batch) is not str or batch not in SEMANTIC_BATCHES or batch not in batches:
            raise ValueError('batch de triplet inválido')
        indices = tuple(triplet[key] for key in ('query', 'positive', 'negative'))
        if (any(type(index) is not int or not 0 <= index < len(batches[batch])
                for index in indices) or len(set(indices)) != 3):
            raise ValueError('índice de triplet inválido')
        identity = (batch, *indices)
        if identity in seen:
            raise ValueError('triplet duplicado')
        seen.add(identity)
        covered.add(batch)
    if covered != SEMANTIC_BATCHES:
        raise ValueError('faltan batches semánticos declarados')
    batch = labels['boundary_batch']
    targets = labels['boundary_targets']
    if (type(batch) is not str or batch != 'boundary_candidates' or batch not in batches
            or type(targets) is not list or any(type(target) is not int for target in targets)
            or targets != BOUNDARY_TARGETS or len(batches[batch]) != len(targets)):
        raise ValueError('frontera inválida o presentada como medida')


def validate(corpus=None, labels=None):
    if corpus is not None:
        # Parse before inspecting fields so malformed supplied objects fail controllably.
        try:
            raw = json.dumps(corpus, ensure_ascii=False).encode()
        except (TypeError, ValueError, UnicodeError) as error:
            raise ValueError('corpus supplied inválido') from error
        try:
            corpus, _ = read_corpus(io.BytesIO(raw))
        except RunError:
            # Preserve the existing duplicate-ID error, but only inspect safe shapes.
            if type(corpus) is dict and type(corpus.get('batches')) is list:
                ids = [batch.get('id') for batch in corpus['batches']
                       if type(batch) is dict and type(batch.get('id')) is str]
                if len(ids) != len(set(ids)):
                    raise ValueError('batch duplicado') from None
            raise
    corpus_hash = None
    labels_hash = None
    if corpus is None:
        raw = (ROOT / CORPUS).read_bytes()
        corpus, corpus_hash = read_corpus(io.BytesIO(raw))
    if labels is None:
        raw = (ROOT / LABELS).read_bytes()
        labels = json.loads(raw)
        labels_hash = hashlib.sha256(raw).hexdigest()
    baseline = json.loads((ROOT / C0).read_text())
    exact, nfc = check_disjoint(corpus, baseline)
    batches = {}
    for batch in corpus['batches']:
        if batch['id'] in batches:
            raise ValueError('batch duplicado')
        batches[batch['id']] = batch['texts']
    validate_labels(labels, corpus, batches)
    return {'corpus_id': corpus['corpus_id'], 'batches': len(batches),
            'text_positions': sum(map(len, batches.values())),
            'triplets': len(labels['triplets']), 'shared_exact': exact, 'shared_nfc': nfc,
            'acceptance_ready': False,
            'corpus_sha256': corpus_hash, 'labels_sha256': labels_hash}


if __name__ == '__main__':
    print(json.dumps(validate(), ensure_ascii=False, sort_keys=True))
