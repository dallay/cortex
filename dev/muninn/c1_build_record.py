"""Produce CI build metadata; authentication is supplied separately by Actions."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

import c1_provenance

TREES = ('crates/muninn/c1-exporter', 'third_party/candle-core')
FILES = (
    'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml',
    'docs/muninn/evidence/artifact-lock.json',
    'dev/muninn/c1_build_record.py', 'dev/muninn/c1_provenance.py',
    'dev/muninn/c1_runner.py', 'dev/muninn/c1_attempt_run.py',
    'dev/muninn/c1_attempt_plan.json', 'dev/muninn/c1_manifest_build.py',
    'dev/muninn/fixtures/minilm-c1-contract-cases.json',
    'dev/muninn/fixtures/minilm-c1-identity-golden.json',
    'dev/muninn/minilm_tokenizer_reference.py',
    'dev/muninn/requirements-minilm-tokenizer.txt',
    '.github/workflows/ci.yml',
)


def _reject_symlink_components(root, relative):
    current = root
    for component in relative.parts:
        current = current / component
        if current.is_symlink():
            raise ValueError('symlink_material')


def materials(root):
    root = Path(root).resolve(strict=True)
    paths = []
    for name in FILES:
        relative = Path(name)
        path = root / relative
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('unsafe_material_path')
        _reject_symlink_components(root, relative)
        if not path.is_file():
            raise ValueError('missing_or_unsafe_material')
        paths.append(path)
    for tree in TREES:
        relative = Path(tree)
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('unsafe_source_tree')
        _reject_symlink_components(root, relative)
        base = root / relative
        if not base.is_dir():
            raise ValueError('missing_or_unsafe_source_tree')
        for current, directories, filenames in os.walk(base):
            kept = []
            for name in sorted(directories):
                path = Path(current) / name
                if name in {'target', '.git', '__pycache__'}:
                    continue
                if path.is_symlink():
                    raise ValueError('symlink_source_tree')
                kept.append(name)
            directories[:] = kept
            for name in sorted(filenames):
                path = Path(current) / name
                if path.is_symlink():
                    raise ValueError('symlink_source_file')
                if path.is_file():
                    paths.append(path)
    unique = {path.relative_to(root).as_posix(): path for path in paths}
    result = []
    for relative, path in sorted(unique.items()):
        raw = path.read_bytes()
        result.append({'path': relative, 'size': len(raw),
                       'sha256': hashlib.sha256(raw).hexdigest()})
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    root = Path.cwd()
    # CI checkout must contain no local edits to tracked build materials.
    subprocess.run(['git', 'diff', '--exit-code', 'HEAD', '--', *FILES, *TREES], check=True)
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
    data = {'schema_version': 1, 'repository': c1_provenance.REPOSITORY,
            'workflow': c1_provenance.WORKFLOW, 'source_ref': 'refs/heads/main',
            'source_commit': commit, 'target': c1_provenance.TARGET,
            'package': 'muninn-c1-exporter', 'features': ['cpu-f32'],
            'toolchain': subprocess.check_output(['rustc', '--version'], text=True).strip(),
            'materials': materials(root),
            'binary_sha256': hashlib.sha256(Path(args.binary).read_bytes()).hexdigest()}
    with open(args.output, 'x', encoding='utf-8') as stream:
        json.dump(data, stream, sort_keys=True, indent=2)
        stream.write('\n')


if __name__ == '__main__':
    main()
