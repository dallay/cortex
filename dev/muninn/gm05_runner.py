"""Ejecuta GM-05 contra el oráculo fijado y contra Muninn en Neo4j efímeras.

Cada lado usa su propia base desechable y su propio intérprete. El runner no
decide el veredicto: lo emite el contrato a partir de las dos capturas.

El runner publica `summary.json` en toda ruta, también cuando una captura falla:
un fallo del arnés no puede quedar sin veredicto. Si la captura de la referencia
falla, deja además `reference-failure.json` con el código de salida y la cola del
registro. Las dependencias del entorno de ejecución (`httpx`, `neo4j` y el
fixture del producto) se importan donde se usan para que el módulo sea
importable sin ellas.
"""

import argparse
import asyncio
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import tempfile
import time

from gm05_adapters import BridgeAdapter, GraphReader
from gm05_build import build_candidate
from gm05_capture import capture_gm05
from gm05_contract import compare_gm05
from reference import verify_source_tree

NEO4J_IMAGE = ('neo4j:5.26-community@sha256:'
               '414adfe41c6ff32abb752be37f265c77607c166cab298f65a917fc4f05776218')
CONTAINER_ID = re.compile(r'[0-9a-f]{12,64}')
# Desviaciones autorizadas explícitamente; ninguna otra se perdona.
DEVIATIONS = ('GM05-DEV-001',)
STARTUP_SECONDS = 120
LOG_TAIL_LINES = 40
# Una línea del registro que pueda llevar una credencial no sale del artefacto.
CREDENTIAL = re.compile(r'(?i)(password|secret|auth|token)')


class ReferenceCaptureFailure(RuntimeError):
    """La captura del oráculo falló; el runner publica veredicto y diagnóstico."""

    def __init__(self, message, exit_code=None, log=None):
        super().__init__(message)
        self.exit_code = exit_code
        self.log = Path(log) if log is not None else None


def _log_tail(path, limit=LOG_TAIL_LINES):
    """Últimas líneas del registro, sin líneas que puedan llevar credenciales."""
    if path is None:
        return []
    try:
        lines = Path(path).read_text(encoding='utf-8', errors='replace').splitlines()
    except OSError:
        return []
    return [line for line in lines[-limit:] if not CREDENTIAL.search(line)]


def failure_summary(stage, error):
    """Resumen de un fallo con la misma forma que un veredicto del contrato."""
    return {
        'scenario': 'gm05-v1',
        'failed_stage': stage,
        'verdict': {'comparison': 'fail', 'deviations': [],
                    'errors': [f'error: {type(error).__name__}: {error}']},
        'inference': False,
    }


def write_summary(output, summary):
    """Publica `summary.json`; se llama en toda ruta, también cuando hay fallo."""
    path = Path(output) / 'summary.json'
    path.write_text(json.dumps(summary, ensure_ascii=False, indent=2, default=str) + '\n',
                    encoding='utf-8')
    return path


def publish_reference_failure(output, error):
    """Deja el artefacto diagnóstico de una captura de referencia fallida."""
    diagnostic = {
        'stage': 'reference-capture',
        'error': f'{type(error).__name__}: {error}',
        'exit_code': getattr(error, 'exit_code', None),
        'log': Path(error.log).name if getattr(error, 'log', None) else None,
        'log_tail': _log_tail(getattr(error, 'log', None)),
    }
    path = Path(output) / 'reference-failure.json'
    path.write_text(json.dumps(diagnostic, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    return path


class Neo4jFixture:
    """Base desechable propia; nunca se conecta a una base del operador."""

    def __init__(self):
        self.password = secrets.token_urlsafe(30)
        self.container = None
        self.driver = None

    def __enter__(self):
        from neo4j import GraphDatabase
        with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8') as auth:
            auth.write('NEO4J_AUTH=neo4j/' + self.password + '\n')
            auth.flush()
            result = subprocess.run(
                ['docker', 'run', '-d', '--rm', '--publish', '127.0.0.1::7687',
                 '--env-file', auth.name, NEO4J_IMAGE],
                capture_output=True, text=True, check=False)
        if result.returncode:
            raise RuntimeError('No se pudo crear la base efímera')
        container = result.stdout.strip()
        if not CONTAINER_ID.fullmatch(container):
            raise RuntimeError('Docker devolvió un identificador inválido')
        self.container = container
        port = subprocess.check_output(
            ['docker', 'port', self.container, '7687/tcp'], text=True).strip().rsplit(':', 1)[1]
        self.uri = f'bolt://127.0.0.1:{port}'
        self.driver = GraphDatabase.driver(self.uri, auth=('neo4j', self.password))
        for _ in range(STARTUP_SECONDS * 2):
            try:
                self.driver.verify_connectivity()
                break
            except Exception:
                time.sleep(.5)
        else:
            self.__exit__(None, None, None)
            raise RuntimeError('La base efímera no está disponible')
        if self.query('MATCH (n) RETURN count(n) AS count')[0]['count'] != 0:
            self.__exit__(None, None, None)
            raise RuntimeError('La base recién creada no está vacía')
        self.image = subprocess.check_output(
            ['docker', 'inspect', '--format', '{{.Image}}', self.container], text=True).strip()
        return self

    def query(self, cypher, parameters=None):
        with self.driver.session() as session:
            return session.run(cypher, **(parameters or {})).data()

    def __exit__(self, *_):
        errors = []
        if self.driver is not None:
            driver, self.driver = self.driver, None
            driver.close()
        if self.container is not None:
            container, self.container = self.container, None
            try:
                subprocess.run(['docker', 'rm', '-f', container], capture_output=True, check=True)
            except subprocess.CalledProcessError as error:
                errors.append(error)
        if errors:
            raise RuntimeError('Falló la limpieza de la base efímera')
        return False


def _bridge_post(url):
    async def post(operation, payload):
        import httpx
        async with httpx.AsyncClient() as client:
            response = await client.post(f'{url}/{operation}', json=payload, timeout=30)
        if response.status_code == 404:
            raise LookupError(operation)
        if not response.is_success:
            raise RuntimeError(f'{operation}: HTTP {response.status_code}')
        return response.json() if response.content else None
    return post


def _file_hashes(base):
    names = ('gm05_contract.py', 'gm05_capture.py', 'gm05_adapters.py',
             'gm05_reference_capture.py', 'gm05_runner.py', 'gm05_build.py',
             'gm01_runner.py', 'product_chain_fixture.py')
    return {name: hashlib.sha256((base / name).read_bytes()).hexdigest() for name in names}


def run_capture(args, base, output, artifact):
    """Ejecuta las dos capturas y devuelve el resumen que el contrato emite.

    Un fallo de la captura de la referencia se señala con
    `ReferenceCaptureFailure`; cualquier otro fallo propaga su excepción. En ambos
    casos `main` publica el veredicto.
    """
    integrity = json.loads((base / 'source-integrity.json').read_text(encoding='utf-8'))
    references = json.loads((base / 'references.json').read_text(encoding='utf-8'))
    verify_source_tree(args.sources, {name: integrity[name]
                                      for name in ('agent-memory', 'agent-memory-tck')})
    window_start = datetime.now(timezone.utc)
    reference_output = Path(output) / 'reference-capture.json'
    environment = {**os.environ, 'GM05_SOURCES': str(args.sources.resolve()),
                   'GM05_OUTPUT': str(reference_output),
                   'PYTHONDONTWRITEBYTECODE': '1',
                   'PYTHONPATH': str(base)}
    with tempfile.TemporaryDirectory(prefix='gm05-') as work:
        log = Path(output) / 'reference-capture.log'
        try:
            with Neo4jFixture() as reference_db:
                reference_image = reference_db.image
                environment.update({'GM05_URI': reference_db.uri,
                                    'GM05_PASSWORD': reference_db.password})
                with log.open('w') as handle:
                    code = subprocess.run(
                        [str(args.reference_python),
                         str(base / 'gm05_reference_capture.py')],
                        env=environment, stdout=handle, stderr=handle).returncode
        except ReferenceCaptureFailure:
            raise
        except RuntimeError as error:
            raise ReferenceCaptureFailure(
                f'no se pudo preparar la base del oráculo: {error}',
                log=log) from error
        if code or not reference_output.exists():
            raise ReferenceCaptureFailure('la captura del oráculo falló',
                                          exit_code=code, log=log)
        reference = json.loads(reference_output.read_text(encoding='utf-8'))['capture']
        from product_chain_fixture import ProductFixture
        artifact.verify()
        fixture = ProductFixture(artifact.path)
        if fixture.binary_sha256 != artifact.manifest['binary_sha256']:
            raise RuntimeError('Fixture executable hash differs from the build')
        with fixture as muninn:
            artifact.verify()
            muninn_adapter = BridgeAdapter(
                _bridge_post(muninn.url),
                GraphReader(lambda cypher, params: muninn.query(cypher, **params)))
            muninn_capture = asyncio.run(capture_gm05(muninn_adapter))
            binary_sha256, muninn_image = muninn.binary_sha256, muninn.image
        window_end = datetime.now(timezone.utc)
        verdict = compare_gm05(reference, muninn_capture, deviations=DEVIATIONS,
                               window=(window_start, window_end))
        summary = {
            'scenario': 'gm05-v1',
            'reference_commit': references['agent-memory']['commit'],
            'tck_commit': references['agent-memory-tck']['commit'],
            'binary_sha256': binary_sha256,
            'muninn_neo4j_image': muninn_image,
            'reference_neo4j_image': reference_image,
            'verdict': verdict,
            'reference_gaps': reference.get('gaps', []),
            'muninn_gaps': muninn_capture.get('gaps', []),
            'harness_sha256': _file_hashes(base),
            'inference': False,
            'deviation_notes': {
                'GM05-DEV-001': ('el oráculo acepta `attributes` en add_relationship y no los '
                                 'persiste; Muninn los guarda y el contrato retira ese dato '
                                 'idéntico de la comparación'),
            },
            'reference_embedder': 'tck MockEmbedder (SHA256, 1536)',
            'muninn_embedder': 'hash',
        }
        (Path(output) / 'muninn-capture.json').write_text(
            json.dumps(muninn_capture, ensure_ascii=False, indent=2, default=str) + '\n')
        return summary, muninn_capture


def main(argv=None):
    """Ejecuta la unidad y publica siempre `summary.json`, con veredicto o fallo."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help=argparse.SUPPRESS)
    parser.epilog = ('Builds current workspace bytes in a temporary isolated source copy '
                     'with cargo build --locked --release -p muninn --bin muninn. '
                     'The system toolchain and dependencies are trusted; not hermetic. '
                     '--binary is no longer accepted.')
    parser.add_argument('--sources', type=Path, required=True,
                        help='Directorio con agent-memory y agent-memory-tck verificados')
    parser.add_argument('--reference-python', type=Path, required=True,
                        help='Intérprete con requirements-oracle.txt instalado por hash')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args(argv)
    if args.binary is not None:
        parser.error('--binary is no longer accepted; GM-05 builds the current workspace')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    muninn_capture = {}
    stage = 'build'
    provenance = None
    try:
        base = Path(__file__).resolve().parent
        with build_candidate(base.parents[1]) as artifact:
            provenance = artifact.manifest
            artifact.verify()
            stage = 'capture'
            summary, muninn_capture = run_capture(args, base, output, artifact)
    except ReferenceCaptureFailure as error:
        publish_reference_failure(output, error)
        summary = failure_summary('reference-capture', error)
    except Exception as error:  # noqa: BLE001 - el runner publica el veredicto siempre
        summary = failure_summary(stage, error)
    if provenance is not None:
        summary['build'] = provenance
        summary['binary_sha256'] = provenance['binary_sha256']
    write_summary(output, summary)
    verdict = summary['verdict']
    print(json.dumps({'verdict': verdict['comparison'],
                      'errors': len(verdict['errors']),
                      'gaps': sorted(set(muninn_capture.get('gaps', [])))[:10]},
                     ensure_ascii=False))
    return 0 if verdict['comparison'] == 'pass' else 2


if __name__ == '__main__':
    raise SystemExit(main())
