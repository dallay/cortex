"""Prepare and run the isolated GM-01 Compose capture."""

import argparse
import asyncio
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from gm01_capture import ReferenceAdapter, RustBridgeAdapter, capture_gm01

# Keep capability class identity when invoked as a script.
if __name__ == '__main__':
    sys.modules['gm01_runner'] = sys.modules[__name__]
from gm01_differential import compare

ROOT = Path(__file__).resolve().parents[2]
PIN = '9d45af5529ecb5f2fe352013468a11087f3163d4'
NEO4J_IMAGE = ('neo4j:5.26-community@sha256:'
               '414adfe41c6ff32abb752be37f265c77607c166cab298f65a917fc4f05776218')
RUST_BUILDER_IMAGE = ('rust:1.99.1-slim@sha256:'
                      'b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247')
PYTHON_IMAGE = ('python:3.13.18-slim@sha256:'
                '76baad6037690d6e8c4397fbcaef17bca6f19eecef822b708fe8fa3417d30bb5')
TARGET_SOURCE_DIRS = (
    'apps/muninn/src',
    'crates/muninn/core/src',
    'crates/muninn/application/src',
    'crates/muninn/neo4j/src',
    'crates/muninn/providers/src',
    'crates/muninn/bridge/src',
)
EXCLUDED_INCLUDE_SOURCES = {
    # This optional feature is disabled by the pinned default build command.
    'crates/muninn/providers/src/minilm.rs',
}
INCLUDE_MACRO = re.compile(r'\binclude_(?:bytes|str)?\s*!')
INCLUDE_LITERAL = re.compile(
    r'\binclude_(?:bytes|str)?\s*!\s*\(\s*("(?:\\.|[^"\\])*")', re.DOTALL
)
EXCLUDED_CONTEXT_PARTS = {'.git', '.cache', 'target', 'node_modules', 'dist', 'build'}
BRIDGE_STARTUP_SECONDS = 180
GM01_COMPOSE_SERVICES = frozenset({'reference-db', 'rust-db', 'capture'})
CONTAINER_ID = re.compile(r'[a-f0-9]{12,64}')
CONTAINER_STATES = frozenset({
    'created', 'restarting', 'running', 'removing', 'paused', 'exited', 'dead',
})
HEALTH_STATES = frozenset({'starting', 'healthy', 'unhealthy'})


def clean_env():
    """Return the explicit environment passed to the Rust child process."""
    return {'PATH': os.defpath, 'LANG': 'C.UTF-8', 'PYTHONDONTWRITEBYTECODE': '1'}


def docker_env():
    """Use the operator's current Docker configuration without inheriting overrides."""
    path = os.defpath + os.pathsep + '/opt/homebrew/bin:/usr/local/bin'
    return {'PATH': path, 'HOME': str(Path.home()), 'LANG': 'C.UTF-8'}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_linux_arm64_binary(path):
    with Path(path).open('rb') as binary:
        header = binary.read(20)
    if (len(header) < 20 or header[:4] != b'\x7fELF' or header[4] != 2
            or header[5] != 1 or int.from_bytes(header[18:20], 'little') != 183):
        raise RuntimeError('El candidato no es Linux arm64 ELF')
    return 'ELF64 little-endian AArch64'


def _validate_cargo_config(path):
    text = path.read_text(encoding='utf-8')
    try:
        value = tomllib.loads(text)
    except tomllib.TOMLDecodeError:
        raise RuntimeError('Configuración Cargo inválida') from None

    def secret_like_key(key):
        normalized = re.sub(r'[^a-z0-9]', '', key.casefold())
        if normalized in {'authrequired', 'credentialprovider', 'credentialproviders'}:
            return False
        return normalized == 'auth' or any(
            marker in normalized for marker in (
                'token', 'password', 'passwd', 'secret', 'credential',
                'authorization', 'apikey', 'accesskey', 'privatekey',
                'clientcertificate',
            )
        )

    def visit(item):
        if isinstance(item, dict):
            for key, child in item.items():
                if secret_like_key(key):
                    raise RuntimeError('Configuración Cargo contiene credenciales')
                visit(child)
        elif isinstance(item, list):
            for child in item:
                visit(child)
        elif isinstance(item, str) and (
                re.search(r'(?:https?|sparse\+https?)://[^/\s@]+@', item)
                or re.search(r'(?i)[?&](?:token|password|auth|key)=', item)):
            raise RuntimeError('Configuración Cargo contiene credenciales')

    visit(value)


def _include_dependencies(root, source_paths):
    dependencies = set()
    for source in source_paths:
        relative = source.relative_to(root).as_posix()
        if relative in EXCLUDED_INCLUDE_SOURCES:
            continue
        if any(part in ('tests', 'examples', 'benches') for part in source.parts):
            continue
        text = source.read_text(encoding='utf-8')
        matches = list(INCLUDE_LITERAL.finditer(text))
        if len(matches) != len(list(INCLUDE_MACRO.finditer(text))):
            raise RuntimeError(f'Macro include no literal en {relative}')
        for match in matches:
            try:
                included = json.loads(match.group(1))
            except json.JSONDecodeError:
                raise RuntimeError(f'Ruta include inválida en {relative}') from None
            path = (source.parent / included).resolve()
            try:
                path.relative_to(root.resolve())
            except ValueError:
                raise RuntimeError(f'Include fuera del workspace en {relative}') from None
            if not path.is_file() or path.is_symlink():
                raise RuntimeError(f'Falta el recurso include de {relative}')
            dependencies.add(path)
    return dependencies


def source_hashes(root):
    """Hash the Cargo workspace inputs and active Muninn compile-time assets."""
    root = Path(root).resolve()
    paths = {root / 'Cargo.lock', root / 'Cargo.toml', root / 'rust-toolchain.toml'}
    for directory in ('apps', 'crates', 'third_party'):
        base = root / directory
        if not base.exists():
            continue
        paths.update(path for path in base.rglob('*')
                     if path.is_file() and not path.is_symlink()
                     and not EXCLUDED_CONTEXT_PARTS.intersection(path.parts)
                     and (path.suffix == '.rs' or path.name == 'Cargo.toml'))
    cargo_directory = root / '.cargo'
    if cargo_directory.is_symlink():
        raise RuntimeError('El directorio Cargo no puede ser symlink')
    for name in ('config.toml', 'config'):
        cargo_config = cargo_directory / name
        if cargo_config.is_symlink():
            raise RuntimeError('La configuración Cargo no puede ser symlink')
        if cargo_config.is_file():
            _validate_cargo_config(cargo_config)
            paths.add(cargo_config)

    target_sources = []
    for directory in TARGET_SOURCE_DIRS:
        base = root / directory
        if base.exists():
            target_sources.extend(path for path in base.rglob('*.rs')
                                  if path.is_file() and not path.is_symlink()
                                  and not any(part in ('tests', 'examples', 'benches')
                                              for part in path.parts))
    paths.update(_include_dependencies(root, target_sources))

    return {path.relative_to(root).as_posix(): digest(path)
            for path in sorted(paths) if path.is_file() and not path.is_symlink()}


@dataclass
class BuiltArtifact:
    path: Path
    manifest: dict

    def verify(self):
        if digest(self.path) != self.manifest['binary_sha256']:
            raise RuntimeError('Cambió el binario privado')
        verify_linux_arm64_binary(self.path)


def _copy_file(root, context, relative, expected_hash):
    source = root / relative
    if source.is_symlink() or not source.is_file() or digest(source) != expected_hash:
        raise RuntimeError(f'Fuente cambió o no es regular: {relative}')
    destination = context / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)
    if digest(destination) != expected_hash:
        raise RuntimeError(f'Error al copiar fuente: {relative}')


HARNESS_FILES = ('gm01_runner.py', 'gm01_capture.py', 'gm01_differential.py',
                 'gm01_contract.py', 'gm01_lifecycle.py')


def stage_build_context(root, reference_source, destination):
    """Create a fresh context containing only hashed workspace inputs and the verified package."""
    root = Path(root).resolve()
    reference_source = Path(reference_source).resolve()
    destination = Path(destination)
    integrity = json.loads((ROOT / 'dev/muninn/source-integrity.json').read_text())
    verified_reference = integrity['agent-memory']
    if verified_reference['commit'] != PIN:
        raise RuntimeError('Pin de referencia incorrecto')
    destination.mkdir(parents=True, exist_ok=False, mode=0o700)
    sources = source_hashes(root)
    for relative, expected_hash in sources.items():
        _copy_file(root, destination, relative, expected_hash)
    (destination / '.cargo').mkdir(exist_ok=True)
    cargo_config_context = destination / '.cargo/config.toml'
    if not cargo_config_context.exists():
        legacy_cargo_config = destination / '.cargo/config'
        if legacy_cargo_config.exists():
            cargo_config_context = legacy_cargo_config
        else:
            cargo_config_context.write_text('# No workspace Cargo configuration.\n',
                                            encoding='utf-8')

    expected_package = {
        name.removeprefix('src/neo4j_agent_memory/'): expected
        for name, expected in verified_reference['files'].items()
        if name.startswith('src/neo4j_agent_memory/')
    }
    reference_package = reference_source / 'neo4j_agent_memory'
    if not reference_package.is_dir():
        raise RuntimeError('Paquete de referencia verificado ausente')
    reference_hashes = {}
    for source in sorted(reference_package.rglob('*')):
        if source.is_symlink():
            raise RuntimeError('La referencia materializada contiene symlink')
        if source.is_file():
            relative = source.relative_to(reference_package).as_posix()
            expected_hash = digest(source)
            if expected_package.get(relative) != expected_hash:
                raise RuntimeError('El paquete de referencia no coincide con source-integrity')
            destination_file = destination / 'reference/src/neo4j_agent_memory' / relative
            destination_file.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination_file)
            if digest(destination_file) != expected_hash:
                raise RuntimeError('Error al copiar paquete de referencia')
            reference_hashes[relative] = expected_hash
    if reference_hashes != expected_package:
        raise RuntimeError('El paquete de referencia tiene archivos extra o ausentes')

    license_path = reference_source.parent / 'LICENSE'
    expected_license = verified_reference['files']['LICENSE']
    if (license_path.is_symlink() or not license_path.is_file()
            or digest(license_path) != expected_license):
        raise RuntimeError('La licencia de referencia no coincide con source-integrity')
    shutil.copyfile(license_path, destination / 'reference/LICENSE')

    harness = destination / 'harness'
    harness.mkdir()
    harness_hashes = {}
    for name in HARNESS_FILES:
        source = ROOT / 'dev/muninn' / name
        shutil.copyfile(source, harness / name)
        harness_hashes[name] = digest(harness / name)
    requirements = ROOT / 'dev/muninn/requirements-oracle.txt'
    shutil.copyfile(requirements, destination / 'requirements-oracle.txt')
    shutil.copyfile(ROOT / 'dev/muninn/Dockerfile.gm01', destination / 'Dockerfile.gm01')

    provenance = {
        'reference_commit': PIN,
        'reference_integrity_sha256': digest(ROOT / 'dev/muninn/source-integrity.json'),
        'reference_package_sha256': reference_hashes,
        'reference_license_sha256': expected_license,
        'requirements_oracle_sha256': digest(requirements),
        'capture_harness_sha256': harness_hashes,
        'dockerfile_gm01_sha256': digest(destination / 'Dockerfile.gm01'),
        'cargo_config_context_sha256': digest(cargo_config_context),
        'sources': sources,
        'cargo_lock_sha256': sources['Cargo.lock'],
        'rust_toolchain_sha256': sources['rust-toolchain.toml'],
        'build_command': ['cargo', '+1.99.1', 'build', '--locked', '--release',
                          '-p', 'muninn', '--bin', 'muninn'],
        'rust_builder_image': RUST_BUILDER_IMAGE,
        'python_runtime_image': PYTHON_IMAGE,
    }
    (destination / 'build-provenance.json').write_text(
        json.dumps(provenance, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    return destination


def _yaml(value):
    return json.dumps(str(value), ensure_ascii=False)


def compose_document(context, evidence, reference_secret, rust_secret, project, uid, gid):
    """Render the fixed three-service internal-only stack."""
    if not re.fullmatch(r'muninn-gm01-[a-f0-9]+', project):
        raise ValueError('Nombre Compose fuera del espacio privado')
    if uid <= 0 or gid < 0:
        raise ValueError('El capturador requiere UID no-root')
    context = Path(context).resolve()
    evidence = Path(evidence).resolve()
    reference_secret = Path(reference_secret).resolve()
    rust_secret = Path(rust_secret).resolve()
    services = []
    for name, secret in (('reference-db', 'reference_auth'), ('rust-db', 'rust_auth')):
        services.extend([
            f'  {name}:',
            f'    image: {_yaml(NEO4J_IMAGE)}',
            '    platform: linux/arm64',
            '    user: "7474:7474"',
            '    cap_drop: [ALL]',
            '    security_opt: ["no-new-privileges:true"]',
            '    cpus: 2',
            '    mem_limit: 2g',
            '    environment:',
            f'      NEO4J_AUTH_FILE: /run/secrets/{secret}',
            '      NEO4J_server_memory_heap_initial__size: 256m',
            '      NEO4J_server_memory_heap_max__size: 512m',
            '      NEO4J_server_memory_pagecache_size: 256m',
            f'    secrets: [{secret}]',
            '    tmpfs:',
            '      - /data:uid=7474,gid=7474,mode=0700',
            '      - /logs:uid=7474,gid=7474,mode=0700',
            '    networks: [gm01]',
            '    healthcheck:',
            '      test: ["CMD", "wget", "--quiet", "--spider", "http://localhost:7474"]',
            '      interval: 5s',
            '      timeout: 5s',
            '      retries: 24',
            '      start_period: 20s',
            '',
        ])
    services.extend([
        '  capture:',
        f'    image: muninn-gm01-capture:{project.removeprefix("muninn-gm01-")}',
        '    platform: linux/arm64',
        '    build:',
        f'      context: {_yaml(context)}',
        '      dockerfile: Dockerfile.gm01',
        '      platforms: [linux/arm64]',
        f'    user: "{uid}:{gid}"',
        '    read_only: true',
        '    cap_drop: [ALL]',
        '    security_opt: ["no-new-privileges:true"]',
        '    cpus: 1',
        '    mem_limit: 768m',
        '    logging:',
        '      driver: none',
        '    environment:',
        '      HOME: /home/capture',
        '      PYTHONDONTWRITEBYTECODE: "1"',
        '    tmpfs:',
        f'      - /tmp:uid={uid},gid={gid},mode=0700,size=64m',
        f'      - /home/capture:uid={uid},gid={gid},mode=0700,size=64m',
        '    secrets: [reference_auth, rust_auth]',
        '    volumes:',
        '      - type: bind',
        f'        source: {_yaml(evidence)}',
        '        target: /evidence',
        '        read_only: false',
        '    networks: [gm01]',
        '    depends_on:',
        '      reference-db:',
        '        condition: service_healthy',
        '      rust-db:',
        '        condition: service_healthy',
        '    command: ["--container", "--evidence", "/evidence"]',
        '',
        'secrets:',
        '  reference_auth:',
        f'    file: {_yaml(reference_secret)}',
        '  rust_auth:',
        f'    file: {_yaml(rust_secret)}',
        '',
        'networks:',
        '  gm01:',
        '    internal: true',
        '',
    ])
    return 'services:\n' + '\n'.join(services)


def compose_command(compose_path, project, *arguments):
    if not re.fullmatch(r'muninn-gm01-[a-f0-9]+', project):
        raise ValueError('Nombre Compose fuera del espacio privado')
    return ['docker', 'compose', '-f', str(Path(compose_path).resolve()),
            '-p', project, *arguments]


def compose_run(compose_path, project, *arguments):
    try:
        result = subprocess.run(compose_command(compose_path, project, *arguments),
                                env=docker_env(), stdin=subprocess.DEVNULL,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                check=False, timeout=3600)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError('Operación Compose fallida') from None
    return result.returncode


def inspect_capture_image(project):
    """Return the immutable ID for this run's uniquely tagged capture image."""
    if not re.fullmatch(r'muninn-gm01-[a-f0-9]+', project):
        raise RuntimeError('Proyecto Compose fuera del espacio privado')
    image = 'muninn-gm01-capture:' + project.removeprefix('muninn-gm01-')
    try:
        result = subprocess.run(
            ['docker', 'image', 'inspect', '--format', '{{.Id}}', image],
            env=docker_env(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, check=False, timeout=30, text=True)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError('Inspección de imagen fallida') from None
    image_id = result.stdout
    if (result.returncode != 0 or not isinstance(image_id, str)
            or re.fullmatch(r'sha256:[a-f0-9]{64}', image_id.strip()) is None):
        raise RuntimeError('ID de imagen no válido')
    return {'reference': image, 'id': image_id.strip()}


def inspect_neo4j_image():
    """Require the digest-pinned Neo4j image locally without pulling it."""
    try:
        result = subprocess.run(
            ['docker', 'image', 'inspect', '--format', '{{.Id}}', NEO4J_IMAGE],
            env=docker_env(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, check=False, timeout=30, text=True)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError('Inspección de imagen Neo4j fallida') from None
    image_id = result.stdout
    if (result.returncode != 0 or not isinstance(image_id, str)
            or re.fullmatch(r'sha256:[a-f0-9]{64}', image_id.strip()) is None):
        raise RuntimeError('ID de imagen Neo4j no válido')
    return {'reference': NEO4J_IMAGE, 'id': image_id.strip()}


def _safe_container_state(container):
    """Return only allowlisted Compose service state fields from docker inspect."""
    if not isinstance(container, dict):
        raise ValueError('Respuesta de inspección no válida')
    config = container.get('Config')
    labels = config.get('Labels') if isinstance(config, dict) else None
    labels = labels if isinstance(labels, dict) else {}
    service = labels.get('com.docker.compose.service')
    if not isinstance(service, str) or service not in GM01_COMPOSE_SERVICES:
        return None

    state = container.get('State')
    state = state if isinstance(state, dict) else {}
    status = state.get('Status')
    health = state.get('Health')
    health = health if isinstance(health, dict) else {}
    health_status = health.get('Status')
    exit_code = state.get('ExitCode')
    oom_killed = state.get('OOMKilled')
    return {
        'service': service,
        'state': status if isinstance(status, str) and status in CONTAINER_STATES else None,
        'exit_code': exit_code if isinstance(exit_code, int) and not isinstance(exit_code, bool)
        else None,
        'oom_killed': oom_killed if isinstance(oom_killed, bool) else None,
        'health_status': health_status if isinstance(health_status, str)
        and health_status in HEALTH_STATES else None,
    }


def inspect_compose_project(project):
    """Collect minimal state for containers discovered by this exact Compose project label."""
    if not re.fullmatch(r'muninn-gm01-[a-f0-9]+', project):
        raise ValueError('Proyecto Compose fuera del espacio privado')
    try:
        listed = subprocess.run(
            ['docker', 'ps', '-aq', '--filter',
             f'label=com.docker.compose.project={project}'],
            env=docker_env(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, check=False, timeout=30, text=True)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError('Diagnóstico Docker fallido') from None
    if listed.returncode != 0 or not isinstance(listed.stdout, str):
        raise RuntimeError('Diagnóstico Docker fallido')

    container_ids = listed.stdout.split()
    if any(CONTAINER_ID.fullmatch(container_id) is None for container_id in container_ids):
        raise ValueError('ID de contenedor no válido')
    if len(container_ids) != len(set(container_ids)):
        raise ValueError('ID de contenedor duplicado')
    if not container_ids:
        return {'status': 'no_containers', 'services': []}

    try:
        inspected = subprocess.run(
            ['docker', 'inspect', *container_ids], env=docker_env(),
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            check=False, timeout=30, text=True)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError('Diagnóstico Docker fallido') from None
    if inspected.returncode != 0 or not isinstance(inspected.stdout, str):
        raise RuntimeError('Diagnóstico Docker fallido')
    try:
        records = json.loads(inspected.stdout)
    except json.JSONDecodeError:
        raise ValueError('Respuesta de inspección no válida') from None
    if not isinstance(records, list) or len(records) != len(container_ids):
        raise ValueError('Respuesta de inspección no válida')
    services = [diagnostic for record in records
                if (diagnostic := _safe_container_state(record)) is not None]
    return {'status': 'containers_found', 'services': services}


def merge_capture_image(evidence_manifest, capture_image, neo4j_image=None):
    """Add host-inspected image identities without replacing capture evidence."""
    path = Path(evidence_manifest)
    if path.is_symlink() or not path.is_file():
        raise RuntimeError('Manifiesto de captura no válido')
    try:
        value = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, json.JSONDecodeError):
        raise RuntimeError('Manifiesto de captura no válido') from None
    if not isinstance(value, dict):
        raise RuntimeError('Manifiesto de captura no válido')
    value['capture_image'] = capture_image
    if neo4j_image is not None:
        value['neo4j_image'] = neo4j_image
    temporary = path.with_name('manifest.json.tmp')
    try:
        with temporary.open('x', encoding='utf-8') as stream:
            json.dump(value, stream, indent=2, ensure_ascii=False)
            stream.write('\n')
        os.replace(temporary, path)
    except OSError:
        try:
            temporary.unlink()
        except OSError:
            pass
        raise RuntimeError('No se pudo actualizar el manifiesto') from None


def write_auth_file(path, password):
    path = Path(path)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o444)
    with os.fdopen(descriptor, 'w', encoding='utf-8') as stream:
        stream.write('neo4j/' + password)
    path.chmod(0o444)


@contextmanager
def temporary_compose_secrets(root=ROOT):
    """Create a private temporary directory for Compose secret files in the checkout cache."""
    root = Path(root).resolve()
    cache = root / 'dev/muninn/.cache'
    if cache.is_symlink():
        raise RuntimeError('El caché de secretos no puede ser symlink')
    try:
        resolved_cache = cache.resolve(strict=True)
        resolved_cache.relative_to(root)
    except (OSError, ValueError):
        raise RuntimeError('El caché de secretos debe estar dentro del checkout') from None
    if not resolved_cache.is_dir():
        raise RuntimeError('El caché de secretos no es un directorio')

    with tempfile.TemporaryDirectory(prefix='muninn-gm01-secrets-',
                                     dir=resolved_cache) as temporary:
        private = Path(temporary)
        private.chmod(0o700)
        yield private


@dataclass
class Database:
    """Connection details for one Compose-owned ephemeral Neo4j service."""
    uri: str
    password: str
    username: str = 'neo4j'
    active: bool = True

    def close(self):
        self.active = False


class StartupFailure(RuntimeError):
    def __init__(self, stage, cause, **details):
        self.diagnostic = dict(stage=stage, cause=cause, **details)
        super().__init__(f'{stage}: {cause}')


class OwnedFixture:
    """Bridge child and connection owned by this capture process."""
    def __init__(self, artifact, private, database):
        self.artifact = artifact
        self.private = Path(private)
        self.database = database
        self.active = False
        self.process = None
        self.output = None
        self.process_log = None
        self.base_url = None
        self.startup_stage = 'bridge_process'

    def start(self):
        try:
            self._start()
        except StartupFailure:
            self._save_process_diagnostic()
            raise
        except Exception:
            self._save_process_diagnostic()
            raise StartupFailure(self.startup_stage, 'operation_failed') from None

    def _save_process_diagnostic(self):
        if self.output is None:
            return
        self.output.flush()
        size = os.fstat(self.output.fileno()).st_size
        data = os.pread(self.output.fileno(), min(size, 65536), max(0, size - 65536))
        text = data.decode('utf-8', errors='replace')
        if self.database and self.database.password:
            text = text.replace(self.database.password, '[REDACTED]')
        descriptor, name = tempfile.mkstemp(prefix='bridge-diagnostic-', suffix='.log',
                                             dir=self.private)
        self.process_log = Path(name)
        with os.fdopen(descriptor, 'w', encoding='utf-8') as stream:
            stream.write(text)

    def _start(self):
        self.startup_stage = 'bridge_process'
        self.artifact.verify()
        self.startup_stage = 'database_connection'
        if not self.database or not self.database.active:
            raise RuntimeError('Base efímera no activa')
        self.startup_stage = 'bridge_process'
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        env = clean_env()
        env.update(HOME=str(self.private), NAM_NEO4J__URI=self.database.uri,
                   NAM_NEO4J__USERNAME=self.database.username,
                   NAM_NEO4J__PASSWORD=self.database.password,
                   NAM_NEO4J__DATABASE='neo4j', NAM_EMBEDDING__PROVIDER='none')
        self.output = tempfile.TemporaryFile(dir=self.private)
        self.process = subprocess.Popen([str(self.artifact.path), 'bridge', '--host',
                                         '127.0.0.1', '--port', str(port)],
                                        cwd=self.private, env=env,
                                        stdout=self.output, stderr=self.output)
        self.base_url = f'http://127.0.0.1:{port}'
        self.startup_stage = 'bridge_readiness'
        deadline = time.monotonic() + BRIDGE_STARTUP_SECONDS
        status = None
        probe = 'not_attempted'
        while True:
            code = self.process.poll()
            cause = 'early_exit' if code is not None else None
            remaining = deadline - time.monotonic()
            if cause or remaining <= 0:
                raise StartupFailure('bridge_readiness', cause or 'timeout',
                                     exit_code=code, last_http_status=status,
                                     last_probe=probe, budget_seconds=BRIDGE_STARTUP_SECONDS)
            try:
                status, _ = self._request('setup', {}, timeout=min(5, remaining))
                probe = 'http_response'
                if 200 <= status < 300:
                    self.active = True
                    return
            except (URLError, TimeoutError, OSError):
                probe = 'transport_error'
            remaining = deadline - time.monotonic()
            if remaining > 0:
                time.sleep(min(.5, remaining))

    def _request(self, method, arguments, timeout=30):
        request = Request(self.base_url + '/' + method,
                          data=json.dumps(arguments, ensure_ascii=False).encode('utf-8'),
                          headers={'Content-Type': 'application/json'}, method='POST')
        try:
            with urlopen(request, timeout=timeout) as response:
                if response.status == 204:
                    return response.status, None
                return response.status, json.loads(response.read())
        except HTTPError as error:
            error.close()
            return error.code, {}

    def assert_live(self):
        if not (self.active and self.database and self.database.active and self.process
                and self.process.poll() is None):
            raise RuntimeError('Fixture propia no activa')
        self.artifact.verify()

    def call(self, method, **arguments):
        self.assert_live()
        try:
            status, payload = self._request(method, arguments)
        except ValueError:
            raise RuntimeError('Bridge devolvió error') from None
        if not 200 <= status < 300:
            raise RuntimeError('Bridge devolvió error')
        if not isinstance(payload, dict):
            raise RuntimeError('Bridge devolvió error')
        return payload

    def close(self, *, preserve_database=False):
        self.active = False
        try:
            if self.process:
                self.process.terminate()
                try:
                    self.process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=10)
        finally:
            try:
                if self.output is not None:
                    self.output.close()
            finally:
                if self.database and not preserve_database:
                    self.database.close()


class RejectEmbedder:
    dimensions = 1536

    async def embed(self, text):
        raise RuntimeError('GM-01 prohíbe embeddings')

    async def embed_batch(self, texts):
        raise RuntimeError('GM-01 prohíbe embeddings')


@contextmanager
def reference_environment(private):
    previous = dict(os.environ)
    os.environ.clear()
    os.environ.update(clean_env(), HOME=str(private))
    try:
        yield
    finally:
        os.environ.clear()
        os.environ.update(previous)


def prepare_reference(source, private):
    from reference import materialize_sources, verify_repository

    integrity_path = ROOT / 'dev/muninn/source-integrity.json'
    integrity = json.loads(integrity_path.read_text())
    selected = {'agent-memory': integrity['agent-memory']}
    if selected['agent-memory']['commit'] != PIN:
        raise RuntimeError('Pin de referencia incorrecto')
    verify_repository(source, PIN)
    materialize_sources(source.parent, selected, private / 'reference')
    return private / 'reference/agent-memory/src'


def reference_options(uri, password):
    return {'backend': 'bolt',
            'neo4j': {'uri': uri, 'username': 'neo4j', 'password': password, 'database': 'neo4j'},
            'embedding': {'provider': 'custom', 'dimensions': RejectEmbedder.dimensions},
            'extraction': {'extractor_type': 'none', 'enable_llm_fallback': False}}


def load_reference_api():
    from neo4j_agent_memory import MemoryClient, MemorySettings
    return MemoryClient, MemorySettings


def reserve_evidence(path):
    Path(path).mkdir(parents=True, exist_ok=False, mode=0o700)


def save(path, value):
    with Path(path).open('x', encoding='utf-8') as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False)
        stream.write('\n')


def empty_manifest():
    return {'h1_acceptance': 'not_established', 'status': 'incomplete',
            'reference_commit': PIN, 'stage': 'host_prepare'}


def _write_blocked_manifest(evidence, manifest, error):
    manifest['status'] = 'blocked'
    manifest['error_type'] = type(error).__name__
    path = Path(evidence) / 'manifest.json'
    if path.exists() or path.is_symlink():
        if path.is_symlink() or not path.is_file():
            return
        try:
            captured = json.loads(path.read_text(encoding='utf-8'))
        except (OSError, json.JSONDecodeError):
            return
        if not isinstance(captured, dict):
            return
        captured['h1_acceptance'] = 'not_established'
        captured['status'] = 'blocked'
        captured['orchestration_stage'] = manifest.get('stage', 'host_prepare')
        captured['orchestration_error_type'] = type(error).__name__
        for key in ('capture_image', 'neo4j_image', 'cleanup_status', 'cleanup_error_type',
                    'cleanup_warning', 'compose_diagnostics'):
            if key in manifest:
                captured[key] = manifest[key]
        value = captured
    else:
        value = manifest
    temporary = path.with_name('manifest.json.tmp')
    try:
        with temporary.open('x', encoding='utf-8') as stream:
            json.dump(value, stream, indent=2, ensure_ascii=False)
            stream.write('\n')
        os.replace(temporary, path)
    except OSError:
        try:
            temporary.unlink()
        except OSError:
            pass


async def run(source, evidence):
    """Verify sources, package the Linux image, and delegate all runtime work to Compose."""
    evidence = Path(evidence).resolve()
    reserve_evidence(evidence)
    manifest = empty_manifest()
    compose_path = None
    project = None
    error = None
    try:
        with tempfile.TemporaryDirectory(prefix='muninn-gm01-') as temporary:
            private = Path(temporary)
            private.chmod(0o700)
            with temporary_compose_secrets(ROOT) as secret_directory:
                try:
                    reference_password = secrets.token_hex(24)
                    rust_password = secrets.token_hex(24)
                    reference_secret = secret_directory / 'reference.auth'
                    rust_secret = secret_directory / 'rust.auth'
                    write_auth_file(reference_secret, reference_password)
                    write_auth_file(rust_secret, rust_password)

                    manifest['stage'] = 'prepare_reference'
                    reference_source = prepare_reference(Path(source).resolve(), private)
                    manifest['reference_integrity_sha256'] = digest(
                        ROOT / 'dev/muninn/source-integrity.json')
                    manifest['stage'] = 'stage_build_context'
                    context = stage_build_context(ROOT, reference_source, private / 'context')
                    project = 'muninn-gm01-' + secrets.token_hex(10)
                    compose_path = private / 'compose.yaml'
                    compose_path.write_text(compose_document(
                        context, evidence, reference_secret, rust_secret, project,
                        os.getuid(), os.getgid()), encoding='utf-8')
                    manifest['stage'] = 'compose_config'
                    if compose_run(compose_path, project, 'config', '--quiet') != 0:
                        raise RuntimeError('Configuración Compose inválida')
                    manifest['stage'] = 'inspect_neo4j_image'
                    manifest['neo4j_image'] = inspect_neo4j_image()
                    manifest['stage'] = 'build_image'
                    if compose_run(compose_path, project, 'build', '--pull', 'capture') != 0:
                        raise RuntimeError('Build de imagen fallido')
                    expected_sources = json.loads(
                        (context / 'build-provenance.json').read_text())['sources']
                    if source_hashes(ROOT) != expected_sources:
                        raise RuntimeError('La fuente cambió durante build')
                    manifest['stage'] = 'inspect_capture_image'
                    manifest['capture_image'] = inspect_capture_image(project)
                    manifest['stage'] = 'compose_up'
                    result = compose_run(compose_path, project, 'up', '--pull', 'never',
                                         '--no-build', '--abort-on-container-exit',
                                         '--exit-code-from', 'capture')
                    if result != 0:
                        try:
                            manifest['compose_diagnostics'] = inspect_compose_project(project)
                        except Exception as diagnostic_error:
                            manifest['compose_diagnostics'] = {
                                'diagnostic_error_type': type(diagnostic_error).__name__,
                            }
                        raise RuntimeError('Captura Compose fallida')
                    capture_manifest = evidence / 'manifest.json'
                    if not capture_manifest.is_file():
                        raise RuntimeError('El capturador no escribió manifest.json')
                    manifest['stage'] = 'record_capture_image'
                    merge_capture_image(capture_manifest, manifest['capture_image'],
                                        manifest['neo4j_image'])
                except Exception as caught:
                    error = caught
                    _write_blocked_manifest(evidence, manifest, caught)
                finally:
                    if compose_path is not None and project is not None:
                        try:
                            cleanup_result = compose_run(
                                compose_path, project, 'down', '--volumes',
                                '--remove-orphans')
                            if cleanup_result != 0:
                                raise RuntimeError('Limpieza Compose fallida')
                        except Exception as caught:
                            manifest['cleanup_status'] = 'failed'
                            manifest['cleanup_error_type'] = type(caught).__name__
                            manifest['cleanup_warning'] = (
                                'La limpieza de recursos puede estar incompleta.')
                            if error is None:
                                error = caught
                                manifest['stage'] = 'compose_cleanup'
                            _write_blocked_manifest(evidence, manifest, error)
    except Exception as caught:
        error = caught
        _write_blocked_manifest(evidence, manifest, caught)
    if error is not None:
        raise RuntimeError('GM-01 bloqueado; consultar tipo de error en manifest.json') from None


async def run_container(evidence):
    """Run the capture inside the isolated network; this process owns its manifest."""
    evidence = Path(evidence)
    if not evidence.is_dir() or any(evidence.iterdir()):
        raise RuntimeError('Directorio de evidencia no vacío')
    manifest = empty_manifest()
    fixture = None
    client = None
    try:
        with tempfile.TemporaryDirectory(prefix='muninn-gm01-runtime-') as temporary:
            private = Path(temporary)
            private.chmod(0o700)
            provenance = json.loads(Path('/opt/harness/build-provenance.json').read_text())
            binary = Path('/usr/local/bin/muninn')
            backend = dict(provenance)
            backend['binary_sha256'] = digest(binary)
            backend['binary_format'] = verify_linux_arm64_binary(binary)
            artifact = BuiltArtifact(binary, backend)
            artifact.verify()
            manifest['reference_commit'] = provenance['reference_commit']
            manifest['reference_integrity_sha256'] = provenance['reference_integrity_sha256']
            manifest['backend'] = backend
            manifest['image'] = {'neo4j': NEO4J_IMAGE, 'platform': 'linux/arm64'}

            passwords = {}
            for name in ('reference_auth', 'rust_auth'):
                value = Path('/run/secrets/' + name).read_text(encoding='utf-8').strip()
                if not value.startswith('neo4j/') or len(value) <= len('neo4j/'):
                    raise RuntimeError('Secreto Neo4j no válido')
                passwords[name] = value[len('neo4j/'):]

            rust_db = Database('bolt://rust-db:7687', passwords['rust_auth'])
            reference_db = Database('bolt://reference-db:7687', passwords['reference_auth'])
            with reference_environment(private):
                if any(name == 'neo4j_agent_memory' or name.startswith('neo4j_agent_memory.')
                       for name in sys.modules):
                    raise RuntimeError('Referencia ya importada; usar proceso nuevo')
                sys.path.insert(0, '/opt/reference/src')
                try:
                    manifest['stage'] = 'import_reference'
                    MemoryClient, MemorySettings = load_reference_api()
                    fixture = OwnedFixture(artifact, private, rust_db)
                    manifest['stage'] = 'start_bridge'
                    fixture.start()
                    manifest['stage'] = 'create_reference_client'
                    settings = MemorySettings(**reference_options(reference_db.uri,
                                                                   reference_db.password))
                    client = MemoryClient(settings, embedder=RejectEmbedder())
                    try:
                        manifest['stage'] = 'connect_reference'
                        await client.connect()
                        manifest['stage'] = 'capture_reference'
                        reference_capture = await capture_gm01(
                            ReferenceAdapter(client), ('gm01-a', 'gm01-b'))
                        manifest['stage'] = 'capture_backend'
                        rust_capture = await capture_gm01(
                            RustBridgeAdapter(fixture), ('gm01-a', 'gm01-b'))
                        from gm01_lifecycle import LifecycleAdapter, capture_lifecycle
                        manifest['stage'] = 'capture_lifecycle'
                        lifecycle = await capture_lifecycle(
                            LifecycleAdapter(reference_db, reference=client,
                                             checkpoint=lambda value: manifest.update(lifecycle_operation=value)),
                            LifecycleAdapter(rust_db, fixture=fixture,
                                             checkpoint=lambda value: manifest.update(lifecycle_operation=value)))
                        from gm01_lifecycle import capture_interleaved
                        manifest['stage'] = 'capture_interleaved'
                        interleaved = await capture_interleaved(
                            LifecycleAdapter(reference_db, reference=client,
                                             checkpoint=lambda value: manifest.update(lifecycle_operation=value)),
                            LifecycleAdapter(rust_db, fixture=fixture,
                                             checkpoint=lambda value: manifest.update(lifecycle_operation=value)))
                        lifecycle['cases'].append(interleaved)
                        lifecycle['errors'].extend('interleaved:' + error for error in interleaved['errors'])
                        from gm01_lifecycle import finalize_lifecycle
                        finalize_lifecycle(lifecycle)
                        save(evidence / 'lifecycle.json', lifecycle)
                        manifest['gm01_acceptance'] = lifecycle['gm01_acceptance']
                        manifest['gm01_lifecycle_errors'] = lifecycle['errors']
                        manifest['gm01_pending'] = lifecycle['pending']
                    finally:
                        manifest['cleanup_stage'] = 'close_reference_client'
                        closing_client = client
                        client = None
                        await closing_client.close()
                    manifest['stage'] = 'save_captures'
                    save(evidence / 'reference.json', reference_capture)
                    save(evidence / 'rust.json', rust_capture)
                    manifest['stage'] = 'compare'
                    result = compare(reference_capture, rust_capture)
                    manifest['stage'] = 'save_comparison'
                    save(evidence / 'comparison.json', result)
                    manifest['status'] = 'captured'
                    manifest['comparison'] = result['comparison']
                finally:
                    sys.path.remove('/opt/reference/src')
    except Exception as caught:
        if isinstance(caught, StartupFailure):
            manifest['stage'] = caught.diagnostic['stage']
            manifest['startup_diagnostic'] = caught.diagnostic
        manifest['error_type'] = type(caught).__name__
        manifest['status'] = 'blocked'
    finally:
        cleanup_errors = []
        if client is not None:
            manifest['cleanup_stage'] = 'close_reference_client'
            try:
                await client.close()
            except Exception as caught:
                cleanup_errors.append(caught)
        if fixture is not None:
            manifest['cleanup_stage'] = 'close_bridge'
            try:
                fixture.close()
            except Exception as caught:
                cleanup_errors.append(caught)
        if cleanup_errors:
            manifest['cleanup_error_type'] = type(cleanup_errors[0]).__name__
            manifest['status'] = 'blocked'
        save(evidence / 'manifest.json', manifest)
    if manifest['status'] != 'captured':
        raise RuntimeError('Captura GM-01 bloqueada; consultar manifest.json')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path,
                        help='Checkout agent-memory fijado; se verifican commit y bytes')
    parser.add_argument('--evidence', type=Path, required=True,
                        help='Directorio nuevo; nunca se sobrescribe')
    parser.add_argument('--container', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    if args.container:
        if args.reference is not None:
            parser.error('--reference no es válido en el capturador')
        asyncio.run(run_container(args.evidence.resolve()))
        return
    if args.reference is None:
        parser.error('--reference es obligatorio en el runner host')
    if os.getuid() == 0:
        parser.error('No ejecutar como root')
    asyncio.run(run(args.reference.resolve(), args.evidence.resolve()))


if __name__ == '__main__':
    main()
