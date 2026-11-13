"""Private C0 CPU reference export; supplied verifier logs are not authentication.

Fresh external verification and a stable trusted local bundle are preconditions.
Tokens are independently recomputed, not captured from encode's internal batches.
"""
import argparse
import hashlib
from importlib import metadata
import json
import math
import platform
import re
import sys

from minilm_cpu_oracle import construct_cpu_reference, encode_cpu_reference, verify_bundle_event

INPUT_LIMIT = 256 * 1024
BUNDLE_ID = 'ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1'
TOKEN_KEYS = ('input_ids', 'attention_mask', 'token_type_ids')


class RunError(Exception):
    """Only fixed categories cross the CLI boundary."""


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate key')
        result[key] = value
    return result


def _constant(value):
    raise ValueError('nonfinite JSON')


def _read_json(reader):
    raw = reader.read(INPUT_LIMIT + 1)
    if len(raw) > INPUT_LIMIT:
        raise ValueError('oversize')
    return json.loads(raw.decode('utf-8'), object_pairs_hook=_object,
                      parse_constant=_constant), raw


def _id(value):
    return isinstance(value, str) and re.fullmatch(r'[A-Za-z0-9_-]{1,64}', value) is not None


def read_corpus(reader):
    """Match Rust's byte limits, identifiers, exact fields and batch constraints."""
    try:
        obj, raw = _read_json(reader)
        if (type(obj) is not dict or set(obj) != {'corpus_id', 'batches'}
                or not _id(obj['corpus_id']) or not obj['corpus_id'].startswith('c0')
                or type(obj['batches']) is not list or not 1 <= len(obj['batches']) <= 32):
            raise ValueError('corpus')
        ids = set()
        for batch in obj['batches']:
            if (type(batch) is not dict or set(batch) != {'id', 'texts'}
                    or not _id(batch['id']) or batch['id'] in ids
                    or type(batch['texts']) is not list or not 1 <= len(batch['texts']) <= 32
                    or any(type(text) is not str or len(text.encode('utf-8')) > 4096
                           for text in batch['texts'])):
                raise ValueError('batch')
            ids.add(batch['id'])
        return obj, hashlib.sha256(raw).hexdigest()
    except Exception:
        raise RunError('input') from None


def export_batch(model, batch):
    try:
        # Independent original-order tokenization, NOT encode input capture.
        tokens = model.tokenize(batch['texts'])
        matrices = {key: tokens[key].tolist() for key in TOKEN_KEYS}
        count = len(batch['texts'])
        width = len(matrices['input_ids'][0])
        if not 1 <= width <= 256:
            raise ValueError('width')
        for key, rows in matrices.items():
            if (type(rows) is not list or len(rows) != count
                    or any(type(row) is not list or len(row) != width for row in rows)
                    or any(type(value) is not int or not 0 <= value <= 0xffffffff
                           or (key == 'attention_mask' and value not in (0, 1))
                           or (key == 'token_type_ids' and value != 0)
                           for row in rows for value in row)):
                raise ValueError('tokens')
    except Exception:
        raise RunError('tokenizer') from None
    try:
        vectors = encode_cpu_reference(model, batch['texts'])
        if tuple(vectors.shape) != (count, 384):
            raise ValueError('shape')
        rows = vectors.tolist()
        if (type(rows) is not list or len(rows) != count
                or any(type(row) is not list or len(row) != 384 for row in rows)
                or any(type(value) not in (float, int) or not math.isfinite(value)
                       for row in rows for value in row)):
            raise ValueError('vectors')
    except Exception:
        raise RunError('encoder') from None
    return {'id': batch['id'], **matrices, 'vectors': rows}


def runtime_receipt(model):
    """Actual Python runtime only; no inferred Rust runtime or fixture equality."""
    return {
        'packages': {name: metadata.version(name) for name in (
            'torch', 'sentence-transformers', 'transformers', 'tokenizers', 'numpy', 'safetensors')},
        'python': platform.python_version(), 'platform': platform.platform(),
        'device': 'cpu', 'dtype': 'float32',
        'attention_backend': model[0].auto_model.config._attn_implementation,
        'model_eval': model.training is False, 'max_seq_length': model.max_seq_length,
    }


def run(bundle, verification_log, input_path):
    try:
        with open(input_path, 'rb') as stream:
            corpus, digest = read_corpus(stream)
    except Exception:
        raise RunError('input') from None
    try:
        with open(verification_log, 'rb') as stream:
            raw = stream.read(65536 + 1)
        if len(raw) > 65536:
            raise ValueError('oversize')
        verification = raw.decode('utf-8')
        verify_bundle_event(0, verification)
        model = construct_cpu_reference(bundle, 0, verification)
        receipt = runtime_receipt(model)
    except Exception:
        raise RunError('artifact') from None
    batches = [export_batch(model, batch) for batch in corpus['batches']]
    return {'schema_version': 1, 'corpus_id': corpus['corpus_id'],
            'input_sha256': digest, 'artifact_bundle_id': BUNDLE_ID,
            'execution': 'reference_cpu_f32',
            'token_source': 'independent_recomputed_same_verified_cache',
            'actual_runtime': receipt, 'batches': batches}


class _Parser(argparse.ArgumentParser):
    def error(self, message):
        raise RunError('usage')


def main(argv=None):
    try:
        parser = _Parser(description=__doc__, allow_abbrev=False)
        parser.add_argument('--bundle', required=True)
        parser.add_argument('--verification-log', required=True)
        parser.add_argument('--input', required=True)
        args = parser.parse_args(argv)
        result = run(args.bundle, args.verification_log, args.input)
        try:
            output = json.dumps(result, allow_nan=False, separators=(',', ':'))
            sys.stdout.write(output + '\n')
        except Exception:
            raise RunError('output') from None
        return 0
    except RunError as error:
        category = str(error)
        if category not in {'usage', 'input', 'artifact', 'tokenizer', 'encoder', 'output'}:
            category = 'output'
        sys.stderr.write('C0 reference export failed: ' + category + '\n')
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
