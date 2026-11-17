"""Build current source offline, then capture GM08 on dory with zero tolerance.

Uses installed pinned images, separate fresh authenticated tmpfs databases and an
internal network without published ports. No external binary, archived oracle,
install, pull or tolerance option exists. Real execution belongs to T5.
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

from gm05_build import snapshot_sources, digest
from gm01_runner import _copy_file
from gm05_runner import NEO4J_IMAGE
from gm08_contract import compare_gm08
from reference import materialize_sources, verify_source_tree, verify_repository

ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / 'dev/muninn'
EVIDENCE_ROOTS = (
    ROOT / '.agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/t4-gm08',
    ROOT / '.agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/t5-live',
)
FAITHFUL = ROOT / '.agents/rpi/plan/tasks/muninn-h2-delivery/evidence/h2-current-gm05/orchestrate.py'
RUST = 'rust:1.99.1-slim@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247'
ORACLE = 'muninn-gm05-oracle:20261104'
TOOLCHAIN = '1.99.1-aarch64-unknown-linux-gnu'
HELPERS = ('gm08_runner.py', 'gm08_peer.py', 'gm08_capture.py', 'gm08_contract.py',
           'gm05_build.py', 'gm05_adapters.py', 'gm05_capture.py', 'gm05_contract.py',
           'gm05_runner.py', 'gm01_runner.py', 'gm01_differential.py', 'gm01_contract.py',
           'gm01_capture.py', 'gm01_lifecycle.py', 'product_chain_fixture.py', 'reference.py',
           'source-integrity.json', 'references.json', 'fixtures/gm08-v1.json')


def dump(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n')


def preflight(sources, cargo_caches=()):
    for cache in cargo_caches:
        if not cache.is_dir():
            raise ValueError('requested offline cargo cache is absent')
    integrity = json.loads((BASE / 'source-integrity.json').read_text())
    selected = {name: integrity[name] for name in ('agent-memory', 'agent-memory-tck')}
    pins = {'agent-memory': '9d45af5529ecb5f2fe352013468a11087f3163d4',
            'agent-memory-tck': 'e60b410f1046dc21db50f9aba48e171dc8a9b5a1'}
    for name, pin in pins.items():
        if not (sources / name).is_dir():
            raise ValueError('required verified reference checkout absent')
        if selected[name]['commit'] != pin:
            raise ValueError('reference pin mismatch')
        verify_repository(sources / name, pin)
    verify_source_tree(sources, selected)
    return selected


class Lifecycle:
    """Exact-owned resource journal; credentials never enter command artifacts.

    `runner` substitutes `subprocess.run` so deterministic tests can drive the real
    journal, redaction and cleanup paths without launching Docker.
    """
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
        # Command artifacts never carry the environment, so no credential is written.
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

    def container(self, role, image, network='none', user='1000:1000', writable=False, extra=(), command=()):
        name = 'muninn-gm08-' + role + '-' + self.tag
        args = ['docker', 'run', '-d', '--rm', '--pull=never', '--name', name,
                '--network', network, '--user', user, '--cap-drop', 'ALL',
                '--security-opt', 'no-new-privileges:true', '--memory', '8g' if role == 'build' else '2g',
                '--cpus', '4', '--pids-limit', '512']
        if not writable:
            args.append('--read-only')
        result = self.run('create-' + role, args + list(extra) + [image] + list(command))
        cid = result.stdout.strip()
        if not re.fullmatch('[0-9a-f]{64}', cid):
            raise RuntimeError('invalid owned container identity')
        self.owned.append(cid)
        details = self.run('inspect-' + role, ['docker', 'inspect', '--format',
            '{{json .Image}} {{json .Config.User}} {{json .HostConfig.ReadonlyRootfs}} {{json .HostConfig.CapDrop}} {{json .HostConfig.SecurityOpt}} {{json .HostConfig.PortBindings}}', cid])
        ports = self.run('ports-' + role, ['docker', 'inspect', '--format', '{{json .HostConfig.PortBindings}}', cid])
        if json.loads(ports.stdout) not in (None, {}):
            raise RuntimeError('published fixture ports prohibited')
        return cid

    def cleanup(self):
        """Remove and re-verify exactly the identities this invocation created."""
        outcomes = []
        owned, self.owned = self.owned, []
        for cid in reversed(owned):
            result = self.run('cleanup-container', ['docker', 'rm', '--force', '--volumes', cid], check=False)
            absent = self.run('absence-container', ['docker', 'inspect', '--format', '{{.Id}}', cid], check=False)
            outcomes.append({'id': cid, 'exit': result.returncode, 'absent': absent.returncode != 0})
        network, self.network = self.network, None
        if network:
            result = self.run('cleanup-network', ['docker', 'network', 'rm', network], check=False)
            absent = self.run('absence-network', ['docker', 'network', 'inspect', network], check=False)
            outcomes.append({'id': network, 'exit': result.returncode, 'absent': absent.returncode != 0})
        self.cleanup_outcomes.extend(outcomes)
        dump(self.output / 'cleanup.json', self.cleanup_outcomes)
        return all(item['absent'] and item['exit'] == 0 for item in self.cleanup_outcomes)


def bind(source, target, writable=False):
    return ['--mount', f'type=bind,source={source},target={target}' + ('' if writable else ',readonly')]


def execute(args, integrity):
    output = args.output.resolve()
    if not any(output != root and output.is_relative_to(root) for root in EVIDENCE_ROOTS):
        raise ValueError('output must be a NEW directory under the task evidence roots')
    if output.exists():
        raise ValueError('output must be a NEW directory')
    for parent in [output.parent, *output.parents]:
        if parent.is_symlink():
            raise ValueError('output ancestors must not be symlinks')
    output.mkdir(parents=True)
    lifecycle = Lifecycle(output)
    state = output / ('.gm08-state-' + lifecycle.tag)
    created, manifest = False, {}
    summary = {'status': 'fail', 'stage': 'prepare', 'numeric_tolerance': 0,
               'deviations': [],
               'capture_surface': {
                   'product': 'HTTP bridge wire (crates/muninn/bridge/src/wire.rs)',
                   'reference': 'native adapter projected onto the same wire.rs shape',
                   'projection': 'gm08_peer.project_wire',
                   'graph_labels': list(('Conversation', 'Message', 'Entity', 'ReasoningTrace',
                                         'ReasoningStep', 'ToolCall', 'Tool')),
               },
               'images': {}, 'providers': {},
               'reference_commit': integrity['agent-memory']['commit'],
               'tck_commit': integrity['agent-memory-tck']['commit'],
               'native_live': 'NOT_RUN', 'h3_acceptance': 'not_established',
               'harness_sha256': {name: digest(BASE / name) for name in HELPERS},
               'faithful_helper_sha256': digest(FAITHFUL)}
    try:
        if lifecycle.run('context', ['docker', 'context', 'show']).stdout.strip() != 'dory':
            raise ValueError('only dory is authorized')
        for image in (RUST, ORACLE, NEO4J_IMAGE):
            lifecycle.run('image', ['docker', 'image', 'inspect', '--format', '{{.Id}} {{.Os}} {{.Architecture}}', image])
        lifecycle.run('state-exclusion', ['git', 'check-ignore', str(state.relative_to(ROOT)) + '/'])
        state.mkdir(mode=0o700)
        created = True
        spec = importlib.util.spec_from_file_location('gm08_faithful_copy', FAITHFUL)
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        with tempfile.TemporaryDirectory(prefix='gm08-source-') as temporary:
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
        # Preserve `dev/muninn/...` under the mount: the reused GM-01/GM-05 helpers
        # resolve ROOT from their own location and fail when flattened.
        harness = state / 'harness' / 'dev' / 'muninn'
        harness.mkdir(parents=True)
        for name in HELPERS:
            _copy_file(BASE, harness, name, summary['harness_sha256'][name])
        summary['stage'] = 'build'
        def seed(target):
            return ['--mount', f'type=bind,source={target},target={target},readonly']
        seeds = [str(cache.resolve()) for cache in args.cargo_cache]
        summary['cargo_seed'] = [
            {'path': str(cache.resolve()),
             'crate_files': len(list((cache / 'cache').rglob('*.crate')))}
            for cache in args.cargo_cache
        ] + [{'volume': 'muninn-cargo-registry', 'read_only': True}]
        build_extra = ['--tmpfs', '/target:rw,exec,uid=1000,gid=1000,mode=0700,size=4g',
                       '--tmpfs', '/cargo:rw,noexec,uid=1000,gid=1000,mode=0700,size=2g',
                       '--tmpfs', '/tmp:rw,exec,uid=1000,gid=1000,mode=0700,size=256m',
                       '--mount', 'type=volume,src=muninn-cargo-registry,dst=/cache,readonly',
                       '--env', 'CARGO_HOME=/cargo', '--env', 'CARGO_TARGET_DIR=/target',
                       '--env', 'RUSTUP_TOOLCHAIN=' + TOOLCHAIN,
                       '--env', 'PATH=/usr/local/rustup/toolchains/' + TOOLCHAIN + '/bin:/usr/local/cargo/bin:/usr/local/bin:/usr/bin:/bin',
                       '--env', 'CARGO_PROFILE_DEV_DEBUG=0', '--env', 'CARGO_INCREMENTAL=0', '--workdir', '/src']
        build_extra += bind(state / 'source', '/src') + bind(state / 'built', '/built', True)
        for index, seed_path in enumerate(seeds):
            build_extra += ['--mount',
                            f'type=bind,source={seed_path},target=/seed{index},readonly']
        builder = lifecycle.container('build', RUST, extra=build_extra, command=['sleep', '3600'])
        # Copy only crate archives and index metadata: cargo unpacks sources itself, and
        # copying thousands of unpacked `src/` files hits the container file-descriptor cap.
        copy = ['mkdir -p /cargo/registry/cache /cargo/registry/index',
                'cp -R /cache/cache/. /cargo/registry/cache/',
                'cp -R /cache/index/. /cargo/registry/index/']
        copy += [f'cp -R /seed{index}/cache/. /cargo/registry/cache/' for index in range(len(seeds))]
        copy += [f'cp -R /seed{index}/index/. /cargo/registry/index/' for index in range(len(seeds))]
        lifecycle.run('cache-copy', ['docker', 'exec', builder, 'sh', '-c', ' && '.join(copy)])
        lifecycle.run('toolchain', ['docker', 'exec', builder, 'sh', '-c', 'rustc -Vv && cargo -V'])
        # The build is the completeness probe: `cargo fetch` would also require crates for
        # workspace members and platforms this target never builds. Offline still forbids
        # any download, so a missing crate aborts with its exact name.
        lifecycle.run('build', ['docker', 'exec', builder, 'cargo', 'build', '-p', 'muninn', '--bin', 'muninn', '--locked', '--offline'])
        build_hash = lifecycle.run('build-hash', ['docker', 'exec', builder, 'sha256sum', '/target/debug/muninn']).stdout.split()[0]
        lifecycle.run('export', ['docker', 'exec', builder, 'cp', '/target/debug/muninn', '/built/muninn'])
        if digest(state / 'built/muninn') != build_hash:
            raise ValueError('build/export identity mismatch')
        summary['binary_sha256'] = build_hash
        network = lifecycle.run('network', ['docker', 'network', 'create', '--internal', 'muninn-gm08-' + lifecycle.tag]).stdout.strip()
        if not re.fullmatch('[0-9a-f]{64}', network):
            raise ValueError('invalid network identity')
        lifecycle.network = network
        if lifecycle.run('internal', ['docker', 'network', 'inspect', '--format', '{{.Internal}}', network]).stdout.strip() != 'true':
            raise ValueError('fixture network must be internal')
        for side in ('reference', 'product'):
            summary['stage'] = side + '-capture'
            alias = side + '-db'
            neo = lifecycle.container(side + '-db', NEO4J_IMAGE, network, '7474:7474', True,
                ['--network-alias', alias, '--tmpfs', '/data:rw,noexec,uid=7474,gid=7474,mode=0700,size=1g',
                 '--tmpfs', '/logs:rw,noexec,uid=7474,gid=7474,mode=0700,size=128m', '--env', 'NEO4J_AUTH',
                 '--env', 'NEO4J_server_memory_heap_initial__size=256m', '--env', 'NEO4J_server_memory_heap_max__size=512m',
                 '--env', 'NEO4J_server_memory_pagecache_size=256m'])
            image_id = lifecycle.run('image-' + side, ['docker', 'inspect', '--format', '{{.Image}}', neo]).stdout.strip()
            summary['images'][side] = image_id
            if side == 'reference':
                summary['reference_neo4j_image'] = image_id
            else:
                summary['muninn_neo4j_image'] = image_id
            cypher = ['docker', 'exec', '--env', 'NEO4J_PASSWORD', neo, 'cypher-shell', '-u', 'neo4j', '--format', 'plain']
            for _ in range(120):
                if lifecycle.run('ready-' + side, cypher + ['RETURN 1'], check=False).returncode == 0:
                    break
                time.sleep(1)
            else:
                raise RuntimeError('fresh Neo4j readiness exhausted')
            wrong = lifecycle.run('reject-auth-' + side, cypher + ['RETURN 1'], check=False,
                                  env={**lifecycle.env, 'NEO4J_PASSWORD': secrets.token_urlsafe(32)})
            if wrong.returncode == 0:
                raise ValueError('authentication negative control failed')
            empty = lifecycle.run('empty-' + side, cypher + ['MATCH (n) RETURN count(n) AS count']).stdout
            if empty.strip().splitlines() != ['count', '0']:
                raise ValueError('fresh fixture not empty')
            peer_extra = ['--tmpfs', '/target:rw,exec,uid=1000,gid=1000,mode=0700,size=256m',
                          '--tmpfs', '/tmp:rw,noexec,uid=1000,gid=1000,mode=0700,size=256m',
                          '--env', 'NAM_NEO4J__URI=bolt://' + alias + ':7687', '--env', 'NAM_NEO4J__PASSWORD',
                          '--env', 'NAM_EMBEDDING__PROVIDER=hash', '--env', 'NAM_EMBEDDING__DIMENSIONS=1536',
                          '--env', 'GM08_SIDE=' + side, '--env', 'GM08_BINARY_SHA256=' + build_hash,
                          '--env', 'PYTHONDONTWRITEBYTECODE=1',
                          '--env', 'PYTHONPATH=/harness/dev/muninn', '--entrypoint', 'sh']
            peer_extra += bind(state / 'references', '/references')
            peer_extra += bind(state / 'harness', '/harness')
            peer_extra += bind(state / 'built', '/built') + bind(output, '/out', True)
            peer = lifecycle.container(side + '-peer', ORACLE, network, extra=peer_extra, command=['-c', 'sleep 1800'])
            lifecycle.run('capture-' + side, ['docker', 'exec', peer, '/opt/venv/bin/python', '-B',
                                              '/harness/dev/muninn/gm08_peer.py'])
            if not (output / (side + '.json')).is_file():
                raise ValueError('capture absent')
            runtime = output / (side + '-runtime.json')
            summary['providers'][side] = (json.loads(runtime.read_text()) if runtime.is_file()
                                          else 'runtime-identity-absent')
        summary['stage'] = 'compare'
        summary['verdict'] = compare_gm08(json.loads((output / 'reference.json').read_text()), json.loads((output / 'product.json').read_text()))
        summary['status'] = summary['verdict']['comparison']
    except Exception as error:
        summary['error'] = type(error).__name__ + ': ' + str(error)
    finally:
        try:
            summary['cleanup_verified'] = lifecycle.cleanup()
        except Exception as error:
            summary['cleanup_verified'] = False
            summary['cleanup_error'] = type(error).__name__
        summary['source_unchanged'] = all(digest(ROOT / name) == hashed for name, hashed in manifest.items())
        summary['snapshot_unchanged'] = all(digest(state / 'source' / name) == hashed for name, hashed in manifest.items())
        if created:
            # Only this invocation's unique, exclusion-checked subtree is removable.
            shutil.rmtree(state)
        summary['state_absent'] = not state.exists()
        summary['commands'] = lifecycle.results
        summary['cleanup'] = lifecycle.cleanup_outcomes
        summary['artifacts_sha256'] = {path.name: digest(path) for path in output.iterdir() if path.is_file()}
        if not all(summary.get(key) for key in ('cleanup_verified', 'source_unchanged', 'snapshot_unchanged', 'state_absent')):
            summary['status'] = 'fail'
        dump(output / 'summary.json', summary)
    return summary


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, required=True, help='Verified installed reference source checkouts')
    parser.add_argument('--output', type=Path, required=True,
                        help='NEW directory under the task evidence roots; never overwritten')
    parser.add_argument('--cargo-cache', type=Path, action='append', default=[],
                        help='Host cargo registry to seed read-only into the offline builder; '
                             'used only with --offline, no download or install')
    parser.epilog = ('Builds current workspace bytes in a private faithful copy with '
                     'cargo fetch/build --locked --offline. The shared registry volume and any '
                     'read-only --cargo-cache seeds supply already-verified crate files; the '
                     'system toolchain is trusted, not hermetic. --binary is not accepted.')
    args = parser.parse_args(argv)
    try:
        integrity = preflight(args.sources.resolve(), args.cargo_cache)
    except Exception:
        print(json.dumps({'status': 'fail', 'stage': 'preflight'}))
        return 2
    try:
        summary = execute(args, integrity)
    except Exception:
        print(json.dumps({'status': 'fail', 'stage': 'output'}))
        return 2
    print(json.dumps({'status': summary['status'], 'stage': summary['stage']}))
    return 0 if summary['status'] == 'pass' else 2


if __name__ == '__main__':
    raise SystemExit(main())
