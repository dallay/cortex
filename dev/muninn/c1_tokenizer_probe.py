"""Offline tokenizer-only C1 observations; never encoder or acceptance evidence."""
import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]


def probe_tokens(tokenizer, corpus, labels):
    """Record positional IDs and masks; lengths are measured before padding."""
    tokenizer.truncation_side = 'right'
    tokenizer.padding_side = 'right'
    batches = []
    for batch in corpus['batches']:
        common = dict(add_special_tokens=True, return_attention_mask=True,
                      return_token_type_ids=True)
        full = dict(tokenizer(batch['texts'], padding=False, truncation=False, **common))
        truncated = dict(tokenizer(batch['texts'], padding=False,
                                   truncation='longest_first', max_length=256, **common))
        padded = dict(tokenizer(batch['texts'], padding=True,
                                truncation='longest_first', max_length=256, **common))
        full_lengths = [len(row) for row in full['input_ids']]
        truncated_lengths = [len(row) for row in truncated['input_ids']]
        batches.append(dict(id=batch['id'], full=full, truncated=truncated, padded=padded,
                            full_lengths_including_specials=full_lengths,
                            truncated_lengths_including_specials=truncated_lengths,
                            was_truncated=[a > b for a, b in zip(full_lengths, truncated_lengths)]))
    return dict(schema_version=1, scope='tokenizer_only_not_encoder_evidence',
                acceptance_ready=False, corpus_id=corpus['corpus_id'],
                boundary_batch=labels['boundary_batch'],
                boundary_targets_from_labels=labels['boundary_targets'],
                truncation_side='right', padding_side='right',
                truncation_strategy='longest_first', max_length=256, batches=batches)


def generate(bundle, corpus_path, labels_path):
    # Set before importing transformers or any optional ML backend.
    os.environ.update(HF_HUB_OFFLINE='1', TRANSFORMERS_OFFLINE='1',
                      HF_HUB_DISABLE_TELEMETRY='1', USE_TORCH='0', USE_TF='0', USE_FLAX='0')
    from minilm_cpu_oracle import load_pinned_lock
    from minilm_tokenizer_reference import _checked_files
    from export_minilm_c1 import read_corpus
    from c1_corpus_draft import validate_labels

    lock = load_pinned_lock((ROOT / 'dev/muninn/fixtures/artifact-lock.json').read_bytes())
    model = next(item for item in lock['models']
                 if item['id'] == 'sentence-transformers/all-MiniLM-L6-v2')
    actual_versions = {name: importlib.metadata.version(name) for name in
                       ('transformers', 'tokenizers', 'huggingface-hub', 'safetensors')}
    if any(version != lock['framework_packages'][name]
           for name, version in actual_versions.items()):
        raise ValueError('Reference package version mismatch')
    requirements = (ROOT / 'dev/muninn/requirements-minilm-tokenizer.txt').read_text()
    for line in requirements.splitlines():
        if not line or line.startswith('#'):
            continue
        name, version = line.split(' --hash=sha256:', 1)[0].split('==', 1)
        if importlib.metadata.version(name) != version:
            raise ValueError('Reference dependency lock mismatch')
    with corpus_path.open('rb') as stream:
        corpus, corpus_sha256 = read_corpus(stream)
    # Bound the sidecar read before JSON decoding.
    with labels_path.open('rb') as stream:
        raw_labels = stream.read(1024 * 1024 + 1)
    if len(raw_labels) > 1024 * 1024:
        raise ValueError('Labels exceed size limit')
    labels = json.loads(raw_labels)
    validate_labels(labels, corpus, {batch['id']: batch['texts'] for batch in corpus['batches']})
    before = _checked_files(bundle, model)
    try:
        from transformers import AutoTokenizer
        tokenizer = AutoTokenizer.from_pretrained(str(bundle), local_files_only=True,
                                                  trust_remote_code=False)
        result = probe_tokens(tokenizer, corpus, labels)
    finally:
        if _checked_files(bundle, model) != before:
            raise ValueError('Bundle changed during tokenization')
    import hashlib
    result.update(corpus_sha256=corpus_sha256,
                  labels_sha256=hashlib.sha256(raw_labels).hexdigest(),
                  model_id=model['id'], revision=model['revision'],
                  tokenizer_packages=actual_versions, artifact_files=before)
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', required=True, type=Path)
    parser.add_argument('--corpus', required=True, type=Path)
    parser.add_argument('--labels', required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        result = generate(args.bundle, args.corpus, args.labels)
        print(json.dumps(result, ensure_ascii=False, sort_keys=True, allow_nan=False))
    except Exception as error:
        # Do not echo input, paths, or unbounded third-party exception messages.
        print('C1 tokenizer probe failed: ' + type(error).__name__, file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
