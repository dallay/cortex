"""Pre-execution freeze manifest; not execution, acceptance or authentication.

The manifest binds the exact bytes of every input that a C1 execution will
consume: corpus, labels, acceptance labels, numeric policy, artifact lock,
runtime configuration, tokenizer receipt, tokenizer gate and the exporter and
evaluation sources. It records relative paths, sizes and SHA-256 digests.

Freezing does not authorize execution. It makes the inputs verifiable before
and after a run, so a later comparison can state which bytes produced the
numbers. The manifest is filesystem-backed, exclusive on write, and refuses
symlinks so that a recorded path cannot point outside the workspace.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

SCHEMA_VERSION = 1
STATE = 'frozen_pre_execution'
CONTRACT_PATTERN = re.compile(r'[a-z0-9][a-z0-9._-]{0,63}')


class FreezeError(ValueError):
    """A frozen input is missing, changed, unsafe or outside the workspace."""


def require(condition, category='freeze'):
    if not condition:
        raise FreezeError(category)


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            digest.update(chunk)
    return digest.hexdigest()


def _safe_relative(root, path):
    root = root.resolve()
    resolved = path.resolve()
    try:
        relative = resolved.relative_to(root)
    except ValueError:
        raise FreezeError('outside_workspace') from None
    parts = relative.parts
    require(parts and all(part not in ('..', '') for part in parts), 'outside_workspace')
    return relative.as_posix()


def build(root, inputs, contract, extra=None):
    """Record the frozen inventory of the given input files."""
    require(type(contract) is str and CONTRACT_PATTERN.fullmatch(contract) is not None,
            'contract')
    require(type(inputs) is list and inputs, 'inputs')
    root = Path(root)
    entries = []
    seen = set()
    for path in inputs:
        path = Path(path)
        require(path.is_absolute(), 'absolute_input')
        relative = _safe_relative(root, path)
        require(path.is_symlink() is False, 'symlink')
        require(path.exists() and path.is_file(), 'missing_input')
        require(relative not in seen, 'duplicate_input')
        seen.add(relative)
        entries.append({'path': relative, 'size': path.stat().st_size,
                        'sha256': sha256_file(path)})
    entries.sort(key=lambda entry: entry['path'])
    manifest = {'schema_version': SCHEMA_VERSION, 'state': STATE,
                'numeric_contract_version': contract, 'inputs': entries}
    if extra:
        manifest.update(extra)
    return manifest


def verify(root, manifest):
    """Confirm that every recorded input still has its recorded bytes."""
    require(type(manifest) is dict, 'manifest')
    require(manifest.get('schema_version') == SCHEMA_VERSION, 'schema')
    require(manifest.get('state') == STATE, 'state')
    require(manifest.get('numeric_contract_version') is not None, 'contract')
    require(type(manifest.get('inputs')) is list and manifest['inputs'], 'inputs')
    root = Path(root)
    seen = set()
    for entry in manifest['inputs']:
        require(type(entry) is dict and set(entry) == {'path', 'size', 'sha256'}, 'entry')
        relative = entry['path']
        require(type(relative) is str and relative, 'entry')
        require(relative not in seen, 'duplicate_input')
        seen.add(relative)
        path = root / relative
        require(path.is_symlink() is False, 'symlink')
        require(path.exists() and path.is_file(), 'missing_input')
        require(path.stat().st_size == entry['size'], 'size')
        require(sha256_file(path) == entry['sha256'], 'sha256')
    return manifest


def write_manifest(output, manifest):
    """Write the manifest exclusively; never overwrite an existing file."""
    output = Path(output)
    require(not output.exists(), 'output_exists')
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL
    descriptor = os.open(output, flags, 0o600)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write((json.dumps(manifest, sort_keys=True, indent=2) + '\n').encode())
    except BaseException:
        output.unlink(missing_ok=True)
        raise
    return output


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--contract')
    parser.add_argument('--input', action='append')
    parser.add_argument('--output')
    parser.add_argument('--verify')
    args = parser.parse_args(argv)
    try:
        if args.verify:
            manifest = json.loads(Path(args.verify).read_bytes())
            verify(Path(args.workspace), manifest)
            print(json.dumps({'state': 'frozen_manifest_verified',
                              'inputs': len(manifest['inputs'])}, sort_keys=True))
            return 0
        if not args.contract or not args.input:
            raise FreezeError('usage')
        manifest = build(Path(args.workspace), [Path(item) for item in args.input],
                         contract=args.contract)
        if args.output:
            write_manifest(Path(args.output), manifest)
            print(json.dumps({'state': STATE, 'output': args.output,
                              'inputs': len(manifest['inputs'])}, sort_keys=True))
        else:
            print(json.dumps(manifest, sort_keys=True, indent=2))
        return 0
    except FreezeError as error:
        print(json.dumps({'state': 'freeze_failed', 'category': str(error)},
                         sort_keys=True))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
