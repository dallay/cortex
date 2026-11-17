"""Offline gate for a tokenizer-only receipt; never encoder acceptance.

The gate validates the recorded observation of the pinned tokenizer against the
exact corpus and label bytes, the declared truncation contract and the
invariants that make the receipt interpretable: separator retention, padding
masks, declared lengths, batch coverage, boundary lengths and identical
tokenization of repeated texts across batches.

A passing gate states that the tokenizer observation is internally consistent
and bound to the corpus. It is not encoder acceptance, product quality, or
authentication of the runtime that produced the receipt.
"""
import argparse
import hashlib
import json
from pathlib import Path

CLS_ID = 101
SEP_ID = 102
PAD_ID = 0
MAX_LENGTH = 256
SCOPE = 'tokenizer_only_not_encoder_evidence'
STATE = 'tokenizer_gate_pass_not_encoder_acceptance'
MODEL_ID = 'sentence-transformers/all-MiniLM-L6-v2'
REVISION = '1110a243fdf4706b3f48f1d95db1a4f5529b4d41'


class GateError(ValueError):
    """Receipt does not satisfy the tokenizer observation contract."""


def require(condition, category='receipt'):
    if not condition:
        raise GateError(category)


def sha256_bytes(raw):
    return hashlib.sha256(raw).hexdigest()


def _rows(section, name):
    require(type(section) is dict, name)
    for key in ('input_ids', 'attention_mask', 'token_type_ids'):
        require(type(section.get(key)) is list, name)
        require(len(section[key]) == len(section['input_ids']), name)
    return section


def _check_full(batch):
    section = _rows(batch.get('full'), 'full')
    for ids, mask, types in zip(section['input_ids'], section['attention_mask'],
                                section['token_type_ids']):
        require(type(ids) is list and ids, 'full')
        require(ids[0] == CLS_ID and ids[-1] == SEP_ID, 'full')
        require(all(value == 1 for value in mask) and len(mask) == len(ids), 'full')
        require(all(value == 0 for value in types) and len(types) == len(ids), 'full')


def _expected_truncated(ids):
    if len(ids) <= MAX_LENGTH:
        return list(ids)
    return list(ids[:MAX_LENGTH - 1]) + [SEP_ID]


def _check_truncated(batch):
    section = _rows(batch.get('truncated'), 'truncated')
    for ids, mask, types in zip(section['input_ids'], section['attention_mask'],
                                section['token_type_ids']):
        require(type(ids) is list and ids, 'truncated')
        require(len(ids) <= MAX_LENGTH, 'truncated')
        require(all(value == 1 for value in mask) and len(mask) == len(ids), 'truncated')
        require(all(value == 0 for value in types) and len(types) == len(ids), 'truncated')


def _check_padded(batch, truncated_rows):
    section = _rows(batch.get('padded'), 'padded')
    widths = {len(ids) for ids in section['input_ids']}
    require(len(widths) == 1, 'padded')
    width = widths.pop()
    for ids, mask, types, truncated in zip(section['input_ids'], section['attention_mask'],
                                           section['token_type_ids'], truncated_rows):
        require(len(ids) == width, 'padded')
        require(len(mask) == width and len(types) == width, 'padded')
        require(all(value == 0 for value in types), 'padded')
        real = [index for index, value in enumerate(mask) if value == 1]
        require(real == list(range(len(real))), 'padded')
        require(len(real) == len(truncated), 'padded')
        require(list(ids[:len(truncated)]) == list(truncated), 'padded')
        require(all(ids[index] == PAD_ID for index in range(len(real), width)), 'padded')


def validate(receipt, corpus, labels, corpus_raw, labels_raw):
    """Validate a tokenizer receipt against the corpus and label contract.

    The raw bytes are required: the receipt binds file identity, and a parsed
    object cannot reproduce that identity.
    """
    require(type(receipt) is dict, 'receipt')
    require(receipt.get('schema_version') == 1, 'schema')
    require(receipt.get('scope') == SCOPE, 'scope')
    require(receipt.get('acceptance_ready') is False, 'acceptance')
    require(receipt.get('model_id') == MODEL_ID, 'model')
    require(receipt.get('revision') == REVISION, 'revision')
    require(receipt.get('max_length') == MAX_LENGTH, 'max_length')
    require(receipt.get('padding_side') == 'right', 'padding')
    require(receipt.get('truncation_side') == 'right', 'truncation')
    require(receipt.get('truncation_strategy') == 'longest_first', 'truncation')
    require(receipt.get('corpus_id') == corpus['corpus_id'], 'corpus_id')

    require(receipt.get('corpus_sha256') == sha256_bytes(corpus_raw), 'corpus_hash')
    require(receipt.get('labels_sha256') == sha256_bytes(labels_raw), 'labels_hash')

    require(type(receipt.get('batches')) is list and receipt['batches'], 'batches')
    receipt_batches = {batch['id']: batch for batch in receipt['batches']}
    require(len(receipt_batches) == len(receipt['batches']), 'batches')
    corpus_batches = {batch['id']: batch for batch in corpus['batches']}
    require(set(receipt_batches) == set(corpus_batches), 'batches')

    observed = {}
    for batch_id, batch in receipt_batches.items():
        texts = corpus_batches[batch_id]['texts']
        _check_full(batch)
        _check_truncated(batch)
        full = batch['full']['input_ids']
        truncated = batch['truncated']['input_ids']
        _check_padded(batch, truncated)
        require(len(full) == len(texts) == len(truncated), 'cardinality')
        require(batch['full_lengths_including_specials'] == [len(ids) for ids in full],
                'lengths')
        require(batch['truncated_lengths_including_specials'] ==
                [len(ids) for ids in truncated], 'lengths')
        require(batch['was_truncated'] == [len(a) > len(b) for a, b in
                                           zip(full, truncated)], 'truncation')
        for full_ids, truncated_ids in zip(full, truncated):
            require(truncated_ids == _expected_truncated(full_ids), 'truncation')
        for text, ids in zip(texts, full):
            observed.setdefault(text, set()).add(tuple(ids))
    for text, variants in observed.items():
        require(len(variants) == 1, 'invariance')

    boundary = receipt.get('boundary_batch')
    targets = receipt.get('boundary_targets_from_labels')
    require(type(targets) is list and targets == labels['boundary_targets'], 'boundary')
    require(boundary in receipt_batches, 'boundary')
    boundary_batch = receipt_batches[boundary]
    require(boundary_batch['full_lengths_including_specials'] == targets, 'boundary')
    require(boundary_batch['truncated_lengths_including_specials'] ==
            [min(target, MAX_LENGTH) for target in targets], 'boundary')

    return {'state': STATE, 'scope': SCOPE, 'acceptance_ready': False,
            'corpus_id': corpus['corpus_id'],
            'corpus_sha256': receipt.get('corpus_sha256'),
            'labels_sha256': receipt.get('labels_sha256'),
            'batches': len(receipt_batches),
            'boundary_observed_lengths':
                boundary_batch['full_lengths_including_specials'],
            'model_id': MODEL_ID, 'revision': REVISION}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--corpus', required=True)
    parser.add_argument('--labels', required=True)
    parser.add_argument('--receipt', required=True)
    args = parser.parse_args(argv)
    corpus_raw = Path(args.corpus).read_bytes()
    labels_raw = Path(args.labels).read_bytes()
    corpus = json.loads(corpus_raw)
    labels = json.loads(labels_raw)
    receipt = json.loads(Path(args.receipt).read_bytes())
    try:
        report = validate(receipt, corpus, labels, corpus_raw, labels_raw)
    except GateError as error:
        print(json.dumps({'state': 'tokenizer_gate_fail', 'category': str(error)},
                         sort_keys=True))
        return 1
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
