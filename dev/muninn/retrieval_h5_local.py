"""Explicit local H5 diagnostic. Not authenticated C1 execution or acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import selectors
import signal
import subprocess
import sys
import time

import retrieval_h5_diagnostics as diagnostics
from c1_identity import BUNDLE_ID
from minilm_cpu_oracle import load_pinned_lock, verify_stream

CHANNEL_LIMIT = 8 * 1024 * 1024
SANDBOX = ['/usr/bin/sandbox-exec', '-p', '(version 1)(allow default)(deny network*)']
OFFLINE = {'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1',
           'HF_HUB_DISABLE_TELEMETRY': '1', 'DO_NOT_TRACK': '1',
           'TOKENIZERS_PARALLELISM': 'false', 'OMP_NUM_THREADS': '1',
           'MKL_NUM_THREADS': '1', 'PYTHONDONTWRITEBYTECODE': '1'}


def file_record(path):
    path = Path(path)
    if any(p.is_symlink() for p in (path, *path.parents)) or not path.is_file():
        raise ValueError('material must be regular without symlinks')
    digest = hashlib.sha256()
    size = 0
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(65536), b''):
            digest.update(chunk)
            size += len(chunk)
    return {'path': str(path), 'size': size, 'sha256': digest.hexdigest()}


def capture(argv, output, env, timeout=900):
    """Bound each pipe before writing; always reap the process group on failure."""
    process = subprocess.Popen(SANDBOX + list(argv), cwd=output, env=env,
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    try:
        with selectors.DefaultSelector() as selector, (output / 'stdout.json').open('xb') as stdout, (output / 'stderr.log').open('xb') as stderr:
            selector.register(process.stdout, selectors.EVENT_READ, stdout)
            selector.register(process.stderr, selectors.EVENT_READ, stderr)
            sizes = {stdout: 0, stderr: 0}
            deadline = time.monotonic() + timeout
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValueError('exporter timeout')
                for key, _ in selector.select(min(remaining, 0.1)):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    if sizes[key.data] + len(chunk) > CHANNEL_LIMIT:
                        raise ValueError('channel limit exceeded')
                    key.data.write(chunk)
                    sizes[key.data] += len(chunk)
            return process.wait(timeout=max(0.001, deadline - time.monotonic()))
    finally:
        # Descendants may still hold pipes or continue after the leader exits.
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        process.stdout.close()
        process.stderr.close()


def verify_stages(stages):
    for stage in stages:
        for expected in stage['outputs'] + [stage['receipt']]:
            if file_record(expected['path']) != expected:
                raise ValueError('evidence changed')


def publish_receipt(output, receipt):
    """Last evidence check before publishing the local completion marker."""
    verify_stages(receipt['stages'])
    if receipt['comparison'] != file_record(output / 'comparison.json'):
        raise ValueError('comparison evidence changed')
    diagnostics.write_new(output / 'local-diagnostic.json', receipt)


def run_stage(output, argv, materials, consumed):
    """Capture a bounded, network-denied local process with byte drift checks."""
    output = Path(output)
    before = [file_record(p) for p in materials]
    binary = file_record(Path(argv[0]).resolve(strict=True))
    expected = file_record(consumed)['sha256']
    output.mkdir(mode=0o700)
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'TMPDIR', 'LANG') if key in os.environ}
    env.update(OFFLINE)
    started = time.monotonic()
    returncode = capture(argv, output, env)
    if returncode:
        raise ValueError('exporter exit: ' + str(returncode))
    if before != [file_record(p) for p in materials] or binary != file_record(Path(argv[0]).resolve(strict=True)):
        raise ValueError('material changed')
    document, _ = diagnostics.read_json(output / 'stdout.json')
    if document.get('input_sha256') != expected:
        raise ValueError('consumption hash mismatch')
    record = {'state': 'local_diagnostic_not_c1_acceptance', 'argv': list(argv),
              'binary': binary, 'materials': before, 'input_sha256': expected,
              'network_policy': 'sandbox-exec deny network*', 'offline_environment': OFFLINE,
              'elapsed_seconds': time.monotonic() - started, 'exit_code': returncode,
              'channel_limit_bytes': CHANNEL_LIMIT,
              'outputs': [file_record(output / name) for name in ('stdout.json', 'stderr.log')]}
    diagnostics.write_new(output / 'stage.json', record)
    return {**record, 'receipt': file_record(output / 'stage.json')}


def verify_model(bundle, lock):
    """Fresh pinned byte verification, not a reused historical verifier log."""
    pinned = load_pinned_lock(lock.read_bytes())
    receipt = file_record(bundle / '_muninn_artifacts.json')
    if receipt['sha256'] != BUNDLE_ID:
        raise ValueError('bundle receipt mismatch')
    model = next(m for m in pinned['models'] if m['id'] == 'sentence-transformers/all-MiniLM-L6-v2')
    records = []
    for item in model['files']:
        path = bundle / item['path']
        record = file_record(path)
        with path.open('rb') as stream:
            verify_stream(stream, item['size'], item['sha256'])
        records.append(record)
    return records, {'event': 'bundle_verified', 'bundle_id': BUNDLE_ID,
                     'files': len(records), 'artifact_bytes': sum(r['size'] for r in records),
                     'revision': model['revision'], 'inference_executed': False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True)
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[2]
    output = Path(args.output).absolute()
    # The directory itself is the exclusive attempt boundary; failed attempts stay incomplete.
    output.mkdir(mode=0o700)
    base = root / 'dev/muninn'
    lock = base / 'fixtures/artifact-lock.json'
    bundle = base / '.cache/minilm-artifacts' / BUNDLE_ID
    source = base / 'fixtures/retrieval-pilot-v1-corpus.json'
    derived = root / 'tmp/muninn/h5-singletons.json'
    manifest = root / 'tmp/muninn/h5-singletons-manifest.json'
    historical = root / 'tmp/muninn/retrieval-h5-20261107b.json'
    corpus, raw = diagnostics.read_json(source)
    expected_derived, expected_manifest = diagnostics.derive(corpus, raw)
    if (derived.read_bytes() != diagnostics.encode(expected_derived)
            or diagnostics.read_json(manifest)[0] != expected_manifest):
        raise ValueError('derived inputs mismatch')
    model_records, event = verify_model(bundle, lock)
    verification = output / 'bundle-verification.jsonl'
    diagnostics.write_new(verification, event)
    inputs = [source, derived, manifest, historical, lock, verification]
    # Bind local source bytes as traceability, never as authenticated source-to-binary proof.
    sources = sorted((root / 'crates/muninn/c1-exporter/src').glob('*.rs'))
    sources += sorted((root / 'third_party/candle-core/src').rglob('*.rs'))
    sources += sorted(base.glob('*.py'))
    sources += [root / p for p in ('Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml',
                                   'crates/muninn/c1-exporter/Cargo.toml')]
    materials = inputs + sources + [bundle / '_muninn_artifacts.json'] + [Path(r['path']) for r in model_records]
    baseline = [file_record(path) for path in materials]
    rust = root / 'target/debug/export_minilm_c1'
    python = base / '.cache/minilm-cpu-oracle-venv/bin/python'
    contract = 'h5-local-diagnostic-v1'
    stages = []
    stages.append(run_stage(output / 'rust', [str(rust), str(bundle.parent), BUNDLE_ID,
                                             str(derived), contract], materials, derived))
    stages.append(run_stage(output / 'python', [str(python), str(base / 'export_minilm_c1.py'),
                    '--bundle', str(bundle), '--verification-log', str(verification),
                    '--input', str(derived), '--numeric-contract-version', contract], materials, derived))
    if baseline != [file_record(path) for path in materials]:
        raise ValueError('material changed between stages')
    verify_model(bundle, lock)
    verify_stages(stages)
    report = diagnostics.compare(source, derived, manifest, output / 'rust/stdout.json',
                                 output / 'python/stdout.json', historical)
    diagnostics.write_new(output / 'comparison.json', report)
    receipt = {'state': 'local_diagnostic_not_c1_acceptance', 'authentication': 'none_local_hashes_only',
               'parity_accepted': False, 'platform': platform.platform(),
               'controller_python': sys.version, 'model_files': model_records,
               'stages': stages, 'comparison': file_record(output / 'comparison.json')}
    publish_receipt(output, receipt)
    print(json.dumps({'state': receipt['state'], 'output': str(output)}))


if __name__ == '__main__':
    main()
