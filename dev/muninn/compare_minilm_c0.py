"""Offline C0 measurement, not numerical acceptance or inference evidence.

Scores cover unordered within-batch pairs (including diagonals) and labeled
query/positive and query/negative pairs only, NOT all global corpus pairs.
At most 32 batches of 32 rows, 384 coordinates, and 1024 triplets are read.
No thresholds, embedding-space identity, or runtime measurements are inferred.
"""
import argparse
import hashlib
import json
import math
import re
import sys

EXPORT_LIMIT = 32 * 1024 * 1024
BUNDLE_ID = 'ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1'
PACKAGES = ('torch', 'sentence-transformers', 'transformers', 'tokenizers', 'numpy', 'safetensors')
TOKEN_KEYS = ('input_ids', 'attention_mask', 'token_type_ids')


class ValidationError(ValueError):
    """Input does not meet the private export contract."""


def require(condition, category):
    if not condition:
        raise ValidationError(category)


def exact(obj, keys):
    require(type(obj) is dict and set(obj) == set(keys), 'schema')


def identifier(value):
    return type(value) is str and re.fullmatch(r'[A-Za-z0-9_-]{1,64}', value) is not None


def _object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate_key')
        result[key] = value
    return result


def _float(value):
    result = float(value)
    require(math.isfinite(result), 'nonfinite')
    return result


def _constant(value):
    raise ValidationError('nonfinite')


def decode(raw):
    try:
        return json.loads(raw.decode('utf-8'), object_pairs_hook=_object,
                          parse_float=_float, parse_constant=_constant)
    except (ValueError, UnicodeError, RecursionError, OverflowError):
        raise ValidationError('json') from None


def read_json(reader):
    raw = reader.read(EXPORT_LIMIT + 1)
    require(len(raw) <= EXPORT_LIMIT, 'size')
    return decode(raw)


def validate_corpus(raw):
    require(len(raw) <= 256 * 1024, 'corpus_size')
    corpus = decode(raw)
    exact(corpus, ('corpus_id', 'batches'))
    require(identifier(corpus['corpus_id']) and corpus['corpus_id'].startswith('c0'), 'corpus_id')
    require(type(corpus['batches']) is list and 1 <= len(corpus['batches']) <= 32, 'batches')
    ids = set()
    for batch in corpus['batches']:
        exact(batch, ('id', 'texts'))
        require(identifier(batch['id']) and batch['id'] not in ids, 'batch_id')
        ids.add(batch['id'])
        require(type(batch['texts']) is list and 1 <= len(batch['texts']) <= 32, 'texts')
        for text in batch['texts']:
            require(type(text) is str, 'text')
            try:
                require(len(text.encode('utf-8')) <= 4096, 'text_size')
            except UnicodeError:
                raise ValidationError('text_encoding') from None
    return corpus


def runtime(receipt):
    exact(receipt, ('packages', 'python', 'platform', 'device', 'dtype', 'attention_backend',
                    'model_eval', 'max_seq_length'))
    exact(receipt['packages'], PACKAGES)
    require(all(type(v) is str and v for v in receipt['packages'].values()), 'packages')
    require(all(type(receipt[k]) is str and receipt[k] for k in
                ('python', 'platform', 'attention_backend')), 'runtime')
    require(receipt['device'] == 'cpu' and receipt['dtype'] == 'float32'
            and receipt['model_eval'] is True and type(receipt['max_seq_length']) is int
            and receipt['max_seq_length'] == 256, 'runtime')


def number(value):
    try:
        return type(value) in (int, float) and math.isfinite(value)
    except OverflowError:
        return False


def validate_export(obj, corpus, digest, reference):
    keys = ['schema_version', 'corpus_id', 'input_sha256', 'artifact_bundle_id',
            'execution', 'token_source', 'batches']
    if reference:
        keys.append('actual_runtime')
    exact(obj, keys)
    require(type(obj['schema_version']) is int and obj['schema_version'] == 1, 'version')
    require(obj['corpus_id'] == corpus['corpus_id'] and obj['input_sha256'] == digest
            and obj['artifact_bundle_id'] == BUNDLE_ID, 'identity')
    require(obj['execution'] == ('reference_cpu_f32' if reference else 'candidate_cpu_f32'), 'execution')
    require(obj['token_source'] == ('independent_recomputed_same_verified_cache' if reference
                                   else 'independently_recomputed_same_verified_cache'), 'token_source')
    if reference:
        runtime(obj['actual_runtime'])
    require(type(obj['batches']) is list and len(obj['batches']) == len(corpus['batches']), 'batches')
    for batch, source in zip(obj['batches'], corpus['batches']):
        exact(batch, ('id', *TOKEN_KEYS, 'vectors'))
        require(batch['id'] == source['id'], 'batch_order')
        count = len(source['texts'])
        rows = batch['input_ids']
        require(type(rows) is list and len(rows) == count and type(rows[0]) is list, 'tokens')
        width = len(rows[0])
        require(1 <= width <= 256, 'width')
        for key in TOKEN_KEYS:
            matrix = batch[key]
            require(type(matrix) is list and len(matrix) == count, 'token_rows')
            for row in matrix:
                require(type(row) is list and len(row) == width, 'token_shape')
                require(all(type(v) is int and 0 <= v <= 0xffffffff
                            and (key != 'attention_mask' or v in (0, 1))
                            and (key != 'token_type_ids' or v == 0) for v in row), 'token_value')
        require(type(batch['vectors']) is list and len(batch['vectors']) == count, 'vector_rows')
        for row in batch['vectors']:
            require(type(row) is list and len(row) == 384 and all(number(v) for v in row), 'vector')


def validate_labels(labels, corpus):
    exact(labels, ('schema_version', 'corpus_id', 'status', 'label_origin_es', 'reference_format',
                   'coverage', 'triplets', 'length_hypotheses', 'length_note_es', 'equal_text_groups'))
    require(type(labels['schema_version']) is int and labels['schema_version'] == 1
            and labels['corpus_id'] == corpus['corpus_id']
            and labels['reference_format'] == 'batch_id/zero_based_index'
            and labels['status'] == 'human_hypotheses_not_results', 'labels_identity')
    for key in ('label_origin_es', 'length_note_es'):
        require(type(labels[key]) is str, 'label_text')
    refs = {f"{b['id']}/{i}" for b in corpus['batches'] for i in range(len(b['texts']))}
    def ref(value):
        require(type(value) is str and value in refs, 'label_reference')
    require(type(labels['coverage']) is dict, 'coverage')
    for group in labels['coverage'].values():
        require(type(group) is list and len(group) <= 1024, 'coverage')
        for value in group:
            ref(value)
    require(type(labels['triplets']) is list and 1 <= len(labels['triplets']) <= 1024, 'triplets')
    ids = set()
    for triplet in labels['triplets']:
        exact(triplet, ('id', 'query', 'positive', 'negative', 'rationale_es'))
        require(identifier(triplet['id']) and triplet['id'] not in ids
                and type(triplet['rationale_es']) is str, 'triplet')
        ids.add(triplet['id'])
        for key in ('query', 'positive', 'negative'):
            ref(triplet[key])
    require(type(labels['length_hypotheses']) is list and len(labels['length_hypotheses']) <= 1024, 'lengths')
    for item in labels['length_hypotheses']:
        exact(item, ('ref', 'hello_repetitions', 'expected_encoded_length_before_truncation', 'tokenizer_verified'))
        ref(item['ref'])
        require(all(type(item[k]) is int and item[k] >= 0 for k in
                    ('hello_repetitions', 'expected_encoded_length_before_truncation'))
                and type(item['tokenizer_verified']) is bool, 'length')
    require(type(labels['equal_text_groups']) is list and len(labels['equal_text_groups']) <= 1024, 'groups')
    for group in labels['equal_text_groups']:
        require(type(group) is list and 2 <= len(group) <= 1024, 'group')
        for value in group:
            ref(value)


def unit(vector):
    scale = max(abs(v) for v in vector)
    require(scale > 0, 'zero_vector')
    scaled = [v / scale for v in vector]
    length = math.sqrt(math.fsum(v * v for v in scaled))
    norm = scale * length
    require(math.isfinite(norm), 'norm_overflow')
    return [v / length for v in scaled], norm


def dot(a, b):
    return max(-1.0, min(1.0, math.fsum(x * y for x, y in zip(a, b))))


def cosine(a, b):
    return dot(unit(a)[0], unit(b)[0])


def measure(rust, python, labels):
    per_vector, per_batch, logical = [], [], {}
    for rb, pb in zip(rust['batches'], python['batches']):
        errors, cosines, ru, pu = [], [], [], []
        for i, (r, p) in enumerate(zip(rb['vectors'], pb['vectors'])):
            r_unit, r_norm = unit(r)
            p_unit, p_norm = unit(p)
            error = max(abs(float(x) - float(y)) for x, y in zip(r, p))
            require(math.isfinite(error), 'error_overflow')
            similarity = dot(r_unit, p_unit)
            ref = f"{rb['id']}/{i}"
            logical[ref] = (r_unit, p_unit)
            ru.append(r_unit); pu.append(p_unit)
            errors.append(error); cosines.append(similarity)
            per_vector.append({'ref': ref, 'cosine': similarity, 'rust_norm': r_norm,
                               'python_norm': p_norm, 'max_abs_error': error})
        score_error = max(abs(dot(ru[i], ru[j]) - dot(pu[i], pu[j]))
                          for i in range(len(ru)) for j in range(i, len(ru)))
        per_batch.append({'id': rb['id'], 'max_abs_error': max(errors), 'min_cosine': min(cosines),
                          'max_within_batch_score_error': score_error})
    labeled = []
    for t in labels['triplets']:
        scores = {}
        for index, name in enumerate(('rust', 'python')):
            q, p, n = (logical[t[k]][index] for k in ('query', 'positive', 'negative'))
            positive, negative = dot(q, p), dot(q, n)
            scores[name] = {'positive': positive, 'negative': negative,
                            'gap': positive - negative, 'tie': positive == negative}
        labeled.append({'id': t['id'], **scores,
                        'positive_abs_error': abs(scores['rust']['positive'] - scores['python']['positive']),
                        'negative_abs_error': abs(scores['rust']['negative'] - scores['python']['negative'])})
    return {'metrics': {'max_abs_error': max(b['max_abs_error'] for b in per_batch),
                        'min_cosine': min(b['min_cosine'] for b in per_batch),
                        'max_within_batch_score_error': max(b['max_within_batch_score_error'] for b in per_batch)},
            'per_batch': per_batch, 'per_vector': per_vector, 'labeled_scores': labeled}


def compare(raw, rust, python, labels):
    corpus = validate_corpus(raw)
    digest = hashlib.sha256(raw).hexdigest()
    validate_export(rust, corpus, digest, False)
    validate_export(python, corpus, digest, True)
    validate_labels(labels, corpus)
    report = {'schema_version': 1, 'corpus_id': corpus['corpus_id'], 'input_sha256': digest,
              'artifact_bundle_id': BUNDLE_ID,
              'runtime': {'python': python['actual_runtime'], 'rust': {'state': 'unknown'}},
              'score_scope': 'within_batch_unordered_pairs_including_self_and_labeled_query_pairs_only'}
    mismatches = []
    for r, p in zip(rust['batches'], python['batches']):
        for key in TOKEN_KEYS:
            if r[key] != p[key]:
                mismatches.append({'batch_id': r['id'], 'field': key})
    if mismatches:
        return {**report, 'state': 'token_mismatch', 'token_mismatches': mismatches}
    return {**report, 'state': 'measured_not_accepted', **measure(rust, python, labels)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for name in ('corpus', 'rust', 'python', 'labels'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args(argv)
    try:
        with open(args.corpus, 'rb') as stream:
            raw = stream.read(256 * 1024 + 1)
        exports = []
        for path in (args.rust, args.python, args.labels):
            with open(path, 'rb') as stream:
                exports.append(read_json(stream))
        report = compare(raw, *exports)
        print(json.dumps(report, allow_nan=False, sort_keys=True))
        return 1 if report['state'] == 'token_mismatch' else 0
    except (OSError, ValidationError, OverflowError, RecursionError):
        print(json.dumps({'state': 'invalid_input', 'error': 'comparison_input_or_arithmetic'}))
        return 1


if __name__ == '__main__':
    sys.exit(main())
