"""Offline H5 replay diagnostics. No model execution or acceptance decision."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sys

from retrieval_quality import corpus_hash, validate_corpus

LIMIT = 8 * 1024 * 1024


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key')
        result[key] = value
    return result


def _constant(value):
    raise ValueError('nonfinite JSON')


def read_json(path):
    with open(path, 'rb') as stream:
        raw = stream.read(LIMIT + 1)
    if len(raw) > LIMIT:
        raise ValueError('input size limit')
    return json.loads(raw, object_pairs_hook=_pairs, parse_constant=_constant), raw


def encode(obj):
    return (json.dumps(obj, ensure_ascii=False, sort_keys=True,
                       separators=(',', ':'), allow_nan=False) + '\n').encode()


def derive(corpus, source_raw):
    validate_corpus(corpus)
    batches, items = [], []
    for query in corpus['queries']:
        for role, item in [('query', query)] + [('candidate', c) for c in query['candidates']]:
            text = item['text']
            if len(text.encode()) > 4096:
                raise ValueError('text exceeds exporter limit')
            batch_id = f'item_{len(items):03d}'
            batches.append({'id': batch_id, 'texts': [text]})
            items.append({'batch_id': batch_id, 'role': role, 'query_id': query['id'],
                          'item_id': item['id'], 'text_sha256': digest(text.encode())})
    if len(batches) > 32:
        raise ValueError('singleton count exceeds exporter limit')
    derived = {'corpus_id': 'c1_h5_' + digest(source_raw)[:24], 'batches': batches}
    raw = encode(derived)
    if len(raw) > 256 * 1024:
        raise ValueError('derived corpus exceeds exporter limit')
    return derived, {'schema_version': 1, 'source_sha256': digest(source_raw),
                     'corpus_hash': corpus_hash(corpus), 'derived_sha256': digest(raw),
                     'items': items}


def write_new(path, obj):
    with open(path, 'xb') as stream:
        stream.write(encode(obj))


def prepare(source, derived_path, manifest_path):
    paths = [Path(p).resolve() for p in (source, derived_path, manifest_path)]
    if len(set(paths)) != 3 or any(p.exists() for p in paths[1:]):
        raise FileExistsError('outputs must be distinct new files')
    corpus, raw = read_json(source)
    derived, manifest = derive(corpus, raw)
    write_new(derived_path, derived)
    write_new(manifest_path, manifest)


def _number(value):
    return type(value) in (int, float) and math.isfinite(value)


def _norm(vector):
    return math.hypot(*vector)


def _unit(vector):
    # Scale first so subnormal norm rounding cannot distort normalization.
    scale = max(abs(value) for value in vector)
    scaled = [value / scale for value in vector]
    length = math.sqrt(math.fsum(value * value for value in scaled))
    return [value / length for value in scaled]


def cosine(left, right):
    result = math.fsum(a * b for a, b in zip(_unit(left), _unit(right)))
    return max(-1.0, min(1.0, result))


def validate_export(obj, derived, manifest, runtime):
    from c1_identity import validate
    if type(obj) is not dict or type(obj.get('identity')) is not dict:
        raise ValueError('export identity')
    identity = obj['identity']
    contract = identity.get('compatibility', {}).get('numeric_contract_version')
    validate(identity, contract)
    if (type(obj.get('schema_version')) is not int or obj['schema_version'] != 2
            or obj.get('corpus_id') != derived['corpus_id']
            or obj.get('input_sha256') != manifest['derived_sha256']
            or obj.get('artifact_bundle_id') != identity['artifact_bundle_id']
            or obj.get('execution') != {'rust': 'candidate_cpu_f32',
                                        'python': 'reference_cpu_f32'}[runtime]):
        raise ValueError('export binding')
    batches = obj.get('batches')
    if type(batches) is not list or len(batches) != len(derived['batches']):
        raise ValueError('batch coverage')
    expected = {b['id'] for b in derived['batches']}
    vectors = {}
    for batch in batches:
        if (type(batch) is not dict or type(batch.get('id')) is not str
                or batch['id'] not in expected or batch['id'] in vectors):
            raise ValueError('batch ID')
        rows = batch.get('vectors')
        if (type(rows) is not list or len(rows) != 1 or type(rows[0]) is not list
                or len(rows[0]) != 384 or not all(_number(v) for v in rows[0])
                or not math.isfinite(_norm(rows[0])) or _norm(rows[0]) == 0):
            raise ValueError('vector shape or values')
        width = None
        for key in ('input_ids', 'attention_mask', 'token_type_ids'):
            matrix = batch.get(key)
            if (type(matrix) is not list or len(matrix) != 1 or type(matrix[0]) is not list
                    or not 1 <= len(matrix[0]) <= 256):
                raise ValueError('token shape')
            row = matrix[0]
            if width is not None and len(row) != width:
                raise ValueError('token width')
            width = len(row)
            if any(type(v) is not int or not 0 <= v <= 0xffffffff
                   or (key == 'attention_mask' and v not in (0, 1))
                   or (key == 'token_type_ids' and v != 0) for v in row):
                raise ValueError('token values')
        vectors[batch['id']] = rows[0]
    return vectors


def rank_queries(corpus, items, vectors):
    index = {(i['query_id'], i['role'], i['item_id']): vectors[i['batch_id']] for i in items}
    results = []
    for query in corpus['queries']:
        qv = index[(query['id'], 'query', query['id'])]
        scores = [{'id': c['id'], 'score': cosine(qv, index[(query['id'], 'candidate', c['id'])])}
                  for c in query['candidates']]
        # Stable original-corpus order breaks exact ties for display only.
        scores.sort(key=lambda s: -s['score'])
        groups = []
        for item in scores:
            if not groups or groups[-1][0]['score'] != item['score']:
                groups.append([])
            groups[-1].append(item)
        results.append({'query_id': query['id'], 'scores': scores,
                        'ordered_ids': [s['id'] for s in scores],
                        'adjacent_margins': [a['score'] - b['score'] for a, b in zip(scores, scores[1:])],
                        'exact_tie_groups': [[s['id'] for s in group] for group in groups if len(group) > 1]})
    return results


def historical_orders(history, corpus, ranks):
    provenance = history.get('provenance', {})
    if (provenance.get('corpus_id') != corpus['corpus_id']
            or provenance.get('corpus_hash') != corpus_hash(corpus)):
        raise ValueError('historical corpus binding')
    rows = history.get('per_query')
    if type(rows) is not list or len(rows) != len(corpus['queries']):
        raise ValueError('historical coverage')
    expected = {q['id']: {c['id'] for c in q['candidates']} for q in corpus['queries']}
    seen, output = set(), []
    for row in rows:
        qid = row.get('query_id')
        if type(qid) is not str or qid not in expected or qid in seen:
            raise ValueError('historical query ID')
        seen.add(qid)
        metrics = row.get('metrics', {})
        ids, scores = metrics.get('ordered_ids'), metrics.get('ordered_scores')
        if (type(ids) is not list or any(type(i) is not str for i in ids)
                or len(set(ids)) != len(ids) or set(ids) != expected[qid]
                or type(scores) is not list or len(scores) != len(ids)
                or not all(_number(s) for s in scores)):
            raise ValueError('historical candidate coverage or scores')
        output.append({'query_id': qid, 'ordered_ids': ids, 'score_observations': scores,
                       **{f'matches_{runtime}_replay': ids == next(
                           r['ordered_ids'] for r in replay_rows if r['query_id'] == qid)
                          for runtime, replay_rows in ranks.items()}})
    return output


def compare(source, derived_path, manifest_path, rust_path, python_path, historical=None):
    paths = {'source': source, 'derived': derived_path, 'manifest': manifest_path,
             'rust_export': rust_path, 'python_export': python_path}
    if historical is not None:
        paths['historical'] = historical
    loaded = {name: read_json(path) for name, path in paths.items()}
    corpus, source_raw = loaded['source']
    expected_derived, manifest = derive(corpus, source_raw)
    if (loaded['derived'][0] != expected_derived
            or digest(loaded['derived'][1]) != manifest['derived_sha256']
            or encode(loaded['manifest'][0]) != encode(manifest)):
        raise ValueError('derived text or manifest binding')
    exports = {r: loaded[r + '_export'][0] for r in ('rust', 'python')}
    vectors = {r: validate_export(obj, expected_derived, manifest, r) for r, obj in exports.items()}
    if encode(exports['rust']['identity']) != encode(exports['python']['identity']):
        raise ValueError('incompatible embedding identity')
    differences = []
    for item in manifest['items']:
        a, b = (vectors[r][item['batch_id']] for r in ('rust', 'python'))
        values = {'batch_id': item['batch_id'], 'max_abs_error': max(abs(x-y) for x, y in zip(a, b)),
                  'cosine': cosine(a, b), 'rust_norm': _norm(a), 'python_norm': _norm(b),
                  'norm_difference': abs(_norm(a)-_norm(b)),
                  'rust_unit_norm_error': abs(_norm(a)-1), 'python_unit_norm_error': abs(_norm(b)-1)}
        if not all(math.isfinite(v) for k, v in values.items() if k != 'batch_id'):
            raise ValueError('nonfinite diagnostic result')
        differences.append(values)
    ranks = {r: rank_queries(corpus, manifest['items'], v) for r, v in vectors.items()}
    return {'schema_version': 1, 'state': 'diagnostic_only_not_acceptance',
            'input_sha256': {name: digest(raw) for name, (_, raw) in loaded.items()},
            'runtime_evidence': {r: {'identity_status': 'supplied_declaration_not_execution_authentication',
                                    'receipt_status': 'supplied_not_independently_measured' if 'actual_runtime' in obj else 'not_supplied',
                                    'receipt_sha256': digest(encode(obj['actual_runtime'])) if 'actual_runtime' in obj else None}
                                 for r, obj in exports.items()},
            'measured_here': ['input_byte_hashes', 'vector_arithmetic', 'exhaustive_query_candidate_orders'],
            'embedding_differences': differences, 'raw_cosine': ranks,
            'tie_rule': 'exact_float_equality; corpus_order_display_only; no_tolerance',
            'score_comparison': 'blocked/unverified-scale',
            'historical_orders': historical_orders(loaded['historical'][0], corpus, ranks) if historical else [],
            'limitations': ['Replay is not historical service-vector capture.',
                            'Input hashes do not prove source-to-binary correspondence.',
                            'No Neo4j score scale or acceptance threshold is established.',
                            'Frozen D1 policy is not applied to this diagnostic.',
                            'Token and runtime metadata are supplied export claims, not independent capture.']}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    commands = parser.add_subparsers(dest='command', required=True)
    prep = commands.add_parser('prepare', allow_abbrev=False)
    prep.add_argument('--corpus', required=True)
    prep.add_argument('--derived', required=True)
    prep.add_argument('--manifest', required=True)
    comp = commands.add_parser('compare', allow_abbrev=False)
    for name in ('corpus', 'derived', 'manifest', 'rust', 'python', 'output'):
        comp.add_argument('--' + name, required=True)
    comp.add_argument('--historical')
    args = parser.parse_args(argv)
    try:
        if args.command == 'prepare':
            prepare(args.corpus, args.derived, args.manifest)
        else:
            if Path(args.output).exists():
                raise FileExistsError('output exists')
            report = compare(args.corpus, args.derived, args.manifest, args.rust,
                             args.python, args.historical)
            write_new(args.output, report)
        return 0
    except (ValueError, OSError, TypeError, KeyError, OverflowError, AttributeError):
        # Fixed message prevents untrusted payloads or filesystem paths leaking.
        sys.stderr.write('H5 diagnostics failed: incompatible input or unavailable new output\n')
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
