"""Explicit offline numerical policy evaluation, never C1 acceptance.

Policy JSON has exactly: version, max_abs_error, min_embedding_cosine,
max_score_error, tie_epsilon, max_norm_error. All limits are explicit finite
numbers (not booleans); upper bounds are inclusive and cosine is an inclusive
lower bound. Version is minilm-cpu-f32-v1. Proposed review values are 1e-6,
0.99999999999, 1e-6, 2e-6, 1e-6 respectively; there are no implicit defaults.

compare_minilm_c1.compare validates inputs and supplies baseline measurements
under this explicit identity contract. Additional checks cover every coordinate,
within-batch unordered score including self, labeled score, L2 norm, and exact
text repetition within each runtime. Invariance scores compare the two repeated
vectors against every vector in that runtime. Ordering compares candidate pairs
for each within-batch query and each labeled triplet. A tie in EITHER runtime
excludes that comparison from the no-tie denominator, but never from numerical
checks. All union ties are emitted for review. Label quality is descriptive and
separate for Python and Rust; wrong labels alone do not fail numerical parity.

No inference, cache access, network, Neo4j, TopK, retrieval threshold, external
freeze verification, or statistical guarantee is provided. Scope is D1 encoder
records only. The caller must obtain external policy review and freeze.
"""
import argparse
import itertools
import json
import math

import compare_minilm_c1 as comparator

VERSION = 'minilm-cpu-f32-v1'
POLICY_KEYS = ('version', 'max_abs_error', 'min_embedding_cosine',
               'max_score_error', 'tie_epsilon', 'max_norm_error')


def validate_policy(policy):
    comparator.exact(policy, POLICY_KEYS)
    comparator.require(policy['version'] == VERSION, 'policy_version')
    for key in POLICY_KEYS[1:]:
        value = policy[key]
        comparator.require(type(value) in (int, float), 'policy_number')
        comparator.require(math.isfinite(value) and value >= 0, 'policy_number')
    comparator.require(policy['min_embedding_cosine'] <= 1, 'policy_cosine')


def dot(a, b):
    return math.fsum(x * y for x, y in zip(a, b))


def cosine(a, b):
    # Use the comparator's exact normalization and cosine scoring arithmetic.
    return comparator.dot(comparator.unit(a)[0], comparator.unit(b)[0])


def score(a, b):
    return cosine(a, b)


def evaluate(raw, rust, python, labels, policy):
    """Validate explicit records and policy; invalid contracts raise ValidationError."""
    validate_policy(policy)
    measurement = comparator.compare(raw, rust, python, labels, policy['version'])
    report = {'state': 'numeric_fail', 'policy': dict(policy),
              'policy_status': 'proposed_pending_external_freeze',
              'scope': 'D1_encoder_records_only', 'measurement': measurement,
              'violations': [], 'review_cases': []}
    if measurement['state'] != 'measured_not_accepted':
        report['violations'].append({'kind': 'comparison_validation', 'state': measurement['state']})
        return report
    violations = report['violations']

    def upper(kind, value, limit, **context):
        if value > policy[limit]:
            violations.append({'kind': kind, 'value': value, 'limit': limit, **context})

    def vector_checks(a, b, **context):
        before = len(violations)
        for index, (x, y) in enumerate(zip(a, b)):
            upper('coordinate', abs(x - y), 'max_abs_error', coordinate=index, **context)
        similarity = cosine(a, b)
        if similarity < policy['min_embedding_cosine']:
            violations.append({'kind': 'embedding_cosine', 'value': similarity,
                               'limit': 'min_embedding_cosine', **context})
        return len(violations) != before

    def score_check(a, b, **context):
        upper('score', abs(a - b), 'max_score_error', **context)

    def summary():
        return {'total': 0, 'union_ties': 0, 'without_ties': 0,
                'order_divergences_without_ties': 0, 'order_divergences_total': 0,
                'agreement_rate_total': None, 'agreement_rate_without_ties': None}

    def order(r_margin, p_margin, counts, **context):
        counts['total'] += 1
        r_tie = abs(r_margin) <= policy['tie_epsilon']
        p_tie = abs(p_margin) <= policy['tie_epsilon']
        r_order = (r_margin > 0) - (r_margin < 0)
        p_order = (p_margin > 0) - (p_margin < 0)
        differs = r_order != p_order
        counts['order_divergences_total'] += int(differs)
        if r_tie or p_tie:
            counts['union_ties'] += 1
            report['review_cases'].append({**context, 'rust_margin': r_margin,
                                           'python_margin': p_margin,
                                           'rust_tie': r_tie, 'python_tie': p_tie,
                                           'order_differs': differs})
        else:
            counts['without_ties'] += 1
            if differs:
                counts['order_divergences_without_ties'] += 1
                violations.append({'kind': 'order_outside_ties', **context})

    def finalize(counts):
        for denominator, divergence, name in (
                ('total', 'order_divergences_total', 'agreement_rate_total'),
                ('without_ties', 'order_divergences_without_ties', 'agreement_rate_without_ties')):
            n = counts[denominator]
            counts[name] = 1 - counts[divergence] / n if n else None

    refs = {}
    report['within_batch_order'] = summary()
    for rb, pb in zip(rust['batches'], python['batches']):
        rv, pv = rb['vectors'], pb['vectors']
        for i, (r, p) in enumerate(zip(rv, pv)):
            ref = f"{rb['id']}/{i}"
            refs[ref] = (r, p)
            vector_checks(r, p, scope='cross_runtime', ref=ref)
            for runtime, vector in (('rust', r), ('python', p)):
                upper('l2_norm', abs(math.hypot(*vector) - 1), 'max_norm_error',
                      runtime=runtime, ref=ref)
            for j in range(i, len(rv)):
                score_check(score(r, rv[j]), score(p, pv[j]), scope='within_batch',
                            refs=[ref, f"{rb['id']}/{j}"])
            for j, k in itertools.combinations(range(len(rv)), 2):
                order(score(r, rv[j]) - score(r, rv[k]), score(p, pv[j]) - score(p, pv[k]),
                      report['within_batch_order'], scope='within_batch', query=ref,
                      candidates=[f"{rb['id']}/{j}", f"{rb['id']}/{k}"])
    finalize(report['within_batch_order'])
    report['labels'] = summary()
    report['quality'] = {runtime: {'total': len(labels['triplets']), 'correct': 0,
                                   'incorrect': 0, 'ties': 0}
                         for runtime in ('rust', 'python')}
    for triplet in labels['triplets']:
        scores = []
        for index, runtime in enumerate(('rust', 'python')):
            q, p, n = (refs[triplet[key]][index] for key in ('query', 'positive', 'negative'))
            positive, negative = score(q, p), score(q, n)
            scores.append((positive, negative))
            margin = positive - negative
            outcome = ('ties' if abs(margin) <= policy['tie_epsilon'] else
                       'correct' if margin > 0 else 'incorrect')
            report['quality'][runtime][outcome] += 1
        for i, name in enumerate(('positive', 'negative')):
            score_check(scores[0][i], scores[1][i], scope='label', id=triplet['id'], role=name)
        order(scores[0][0] - scores[0][1], scores[1][0] - scores[1][1],
              report['labels'], scope='label', id=triplet['id'])
    finalize(report['labels'])
    for quality in report['quality'].values():
        quality['without_ties'] = quality['total'] - quality['ties']
        quality['accuracy_total'] = quality['correct'] / quality['total'] if quality['total'] else None
        quality['accuracy_without_ties'] = (quality['correct'] / quality['without_ties']
                                            if quality['without_ties'] else None)

    corpus = comparator.decode(raw)
    occurrences = {}
    for batch in corpus['batches']:
        for i, text in enumerate(batch['texts']):
            occurrences.setdefault(text, []).append(f"{batch['id']}/{i}")
    report['invariance'] = {}
    for index, runtime in enumerate(('rust', 'python')):
        counts = {'pairs': 0, 'failed_pairs': 0}
        for group in occurrences.values():
            for first, second in itertools.combinations(group, 2):
                counts['pairs'] += 1
                before = len(violations)
                a, b = refs[first][index], refs[second][index]
                context = {'scope': 'exact_text_invariance', 'runtime': runtime, 'refs': [first, second]}
                vector_checks(a, b, **context)
                # All runtime occurrences are score contexts, not retrieval evidence.
                for target_ref, vectors in refs.items():
                    target = vectors[index]
                    score_check(score(a, target), score(b, target), target=target_ref, **context)
                counts['failed_pairs'] += int(len(violations) != before)
        report['invariance'][runtime] = counts
    report['state'] = 'numeric_fail' if violations else 'numeric_pass_not_acceptance'
    return report


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    for name in ('corpus', 'rust', 'python', 'labels', 'policy'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args(argv)
    try:
        with open(args.corpus, 'rb') as stream:
            raw = stream.read(256 * 1024 + 1)
        inputs = []
        for path in (args.rust, args.python, args.labels, args.policy):
            with open(path, 'rb') as stream:
                inputs.append(comparator.read_json(stream))
        report = evaluate(raw, *inputs)
        print(json.dumps(report, allow_nan=False, sort_keys=True))
        return int(report['state'] == 'numeric_fail')
    except (OSError, comparator.ValidationError, OverflowError, RecursionError):
        print(json.dumps({'state': 'numeric_fail', 'error': 'policy_input_or_arithmetic'}))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
