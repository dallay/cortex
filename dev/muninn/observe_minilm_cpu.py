"""Local CPU observation, NOT a verifier launcher or authentication boundary.

The operator must freshly verify the trusted bundle with the external Cargo
verifier and keep the bundle stable throughout loading/use. A supplied log is
NOT authentication: its content and the assumed status 0 are not bound to the
bundle. Review this script before real execution. No C0/C1 corpus or weights
are exported. Numeric differences are measurements, never acceptance limits.
"""
import argparse
import json
from pathlib import Path
from contextlib import contextmanager


def exact_embedding_shape(shape, batch_size):
    return tuple(shape) == (batch_size, 384)


def observation_hooks(capture, pre_observer=None, post_observer=None):
    """Read-only hooks: capture independent CPU snapshots, discard observer returns."""
    def snapshot(value):
        if isinstance(value, dict):
            return {key: snapshot(item) for key, item in value.items()}
        if isinstance(value, (list, tuple)):
            return type(value)(snapshot(item) for item in value)
        if hasattr(value, 'detach'):
            return value.detach().cpu().clone()
        return value

    def pre(module, args, kwargs):
        if pre_observer is not None:
            pre_observer(module, args, kwargs)
        capture('pre', snapshot(args[0] if args else kwargs['features']))
        return None

    def post(module, args, kwargs, output):
        if post_observer is not None:
            post_observer(module, args, kwargs, output)
        capture('post', snapshot(output))
        return None

    return pre, post


@contextmanager
def registered_observation_hooks(registrations):
    handles = []
    try:
        for module, pre, post in registrations:
            handles.append(module.register_forward_pre_hook(pre, with_kwargs=True))
            handles.append(module.register_forward_hook(post, with_kwargs=True))
        yield
    finally:
        for handle in handles:
            handle.remove()


def exact_verdict(checks):
    return 'pass' if checks and all(v is True for v in checks.values()) else 'fail'


def validate_batch(ids, masks, pad_id, cls_id, sep_id):
    """Validate exact tokenizer invariants on ordinary JSON-compatible lists."""
    if not ids or len(ids) != len(masks):
        raise ValueError('Missing or mismatched batch')
    for row, mask in zip(ids, masks):
        if not 1 <= len(row) <= 256 or len(row) != len(mask):
            raise ValueError('Invalid sequence length')
        if any(type(v) is not int or v not in (0, 1) for v in mask):
            raise ValueError('Nonbinary mask')
        n = sum(mask)
        if not n or mask != [1] * n + [0] * (len(mask) - n):
            raise ValueError('Expected nonempty right-padded mask')
        if pad_id is not None and any(v != pad_id for v in row[n:]):
            raise ValueError('Unexpected padding ID')
        if cls_id is not None and row[0] != cls_id:
            raise ValueError('Unexpected CLS ID')
        if sep_id is not None and row[n - 1] != sep_id:
            raise ValueError('Unexpected SEP ID')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--verification-log', required=True, type=Path)
    args = parser.parse_args()

    # Runtime-only imports. Importing the record validators needs only stdlib.
    import torch
    from minilm_cpu_oracle import construct_cpu_reference, encode_cpu_reference

    model = construct_cpu_reference(args.bundle, 0, args.verification_log.read_text())
    transformer = model[0]
    tokenizer = model.tokenizer
    modules = list(model)
    pooling = [m for m in modules if type(m).__name__ == 'Pooling']
    normalization = [m for m in modules if type(m).__name__ == 'Normalize']
    if len(pooling) != 1 or len(normalization) != 1:
        raise ValueError('Expected one Pooling and one Normalize module')
    pool, norm = pooling[0], normalization[0]
    report = {
        'scope': 'T3/D1 observation only; no numerical acceptance threshold',
        'trust': 'Operator owns fresh external verification and trusted stable bundle; log is NOT authentication',
        'modules': [{'class': type(m).__module__ + '.' + type(m).__name__,
                     'config': m.get_config_dict() if hasattr(m, 'get_config_dict') else {}}
                    for m in modules],
        'special_ids': {k: getattr(tokenizer, k) for k in
                        ('cls_token_id', 'sep_token_id', 'pad_token_id')},
        'batches': [], 'cases': [], 'individual_vs_batch': [],
    }
    checks = {'max_seq_length_256': model.max_seq_length == 256,
              'eval': all(not m.training for m in model.modules()),
              'parameters_cpu_float32': all(p.device.type == 'cpu' and p.dtype == torch.float32
                                            for p in model.parameters()),
              'buffers_cpu': all(b.device.type == 'cpu' for b in model.buffers())}
    active = {}
    label = ''

    def features(positional, keyword):
        return positional[0] if positional else keyword['features']

    def tensor_info(tensor):
        return {'shape': list(tensor.shape), 'device': str(tensor.device),
                'dtype': str(tensor.dtype)}

    def context():
        return {'grad_enabled': torch.is_grad_enabled(),
                'inference_mode': torch.is_inference_mode_enabled(),
                'cpu_autocast': torch.is_autocast_enabled('cpu'),
                'cuda_autocast': torch.is_autocast_enabled('cuda')}

    def record_context(stage):
        value = context()
        active[stage] = value
        checks[f'{len(report["batches"])}_{stage}_context'] = (
            value == {'grad_enabled': False, 'inference_mode': True,
                      'cpu_autocast': False, 'cuda_autocast': False})
        checks[f'{len(report["batches"])}_{stage}_eval'] = all(
            not m.training for m in model.modules())

    def transformer_pre(module, positional, keyword):
        nonlocal active
        f = features(positional, keyword)
        active = {'case': label, 'inputs': {}}
        report['batches'].append(active)
        record_context('transformer_pre')
        for key in ('input_ids', 'attention_mask', 'token_type_ids'):
            if key in f:
                active['inputs'][key] = {**tensor_info(f[key]), 'values': f[key].tolist()}
                checks[f'{len(report["batches"])}_{key}_cpu'] = f[key].device.type == 'cpu'
        ids, masks = f['input_ids'].tolist(), f['attention_mask'].tolist()
        validate_batch(ids, masks, tokenizer.pad_token_id,
                       tokenizer.cls_token_id, tokenizer.sep_token_id)
        active['lengths'] = [sum(row) for row in masks]
        # Explicit return None: never replace arguments.
        return None

    def transformer_post(module, positional, keyword, output):
        record_context('transformer_post')
        tokens = output['token_embeddings']
        active['token_embeddings'] = tensor_info(tokens)
        checks[f'{len(report["batches"])}_tokens_cpu_float32'] = (
            tokens.device.type == 'cpu' and tokens.dtype == torch.float32)
        return None

    def pooling_pre(module, positional, keyword):
        f = features(positional, keyword)
        record_context('pooling_pre')
        tokens = f['token_embeddings']
        mask = f['attention_mask'].unsqueeze(-1).expand(tokens.size()).to(tokens.dtype)
        # Independent masked mean; no in-place writes to upstream features.
        active['_mean'] = ((tokens * mask).sum(1) / mask.sum(1).clamp(min=1e-9)).detach().cpu().clone()
        return None

    def pooling_post(module, positional, keyword, output):
        record_context('pooling_post')
        pooled = output['sentence_embedding']
        mean = active.pop('_mean')
        active['pooled'] = tensor_info(pooled)
        checks[f'{len(report["batches"])}_pooled_shape'] = exact_embedding_shape(
            pooled.shape, len(active['lengths']))
        active['masked_mean_vs_pooled_maxabs'] = (
            (mean - pooled).abs().max().item() if mean.shape == pooled.shape else None)
        checks[f'{len(report["batches"])}_pooled_cpu_float32'] = (
            pooled.device.type == 'cpu' and pooled.dtype == torch.float32)
        return None

    def norm_pre(module, positional, keyword):
        record_context('normalize_pre')
        active['pre_normalize_norms'] = features(positional, keyword)[
            'sentence_embedding'].norm(p=2, dim=1).tolist()
        return None

    def norm_post(module, positional, keyword, output):
        record_context('normalize_post')
        tensor = output['sentence_embedding']
        active['post_normalize_norms'] = tensor.norm(p=2, dim=1).tolist()
        active['normalized'] = tensor_info(tensor)
        checks[f'{len(report["batches"])}_normalized_shape'] = exact_embedding_shape(
            tensor.shape, len(active['lengths']))
        checks[f'{len(report["batches"])}_normalized_cpu_float32'] = (
            tensor.device.type == 'cpu' and tensor.dtype == torch.float32)
        return None

    cases = [('small_mixed', ['hello', 'hello world', 'A longer sentence for CPU observation.']),
             ('empty_mixed', ['', ' ', 'hello'])]
    for repetitions in (253, 254, 255, 300):
        text = ' '.join(['hello'] * repetitions)
        raw = tokenizer(text, add_special_tokens=False, truncation=False)['input_ids']
        cases.append((f'hello_{repetitions}', [text]))
        report['cases'].append({'case': f'hello_{repetitions}',
                                'hello_repetitions': repetitions,
                                'tokens_without_specials': len(raw),
                                'expected_truncated_length': min(256, len(raw) + 2)})
    report['hello_tokens_without_specials'] = tokenizer(
        'hello', add_special_tokens=False, truncation=False)['input_ids']

    def encode(name, texts):
        nonlocal label
        label = name
        start = len(report['batches'])
        result = encode_cpu_reference(model, texts)
        observed = report['batches'][start:]
        checks[f'{name}_hooks_complete'] = bool(observed) and all(
            all(key in batch for key in ('transformer_post', 'pooling_post', 'normalize_post'))
            for batch in observed)
        raw_lengths = [len(tokenizer(text, add_special_tokens=False, truncation=False)[
            'input_ids']) for text in texts]
        actual_lengths = [n for entry in observed for n in entry['lengths']]
        checks[f'{name}_lengths_with_specials_and_truncation'] = (
            sorted(actual_lengths) == sorted(min(256, n + 2) for n in raw_lengths))
        checks[f'{name}_finite_output'] = bool(torch.isfinite(torch.from_numpy(result)).all().item())
        checks[f'{name}_output_shape'] = exact_embedding_shape(result.shape, len(texts))
        report['cases'].append({'case': name, 'output_shape': list(result.shape),
                                'output_norms': torch.from_numpy(result).norm(p=2, dim=1).tolist()})
        return result

    snapshots = {}
    registrations = []
    for module, pre, post in ((transformer, transformer_pre, transformer_post),
                              (pool, pooling_pre, pooling_post),
                              (norm, norm_pre, norm_post)):
        def capture(stage, value, key=type(module).__name__):
            # Keep only the latest independent CPU snapshot per stage/module.
            # Never serialize token embeddings or return a replacement.
            snapshots[(key, stage)] = value
            return None

        callbacks = observation_hooks(capture, pre, post)
        registrations.append((module, *callbacks))
    with registered_observation_hooks(registrations):
        for name, texts in cases:
            batch = encode(name, texts)
            differences = []
            for i, text in enumerate(texts):
                single = encode(f'{name}_individual_{i}', [text])
                differences.append(float(abs(single[0] - batch[i]).max()))
            report['individual_vs_batch'].append({'case': name, 'maxabs_by_input': differences})
    report['checks'] = checks
    report['exact_invariants_verdict'] = exact_verdict(checks)
    print(json.dumps(report, indent=2, allow_nan=False))
    return 0 if report['exact_invariants_verdict'] == 'pass' else 1


if __name__ == '__main__':
    raise SystemExit(main())
