"""Exact record validation only; no tensors, model doubles or embeddings."""
import unittest

from observe_minilm_cpu import validate_batch, exact_verdict


class ObservationRecordsTest(unittest.TestCase):
    def test_valid_padded_record(self):
        validate_batch([[101, 9, 102, 0], [101, 102, 0, 0]],
                       [[1, 1, 1, 0], [1, 1, 0, 0]], 0, 101, 102)

    def test_rejects_bad_masks_padding_specials_and_length(self):
        for ids, masks in [([[101, 102]], [[1, 2]]),
                           ([[101, 102, 8]], [[1, 1, 0]]),
                           ([[101, 0, 102]], [[1, 0, 1]]),
                           ([[9, 102]], [[1, 1]]),
                           ([[101, 9]], [[1, 1]]),
                           ([[101] * 257], [[1] * 257]),
                           ([[101, 102]], [[1]]), ([], [])]:
            with self.subTest(ids=ids):
                with self.assertRaises(ValueError):
                    validate_batch(ids, masks, 0, 101, 102)

    def test_optional_special_ids(self):
        validate_batch([[7]], [[1]], None, None, None)

    def test_verdict_requires_actual_boolean_checks(self):
        self.assertEqual(exact_verdict({'cpu': True}), 'pass')
        for checks in ({}, {'cpu': False}, {'cpu': 1}, {'cpu': None}):
            self.assertEqual(exact_verdict(checks), 'fail')


class HookRegressionTest(unittest.TestCase):
    def test_callbacks_preserve_identity_and_snapshot(self):
        import torch
        from observe_minilm_cpu import observation_hooks

        class Identity(torch.nn.Module):
            def forward(self, features):
                self.seen = features
                return features

        module = Identity()
        captured = {}

        def capture(stage, snapshot):
            captured[stage] = snapshot
            return snapshot  # Must never become a PyTorch hook return value.

        def pre_observer(module, args, kwargs):
            return {'replacement': True}

        def post_observer(module, args, kwargs, output):
            return torch.tensor([99.0])

        pre, post = observation_hooks(capture, pre_observer, post_observer)
        handles = []
        try:
            handles.append(module.register_forward_pre_hook(pre, with_kwargs=True))
            handles.append(module.register_forward_hook(post, with_kwargs=True))
            for keyword in (False, True):
                features = {'value': torch.tensor([2.0])}
                output = module(features=features) if keyword else module(features)
                self.assertIs(module.seen, features)
                self.assertIs(output, features)
                self.assertIs(output['value'], features['value'])
                features['value'].add_(3)
                for stage in ('pre', 'post'):
                    self.assertEqual(captured[stage]['value'].tolist(), [2.0])
                    self.assertEqual(captured[stage]['value'].device.type, 'cpu')
                self.assertIsNone(pre(module, (features,), {}))
                self.assertIsNone(post(module, (features,), {}, features))
        finally:
            for handle in handles:
                handle.remove()

    def test_partial_registration_cleanup(self):
        import torch
        from observe_minilm_cpu import registered_observation_hooks

        class Broken(torch.nn.Module):
            def register_forward_hook(self, *args, **kwargs):
                raise RuntimeError('registration failed')

        module = Broken()
        with self.assertRaisesRegex(RuntimeError, 'registration failed'):
            with registered_observation_hooks([(module, lambda *a: None, lambda *a: None)]):
                self.fail('registration should fail first')
        self.assertEqual(len(module._forward_pre_hooks), 0)

    def test_exact_embedding_shapes(self):
        from observe_minilm_cpu import exact_embedding_shape
        self.assertTrue(exact_embedding_shape((2, 384), 2))
        for shape in ((1, 384), (2, 383), (2, 385), (2, 384, 1), (384,)):
            self.assertFalse(exact_embedding_shape(shape, 2))


if __name__ == '__main__':
    unittest.main()
