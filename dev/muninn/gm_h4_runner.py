"""Build current source offline, then capture GM-02/03/04 on dory with one epsilon.

Every capture runs against its own fresh authenticated tmpfs database on an
internal network with no published ports, inside the H4 oracle image. The
runner never decides the verdict: the frozen contract in `gm_h4_contract.py`
emits it from the two captures. There is no external binary, archived oracle,
install, pull, tolerance or deviation option.

Only the declared `1e-9` epsilon governs similarity scores; every other value is
compared exactly. The real U6 measurement is the only thing this runner runs.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import tempfile
import time
from uuid import uuid4

from gm01_runner import _copy_file
from gm05_build import digest, snapshot_sources
from gm05_runner import NEO4J_IMAGE
from gm08_runner import FAITHFUL, RUST, TOOLCHAIN, bind, preflight
from gm_h4_contract import (DECLARATIONS, EPSILON, GM06_DIVERGENCES, GM07_DIVERGENCES,
                            compare_gm_h4, declarations_sha256)
from reference import materialize_sources

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'dev/muninn'
EVIDENCE_ROOTS = (
    ROOT / '.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence',
)
ORACLE_SPACY = 'muninn-h4-oracle-spacy:20261105'
SIDES = ('reference', 'product')
HELPERS = (
    'gm_h4_runner.py', 'gm_h4_peer.py', 'gm_h4_capture.py', 'gm_h4_contract.py',
    'gm_h4_extract.py', 'gm_h4_search.py', 'gm08_peer.py', 'gm08_runner.py',
    'gm08_capture.py',
    'gm05_adapters.py', 'gm05_capture.py', 'gm05_contract.py', 'gm05_runner.py',
    'gm05_build.py', 'gm01_runner.py', 'spacy_service.py', 'reference.py',
    'source-integrity.json', 'fixtures/gm_h4_v1.json',
)

def dump(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n')


def declared_epsilon():
    """The single declared numeric tolerance, disclosed with its rule digest."""
    return {'epsilon': EPSILON, 'declarations_sha256': declarations_sha256()}


def summary_skeleton(reference_commit, tck_commit, images):
    return {
        'status': 'fail',
        'stage': 'prepare',
        'numeric_epsilon': EPSILON,
        'epsilon_rules': declared_epsilon(),
        'deviations': [],
        'declarations_sha256': declarations_sha256(),
        'declarations_rule_count': len(DECLARATIONS),
        'declared_divergences': {'gm06': sorted(GM06_DIVERGENCES['product']),
                                 'gm07': sorted(GM07_DIVERGENCES['reference'])},
        'provider_mode': 'recorded local fixtures; no live provider, no network',
        'harness_sha256': {},
        'capture_surface': {
            'product': 'bridge HTTP wire (crates/muninn/bridge)',
            'reference': 'pinned neo4j-agent-memory native client',
            'extraction': 'dev/muninn/spacy_service.py vs pinned SpacyEntityExtractor',
            'enrichment': 'recorded wikimedia/nominatim fixtures vs the product provider '
                          'adapters',
            'retrieval': 'search_messages/search_entities/get_related_entities/get_context',
            'graph_labels': ['Entity', 'Message', 'MENTIONS', 'RELATED_TO', 'SAME_AS'],
        },
        'images': images,
        'providers': {},
        'reference_commit': reference_commit,
        'tck_commit': tck_commit,
        'native_live': 'NOT_RUN',
        'h4_acceptance': 'not_established',
        'real_measurement': 'this invocation',
    }


def guard_output(value):
    output = Path(value).resolve()
    if not any(output.is_relative_to(root) and output != root for root in EVIDENCE_ROOTS):
        raise ValueError('output must be a NEW directory under the H4 task evidence root')
    if output.exists():
        raise ValueError('output must be a NEW directory')
    for parent in [output.parent, *output.parents]:
        if parent.is_symlink():
            raise ValueError('output ancestors must not be symlinks')
    return output


def require_capture(output, side):
    path = Path(output) / (side + '.json')
    if not path.is_file():
        raise ValueError('capture absent')
    return path


class Lifecycle:
    """Exact-owned resource journal; credentials never enter command artifacts."""

    def __init__(self, output, runner=None, password=None):
        self.output, self.tag = output, uuid4().hex
        self.password = password or secrets.token_urlsafe(32)
        self._runner = runner or subprocess.run
        self.owned, self.network = [], None
        self.cleanup_outcomes = []
        self.serial = 0
        self.env = {**os.environ, 'NEO4J_AUTH': 'neo4j/' + self.password,
                    'NAM_NEO4J__PASSWORD': self.password, 'NEO4J_PASSWORD': self.password}
        self.results = []

    def run(self, name, args, check=True, env=None):
        self.serial += 1
        stem = f'{self.serial:03d}-{name}'
        dump(self.output / (stem + '-command.json'), args)
        result = self._runner(args, env=self.env if env is None else env,
                              capture_output=True, text=True, timeout=1800)
        if self.password in result.stdout or self.password in result.stderr:
            raise RuntimeError('credential exposure blocked before artifact publication')
        for suffix, content in (('stdout', result.stdout), ('stderr', result.stderr)):
            (self.output / (stem + '.' + suffix)).write_text(content)
        self.results.append({'name': name, 'command': args, 'exit_code': result.returncode})
        dump(self.output / (stem + '-exit.json'), {'exit_code': result.returncode})
        if check and result.returncode:
            raise RuntimeError(name + ': command failed; see retained streams')
        return result

    def container(self, role, image, network='none', user='1000:1000', writable=False, extra=(),
                  command=()):
        name = 'muninn-h4-' + role + '-' + self.tag
        args = ['docker', 'run', '-d', '--rm', '--pull=never', '--name', name,
                '--network', network, '--user', user, '--cap-drop', 'ALL',
                '--security-opt', 'no-new-privileges:true',
                '--memory', '8g' if role == 'build' else '2g',
                '--cpus', '4', '--pids-limit', '512']
        if not writable:
            args.append('--read-only')
        result = self.run('create-' + role, args + list(extra) + [image] + list(command))
        cid = result.stdout.strip()
        if not re.fullmatch('[0-9a-f]{64}', cid):
            raise RuntimeError('invalid owned container identity')
        self.owned.append(cid)
        self.run('inspect-' + role, ['docker', 'inspect', '--format',
                 '{{json .Image}} {{json .Config.User}} {{json .HostConfig.ReadonlyRootfs}} '
                 '{{json .HostConfig.CapDrop}} {{json .HostConfig.SecurityOpt}} '
                 '{{json .HostConfig.PortBindings}}', cid])
        ports = self.run('ports-' + role, ['docker', 'inspect', '--format',
                                           '{{json .HostConfig.PortBindings}}', cid])
        if json.loads(ports.stdout) not in (None, {}):
            raise RuntimeError('published fixture ports prohibited')
        return cid

    def cleanup(self):
        outcomes = []
        owned, self.owned = self.owned, []
        for cid in reversed(owned):
            result = self.run('cleanup-container', ['docker', 'rm', '--force', '--volumes', cid],
                              check=False)
            absent = self.run('absence-container', ['docker', 'inspect', '--format', '{{.Id}}',
                                                    cid], check=False)
            outcomes.append({'id': cid, 'exit': result.returncode,
                             'absent': absent.returncode != 0})
        network, self.network = self.network, None
        if network:
            result = self.run('cleanup-network', ['docker', 'network', 'rm', network],
                              check=False)
            absent = self.run('absence-network', ['docker', 'network', 'inspect', network],
                              check=False)
            outcomes.append({'id': network, 'exit': result.returncode,
                             'absent': absent.returncode != 0})
        self.cleanup_outcomes.extend(outcomes)
        dump(self.output / 'cleanup.json', self.cleanup_outcomes)
        return all(item['absent'] and item['exit'] == 0 for item in self.cleanup_outcomes)


def execute(output, args, integrity):
    lifecycle = Lifecycle(output)
    state = output / ('.gm-h4-state-' + lifecycle.tag)
    created, manifest = False, {}
    summary = summary_skeleton(integrity['agent-memory']['commit'],
                               integrity['agent-memory-tck']['commit'], {})
    summary['harness_sha256'] = {name: digest(BASE / name) for name in HELPERS}
    summary['faithful_helper_sha256'] = digest(FAITHFUL)
    try:
        if lifecycle.run('context', ['docker', 'context', 'show']).stdout.strip() != 'dory':
            raise ValueError('only dory is authorized')
        for image in (RUST, ORACLE_SPACY, NEO4J_IMAGE):
            lifecycle.run('image', ['docker', 'image', 'inspect', '--format',
                                    '{{.Id}} {{.Os}} {{.Architecture}}', image])
        lifecycle.run('state-exclusion', ['git', 'check-ignore',
                                          str(state.relative_to(ROOT)) + '/'])
        state.mkdir(mode=0o700)
        created = True
        spec = importlib.util.spec_from_file_location('gm_h4_faithful_copy', FAITHFUL)
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        with tempfile.TemporaryDirectory(prefix='gm-h4-source-') as temporary:
            prepared = Path(temporary) / 'root'
            prepared.mkdir()
            proof = helper.prepare(ROOT, prepared)
            manifest = snapshot_sources(prepared, state / 'source')
        dump(output / 'faithful-copy.json', proof)
        if any(digest(ROOT / name) != hashed for name, hashed in manifest.items()):
            raise ValueError('current source differs from faithful snapshot')
        dump(output / 'source-manifest.json', manifest)
        summary['source_manifest_sha256'] = digest(output / 'source-manifest.json')
        (state / 'built').mkdir()
        materialize_sources(args.sources, integrity, state / 'references')
        harness = state / 'harness' / 'dev' / 'muninn'
        harness.mkdir(parents=True)
        for name in HELPERS:
            _copy_file(BASE, harness, name, summary['harness_sha256'][name])
        summary['stage'] = 'build'
        seeds = [str(cache.resolve()) for cache in args.cargo_cache]
        summary['cargo_seed'] = [
            {'path': str(cache.resolve()),
             'crate_files': len(list((cache / 'cache').rglob('*.crate')))}
            for cache in args.cargo_cache
        ] + [{'volume': 'muninn-cargo-registry', 'read_only': True}]
        build_extra = [
            '--tmpfs', '/target:rw,exec,uid=1000,gid=1000,mode=0700,size=4g',
            '--tmpfs', '/cargo:rw,noexec,uid=1000,gid=1000,mode=0700,size=2g',
            '--tmpfs', '/tmp:rw,exec,uid=1000,gid=1000,mode=0700,size=256m',
            '--mount', 'type=volume,src=muninn-cargo-registry,dst=/cache,readonly',
            '--env', 'CARGO_HOME=/cargo', '--env', 'CARGO_TARGET_DIR=/target',
            '--env', 'RUSTUP_TOOLCHAIN=' + TOOLCHAIN,
            '--env', ('PATH=/usr/local/rustup/toolchains/' + TOOLCHAIN
                      + '/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin'),
            '--env', 'CARGO_PROFILE_DEV_DEBUG=0', '--env', 'CARGO_INCREMENTAL=0',
            '--workdir', '/src',
        ]
        build_extra += bind(state / 'source', '/src') + bind(state / 'built', '/built', True)
        for index, seed in enumerate(seeds):
            build_extra += ['--mount', f'type=bind,source={seed},target=/seed{index},readonly']
        builder = lifecycle.container('build', RUST, extra=build_extra,
                                      command=['sleep', '3600'])
        copy = ['mkdir -p /cargo/registry/cache /cargo/registry/index',
                'cp -R /cache/cache/. /cargo/registry/cache/',
                'cp -R /cache/index/. /cargo/registry/index/']
        copy += [f'cp -R /seed{index}/cache/. /cargo/registry/cache/'
                 for index in range(len(seeds))]
        copy += [f'cp -R /seed{index}/index/. /cargo/registry/index/'
                 for index in range(len(seeds))]
        lifecycle.run('cache-copy', ['docker', 'exec', builder, 'sh', '-c', ' && '.join(copy)])
        lifecycle.run('toolchain', ['docker', 'exec', builder, 'sh', '-c', 'rustc -Vv && cargo -V'])
        lifecycle.run('build', ['docker', 'exec', builder, 'cargo', 'build', '-p', 'muninn',
                                '--bin', 'muninn', '--locked', '--offline'])
        build_hash = lifecycle.run('build-hash', ['docker', 'exec', builder, 'sha256sum',
                                                  '/target/debug/muninn']).stdout.split()[0]
        lifecycle.run('export', ['docker', 'exec', builder, 'cp', '/target/debug/muninn',
                                 '/built/muninn'])
        if digest(state / 'built/muninn') != build_hash:
            raise ValueError('build/export identity mismatch')
        summary['binary_sha256'] = build_hash
        network = lifecycle.run('network', ['docker', 'network', 'create', '--internal',
                                            'muninn-h4-' + lifecycle.tag]).stdout.strip()
        if not re.fullmatch('[0-9a-f]{64}', network):
            raise ValueError('invalid network identity')
        lifecycle.network = network
        if lifecycle.run('internal', ['docker', 'network', 'inspect', '--format',
                                      '{{.Internal}}', network]).stdout.strip() != 'true':
            raise ValueError('fixture network must be internal')
        for side in SIDES:
            summary['stage'] = side + '-capture'
            alias = side + '-db'
            neo = lifecycle.container(
                side + '-db', NEO4J_IMAGE, network, '7474:7474', True,
                ['--network-alias', alias,
                 '--tmpfs', '/data:rw,noexec,uid=7474,gid=7474,mode=0700,size=1g',
                 '--tmpfs', '/logs:rw,noexec,uid=7474,gid=7474,mode=0700,size=128m',
                 '--env', 'NEO4J_AUTH',
                 '--env', 'NEO4J_server_memory_heap_initial__size=256m',
                 '--env', 'NEO4J_server_memory_heap_max__size=512m',
                 '--env', 'NEO4J_server_memory_pagecache_size=256m'])
            image_id = lifecycle.run('image-' + side, ['docker', 'inspect', '--format',
                                                       '{{.Image}}', neo]).stdout.strip()
            summary['images'][side + '_neo4j'] = image_id
            cypher = ['docker', 'exec', '--env', 'NEO4J_PASSWORD', neo, 'cypher-shell',
                      '-u', 'neo4j', '--format', 'plain']
            for _ in range(120):
                if lifecycle.run('ready-' + side, cypher + ['RETURN 1'],
                                 check=False).returncode == 0:
                    break
                time.sleep(1)
            else:
                raise RuntimeError('fresh Neo4j readiness exhausted')
            wrong = lifecycle.run('reject-auth-' + side, cypher + ['RETURN 1'], check=False,
                                  env={**lifecycle.env,
                                       'NEO4J_PASSWORD': secrets.token_urlsafe(32)})
            if wrong.returncode == 0:
                raise ValueError('authentication negative control failed')
            empty = lifecycle.run('empty-' + side,
                                  cypher + ['MATCH (n) RETURN count(n) AS count']).stdout
            if empty.strip().splitlines() != ['count', '0']:
                raise ValueError('fresh fixture not empty')
            peer_extra = [
                '--tmpfs', '/target:rw,exec,uid=1000,gid=1000,mode=0700,size=256m',
                '--tmpfs', '/tmp:rw,noexec,uid=1000,gid=1000,mode=0700,size=256m',
                '--env', 'NAM_NEO4J__URI=bolt://' + alias + ':7687',
                '--env', 'NAM_NEO4J__PASSWORD',
                '--env', 'NAM_EMBEDDING__PROVIDER=hash',
                '--env', 'NAM_EMBEDDING__DIMENSIONS=1536',
                '--env', 'GM_H4_SIDE=' + side,
                '--env', 'GM_H4_BINARY_SHA256=' + build_hash,
                '--env', 'PYTHONDONTWRITEBYTECODE=1',
                '--env', 'PYTHONPATH=/harness/dev/muninn',
                '--entrypoint', 'sh',
            ]
            peer_extra += bind(state / 'references', '/references')
            peer_extra += bind(state / 'harness', '/harness')
            peer_extra += bind(state / 'built', '/built') + bind(output, '/out', True)
            peer = lifecycle.container(side + '-peer', ORACLE_SPACY, network,
                                       extra=peer_extra, command=['-c', 'sleep 1800'])
            lifecycle.run('capture-' + side, ['docker', 'exec', peer, '/opt/venv/bin/python',
                                              '-B', '/harness/dev/muninn/gm_h4_peer.py'])
            require_capture(output, side)
            runtime = output / (side + '-runtime.json')
            summary['providers'][side] = (json.loads(runtime.read_text()) if runtime.is_file()
                                          else 'runtime-identity-absent')
        summary['stage'] = 'compare'
        summary['verdict'] = compare_gm_h4(
            json.loads(require_capture(output, 'reference').read_text()),
            json.loads(require_capture(output, 'product').read_text()))
        summary['status'] = summary['verdict']['comparison']
    except Exception as error:
        summary['error'] = type(error).__name__ + ': ' + str(error)
    finally:
        try:
            summary['cleanup_verified'] = lifecycle.cleanup()
        except Exception as error:
            summary['cleanup_verified'] = False
            summary['cleanup_error'] = type(error).__name__
        summary['source_unchanged'] = all(digest(ROOT / name) == hashed
                                          for name, hashed in manifest.items())
        summary['snapshot_unchanged'] = all(digest(state / 'source' / name) == hashed
                                            for name, hashed in manifest.items())
        if created:
            shutil.rmtree(state, ignore_errors=False)
        summary['state_absent'] = not state.exists()
        summary['commands'] = lifecycle.results
        summary['cleanup'] = lifecycle.cleanup_outcomes
        summary['artifacts_sha256'] = {path.name: digest(path) for path in output.iterdir()
                                       if path.is_file()}
        if not all(summary.get(key) for key in ('cleanup_verified', 'source_unchanged',
                                                'snapshot_unchanged', 'state_absent')):
            summary['status'] = 'fail'
        dump(output / 'summary.json', summary)
    return summary


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, required=True,
                        help='Directory with verified agent-memory and agent-memory-tck checkouts')
    parser.add_argument('--output', type=Path, required=True,
                        help='NEW directory under the H4 task evidence root; never overwritten')
    parser.add_argument('--cargo-cache', type=Path, action='append', default=[],
                        help='Host cargo registry seeded read-only into the offline builder; '
                             'used only with --offline, no download or install')
    parser.epilog = ('Builds current workspace bytes in a private faithful copy with cargo '
                     'build --locked --offline, captures GM-02/03/04 in the H4 oracle image on '
                     'dory with fresh internal-only databases and compares them with the '
                     'pre-declared 1e-9 epsilon. An external binary path, a wider tolerance or '
                     'a deviation flag is not accepted.')
    args = parser.parse_args(argv)
    try:
        output = guard_output(args.output)
    except ValueError:
        print(json.dumps({'status': 'fail', 'stage': 'output'}))
        return 2
    try:
        integrity = preflight(args.sources.resolve(), args.cargo_cache)
    except Exception:
        print(json.dumps({'status': 'fail', 'stage': 'preflight'}))
        return 2
    output.mkdir(parents=True)
    try:
        summary = execute(output, args, integrity)
    except Exception:
        print(json.dumps({'status': 'fail', 'stage': 'output'}))
        return 2
    print(json.dumps({'status': summary['status'], 'stage': summary['stage']}))
    return 0 if summary['status'] == 'pass' else 2


if __name__ == '__main__':
    raise SystemExit(main())
