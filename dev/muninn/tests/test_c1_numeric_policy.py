"""Synthetic numerical records only: no inference or statistical guarantee."""
import copy
import hashlib
import io
import json
import math
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

import c1_numeric_policy as policy
from c1_identity import declare
from test_compare_minilm_c1 import records

VERSION = 'minilm-cpu-f32-v1'


def config():
    return {'version': VERSION, 'max_abs_error': 1e-6,
            'min_embedding_cosine': 0.99999999999, 'max_score_error': 1e-6,
            'tie_epsilon': 2e-6, 'max_norm_error': 1e-6}


def fixture():
    args = list(records())
    for runtime in args[1:3]:
        runtime['identity'] = declare(VERSION)
    return args


def replace_text(args, index, text):
    corpus = json.loads(args[0])
    corpus['batches'][0]['texts'][index] = text
    args[0] = json.dumps(corpus).encode()
    for runtime in args[1:3]:
        runtime['input_sha256'] = hashlib.sha256(args[0]).hexdigest()


class PolicyTests(unittest.TestCase):
    def test_explicit_contract_and_pass_is_not_acceptance(self):
        args = fixture()
        with patch.object(policy.comparator, 'compare', wraps=policy.comparator.compare) as compare:
            result = policy.evaluate(*args, config())
        self.assertEqual(compare.call_args.args[-1], VERSION)
        self.assertEqual(result['state'], 'numeric_pass_not_acceptance')
        self.assertEqual(result['policy_status'], 'proposed_pending_external_freeze')
        self.assertEqual(result['labels']['total'], 1)
        self.assertEqual(result['labels']['without_ties'], 1)
        self.assertEqual(result['quality']['python']['correct'], 1)
        self.assertEqual(result['quality']['rust']['correct'], 1)
        self.assertIn('label_provenance', result['measurement'])

    def test_exact_finite_policy_no_bool_or_implicit_default(self):
        bads = [None, {}, {**config(), 'extra': 0}, {**config(), 'version': 'other'}]
        for key in config():
            if key != 'version':
                for value in (True, '0.1', None, math.nan, math.inf, -1):
                    bads.append({**config(), key: value})
        bads.append({**config(), 'min_embedding_cosine': 1.1})
        for bad in bads:
            with self.subTest(bad=bad), self.assertRaises(policy.comparator.ValidationError):
                policy.evaluate(*fixture(), bad)
        with self.assertRaises(TypeError):
            policy.evaluate(*fixture())

    def test_inclusive_limits_and_next_float_failure(self):
        # Derive exact representable boundaries from actual measured differences.
        args = fixture()
        args[1]['batches'][0]['vectors'][2][0] = 1e-6
        report = policy.evaluate(*args, config())
        self.assertEqual(report['state'], 'numeric_pass_not_acceptance')
        metrics = report['measurement']['metrics']
        limits = config()
        limits.update(max_abs_error=metrics['max_abs_error'],
                      min_embedding_cosine=metrics['min_cosine'],
                      max_score_error=max(metrics['max_within_batch_score_error'],
                                          max(max(t['positive_abs_error'], t['negative_abs_error']) for t in report['measurement']['labeled_scores'])))
        self.assertEqual(policy.evaluate(*args, limits)['state'], 'numeric_pass_not_acceptance')
        for key, direction in [('max_abs_error', -math.inf),
                               ('max_score_error', -math.inf),
                               ('min_embedding_cosine', math.inf)]:
            bad = {**limits, key: math.nextafter(limits[key], direction)}
            self.assertEqual(policy.evaluate(*args, bad)['state'], 'numeric_fail', key)

    def test_norm_boundary_and_failure_even_identical_runtimes(self):
        args = fixture()
        for runtime in args[1:3]:
            runtime['batches'][0]['vectors'][2][1] = 1.0 + 1e-6
        error = abs(math.hypot(*args[1]['batches'][0]['vectors'][2]) - 1)
        limits = {**config(), 'max_norm_error': error}
        self.assertEqual(policy.evaluate(*args, limits)['state'], 'numeric_pass_not_acceptance')
        limits['max_norm_error'] = math.nextafter(error, 0)
        result = policy.evaluate(*args, limits)
        self.assertEqual(result['state'], 'numeric_fail')
        self.assertEqual({v['runtime'] for v in result['violations'] if v['kind'] == 'l2_norm'}, {'rust', 'python'})

    def test_union_ties_denominators_and_review_does_not_hide_errors(self):
        args = fixture()
        # Python has a tie, Rust has reverse order outside its own tie band.
        args[2]['batches'][0]['vectors'][2] = args[2]['batches'][0]['vectors'][1][:]
        args[1]['batches'][0]['vectors'][2] = args[1]['batches'][0]['vectors'][0][:]
        args[1]['batches'][0]['vectors'][1] = [0., 1.] + [0.] * 382
        result = policy.evaluate(*args, config())
        self.assertEqual(result['state'], 'numeric_fail')
        self.assertEqual(result['labels']['total'], 1)
        self.assertEqual(result['labels']['union_ties'], 1)
        self.assertEqual(result['labels']['without_ties'], 0)
        self.assertIsNone(result['labels']['agreement_rate_without_ties'])
        self.assertEqual(result['quality']['python']['ties'], 1)
        self.assertEqual(result['quality']['rust']['incorrect'], 1)
        self.assertTrue(any(c['scope'] == 'label' and c['id'] == 't' for c in result['review_cases']))
        self.assertTrue(any(v['kind'] == 'score' for v in result['violations']))

    def test_tie_epsilon_inclusive_in_either_runtime(self):
        args = fixture()
        result = policy.evaluate(*args, {**config(), 'tie_epsilon': 1.0})
        self.assertEqual(result['labels']['union_ties'], 1)
        self.assertEqual(result['labels']['without_ties'], 0)
        result = policy.evaluate(*args, {**config(), 'tie_epsilon': math.nextafter(1., 0.)})
        self.assertEqual(result['labels']['union_ties'], 0)

    def test_order_discrepancy_and_quality_are_separate(self):
        args = fixture()
        vectors = args[1]['batches'][0]['vectors']
        vectors[1], vectors[2] = vectors[2], vectors[1]
        result = policy.evaluate(*args, config())
        self.assertEqual(result['labels']['order_divergences_without_ties'], 1)
        self.assertGreater(result['within_batch_order']['order_divergences_without_ties'], 0)
        self.assertEqual(result['quality']['python']['correct'], 1)
        self.assertEqual(result['quality']['rust']['incorrect'], 1)
        # Identical but wrong rankings are numerical parity, not label acceptance.
        args[2]['batches'][0]['vectors'] = copy.deepcopy(vectors)
        result = policy.evaluate(*args, config())
        self.assertEqual(result['state'], 'numeric_pass_not_acceptance')
        self.assertEqual(result['quality']['python']['incorrect'], 1)

    def test_exact_text_invariance_per_runtime_not_declared_groups(self):
        args = fixture()
        replace_text(args, 2, 'q')
        # Both runtimes agree across systems but disagree across occurrences.
        result = policy.evaluate(*args, config())
        self.assertEqual(result['state'], 'numeric_fail')
        for runtime in ('rust', 'python'):
            self.assertEqual(result['invariance'][runtime]['pairs'], 1)
            self.assertEqual(result['invariance'][runtime]['failed_pairs'], 1)
        replace_text(args, 2, 'Q')
        result = policy.evaluate(*args, config())
        self.assertEqual(result['invariance']['rust']['pairs'], 0)
        self.assertEqual(result['state'], 'numeric_pass_not_acceptance')

    def test_invariance_across_batches_and_score_limit(self):
        args = fixture()
        corpus = json.loads(args[0])
        corpus['batches'].append({'id': 'b', 'texts': ['q']})
        args[0] = json.dumps(corpus).encode()
        for runtime in args[1:3]:
            batch = copy.deepcopy(runtime['batches'][0])
            batch['id'] = 'b'
            for key in ('input_ids', 'attention_mask', 'token_type_ids', 'vectors'):
                batch[key] = batch[key][:1]
            batch['vectors'][0] = [0., 1.] + [0.] * 382
            runtime['batches'].append(batch)
            runtime['input_sha256'] = hashlib.sha256(args[0]).hexdigest()
        limits = {**config(), 'max_abs_error': 1., 'min_embedding_cosine': 0.}
        result = policy.evaluate(*args, limits)
        self.assertEqual(result['state'], 'numeric_fail')
        self.assertTrue(any(v['scope'] == 'exact_text_invariance' and v['kind'] == 'score'
                            for v in result['violations']))
        for runtime in ('rust', 'python'):
            self.assertEqual(result['invariance'][runtime]['pairs'], 1)
            self.assertEqual(result['invariance'][runtime]['failed_pairs'], 1)

    def test_labeled_cross_batch_score_failure(self):
        args = fixture()
        corpus = json.loads(args[0])
        corpus['batches'].append({'id': 'b', 'texts': ['other']})
        args[0] = json.dumps(corpus).encode()
        for runtime in args[1:3]:
            batch = copy.deepcopy(runtime['batches'][0])
            batch['id'] = 'b'
            for key in ('input_ids', 'attention_mask', 'token_type_ids', 'vectors'):
                batch[key] = batch[key][:1]
            runtime['batches'].append(batch)
            runtime['input_sha256'] = hashlib.sha256(args[0]).hexdigest()
        args[1]['batches'][1]['vectors'][0] = [0., 1.] + [0.] * 382
        args[3]['triplets'][0]['positive'] = 'b/0'
        result = policy.evaluate(*args, config())
        self.assertEqual(result['state'], 'numeric_fail')
        self.assertTrue(any(v['kind'] == 'score' and v['scope'] == 'label'
                            for v in result['violations']))

    def test_token_mismatch_and_invalid_data_fail_closed(self):
        args = fixture()
        args[1]['batches'][0]['input_ids'][0][0] = 7
        result = policy.evaluate(*args, config())
        self.assertEqual(result['state'], 'numeric_fail')
        self.assertNotIn('metrics', result['measurement'])
        args = fixture()
        args[1]['batches'][0]['vectors'][0][0] = True
        with self.assertRaises(policy.comparator.ValidationError):
            policy.evaluate(*args, config())

    def test_cli_requires_five_files_and_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            args = fixture()
            paths = [Path(directory) / name for name in ('corpus', 'rust', 'python', 'labels', 'policy')]
            paths[0].write_bytes(args[0])
            for path, value in zip(paths[1:], [*args[1:], config()]):
                path.write_text(json.dumps(value))
            argv = [item for name, path in zip(('corpus', 'rust', 'python', 'labels', 'policy'), paths)
                    for item in ('--' + name, str(path))]
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(policy.main(argv), 0)
            self.assertEqual(json.loads(out.getvalue())['state'], 'numeric_pass_not_acceptance')
            paths[-1].write_text('{"version":1,"version":2}')
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(policy.main(argv), 1)
            self.assertEqual(json.loads(out.getvalue())['state'], 'numeric_fail')
            with redirect_stdout(io.StringIO()), self.assertRaises(SystemExit):
                policy.main(argv[:-2])


if __name__ == '__main__':
    unittest.main()
