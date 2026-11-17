"""Validate local C0 file consistency, NOT authentication or numerical acceptance.

Never launches commands. argv, runtime, exit codes and the claimed before-state
are untrusted declarations, not proof of execution or a reproducible build.
Run against a quiescent workspace; this is not an atomic filesystem snapshot.
"""
import argparse
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import re
import stat

JSON_LIMIT = 256 * 1024
FILE_LIMIT = 256 * 1024 * 1024
MAX_FILES = 256
CHUNK = 1024 * 1024
ROLES = {'rust_export', 'python_export', 'comparison', 'bundle_verify'}
STEPS = {'build', 'rust_export', 'bundle_verify', 'python_export', 'compare'}
SCOPE = 'c0_local_provenance_not_authentication'


class ValidationError(ValueError):
    """Fixed public error category, never file contents or paths."""


def require(condition, category='schema'):
    if not condition:
        raise ValidationError(category)


def exact(value, keys):
    require(type(value) is dict and set(value) == set(keys))


def text(value):
    return type(value) is str and bool(value.strip())


def sha(value):
    return type(value) is str and re.fullmatch(r'[0-9a-f]{64}', value) is not None


def parts(path):
    require(type(path) is str and path and '\\' not in path and '\x00' not in path, 'path')
    items = path.split('/')
    require(all(item not in ('', '.', '..') for item in items), 'path')
    require(not re.match(r'^[A-Za-z]:', path), 'path')
    return items


@contextmanager
def opened(root, path):
    """Walk relative components with no symlink following (POSIX)."""
    items = parts(path)
    directory = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for item in items[:-1]:
            child = os.open(item, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        fd = os.open(items[-1], os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(fd, 'rb') as stream:
            require(stat.S_ISREG(os.fstat(stream.fileno()).st_mode), 'file_access')
            yield stream
    finally:
        os.close(directory)


def pairs(items):
    result = {}
    for key, value in items:
        require(key not in result, 'duplicate_key')
        result[key] = value
    return result


def number(value):
    result = float(value)
    require(math.isfinite(result), 'nonfinite')
    return result


def constant(_):
    raise ValidationError('nonfinite')


def read_json(root, path):
    with opened(root, path) as stream:
        raw = stream.read(JSON_LIMIT + 1)
    require(len(raw) <= JSON_LIMIT, 'json_limit')
    try:
        return json.loads(raw.decode('utf-8'), object_pairs_hook=pairs,
                          parse_float=number, parse_constant=constant)
    except ValidationError:
        raise
    except (ValueError, UnicodeError, RecursionError, OverflowError):
        raise ValidationError('json') from None


def manifest(value, output=False):
    require(type(value) is list and 0 < len(value) <= MAX_FILES, 'manifest')
    result = {}
    roles = set()
    for item in value:
        exact(item, ('path', 'size', 'sha256', 'role') if output else ('path', 'size', 'sha256'))
        parts(item['path'])
        require(item['path'] not in result, 'manifest')
        require(type(item['size']) is int and 0 <= item['size'] <= FILE_LIMIT, 'file_limit')
        require(sha(item['sha256']), 'manifest')
        if output:
            require(type(item['role']) is str and item['role'] in ROLES
                    and item['role'] not in roles, 'roles')
            roles.add(item['role'])
        result[item['path']] = item
    if output:
        require(roles == ROLES, 'roles')
    return result


def check_file(root, item):
    digest = hashlib.sha256()
    size = 0
    with opened(root, item['path']) as stream:
        require(os.fstat(stream.fileno()).st_size <= FILE_LIMIT, 'file_limit')
        while True:
            chunk = stream.read(min(CHUNK, FILE_LIMIT - size + 1))
            if not chunk:
                break
            size += len(chunk)
            require(size <= FILE_LIMIT, 'file_limit')
            digest.update(chunk)
    require(size == item['size'] and digest.hexdigest() == item['sha256'], 'file_mismatch')


def validate(workspace, receipt):
    try:
        root = Path(workspace).resolve(strict=True)
        obj = read_json(root, receipt)
        exact(obj, ('schema_version', 'run_id', 'scope', 'numerical_acceptance',
                    'files_before', 'files_after', 'outputs', 'steps', 'runtime', 'bundle_id'))
        require(type(obj['schema_version']) is int and obj['schema_version'] == 1)
        require(text(obj['run_id']) and obj['scope'] == SCOPE and obj['numerical_acceptance'] is False)
        require(sha(obj['bundle_id']))
        exact(obj['runtime'], ('rustc', 'cargo', 'host'))
        require(all(text(value) for value in obj['runtime'].values()))
        before = manifest(obj['files_before'])
        after = manifest(obj['files_after'])
        outputs = manifest(obj['outputs'], output=True)
        require(sum(len(obj[key]) for key in ('files_before', 'files_after', 'outputs')) <= MAX_FILES,
                'file_limit')
        require(before == after, 'manifest_changed')
        require(not (set(after) & set(outputs)), 'manifest')
        require(type(obj['steps']) is list and len(STEPS) <= len(obj['steps']) <= len(STEPS) + 1, 'steps')
        ids = set()
        for step in obj['steps']:
            exact(step, ('id', 'argv', 'exit_code'))
            require(type(step['id']) is str and step['id'] in STEPS | {'tests'}
                    and step['id'] not in ids, 'steps')
            ids.add(step['id'])
            require(type(step['argv']) is list and step['argv']
                    and all(type(arg) is str for arg in step['argv']), 'steps')
            require(type(step['exit_code']) is int and step['exit_code'] == 0, 'steps')
        require(STEPS <= ids, 'steps')
        for item in [*after.values(), *outputs.values()]:
            check_file(root, item)
        fixtures = [item for path, item in before.items() if path.split('/')[-1] == 'minilm-c0-v1.json']
        require(len(fixtures) == 1, 'comparison')
        comparison = next(item for item in outputs.values() if item['role'] == 'comparison')
        report = read_json(root, comparison['path'])
        require(type(report) is dict and report.get('state') == 'measured_not_accepted'
                and report.get('corpus_id') == 'c0-v1'
                and report.get('input_sha256') == fixtures[0]['sha256'], 'comparison')
        return {'state': 'valid_provenance_not_acceptance', 'run_id': obj['run_id'],
                'scope': SCOPE, 'numerical_acceptance': False}
    except OSError:
        raise ValidationError('file_access') from None
    except (ValueError, RecursionError, OverflowError) as error:
        if isinstance(error, ValidationError):
            raise
        raise ValidationError('schema') from None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--workspace', required=True)
    parser.add_argument('--receipt', required=True, help='workspace-relative receipt path')
    args = parser.parse_args(argv)
    try:
        print(json.dumps(validate(args.workspace, args.receipt)))
        return 0
    except ValidationError as error:
        print(json.dumps({'state': 'invalid_receipt', 'error': str(error)}))
        return 1


if __name__ == '__main__':
    raise SystemExit(main())
