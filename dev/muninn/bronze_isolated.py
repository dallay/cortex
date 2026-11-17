"""Orquesta Bronze en una fixture nueva, internal y sin puertos publicados."""
import argparse
import hashlib
import json
from pathlib import Path
import secrets
import subprocess
import tempfile

import schema_runner as fixtures


class MountPathError(ValueError):
    """Una ruta con coma no se puede interpolar en un `--mount` de Docker."""


def resolve_mount_path(path: Path, label: str) -> Path:
    """Resuelve la ruta y rechaza las comas que rompen el parseo de `--mount`.

    Docker separa el valor de `--mount` por comas, así que una ruta con coma
    inyectaría opciones adicionales en la invocación. El runner falla aquí,
    antes de invocar Docker.
    """
    resolved = Path(path).resolve()
    if ',' in str(resolved):
        raise MountPathError(
            f'La ruta de {label} contiene una coma y no puede interpolarse en un '
            f'--mount de Docker: {resolved}')
    return resolved


def mount_option(source: Path, target: str, *, readonly: bool = True) -> list[str]:
    """Construye la opción `--mount` de Docker desde una ruta ya validada."""
    value = f'type=bind,source={source},target={target}'
    if readonly:
        value += ',readonly'
    return ['--mount', value]


def build_run_command(*, image: str, network: str, base: Path, sources: Path,
                      auth: Path, evidence: Path) -> list[str]:
    """Arma la lista de argumentos de `docker run` para el harness Bronze.

    Toda ruta interpolada se valida aquí: ninguna ruta ambigua llega a Docker.
    """
    mounts = [
        (resolve_mount_path(base, 'el harness'), '/harness', True),
        (resolve_mount_path(sources, 'las fuentes TCK'), '/sources/tck', True),
        (resolve_mount_path(auth, 'el secreto de autenticación'),
         '/run/secrets/neo4j_auth', True),
        (resolve_mount_path(evidence, 'la evidencia'), '/evidence', False),
    ]
    command = ['docker', 'run', '--rm', '--network', network, '--user', '1000:1000',
               '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true']
    for source, target, readonly in mounts:
        command += mount_option(source, target, readonly=readonly)
    command += ['--env', 'PYTHONPATH=/harness', '--env', 'PYTHONDONTWRITEBYTECODE=1',
                image, '--sources', '/sources/tck', '--output', '/evidence/results']
    return command


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--gm01-evidence', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if args.evidence.exists():
        raise ValueError('La evidencia debe ser nueva')
    evidence = resolve_mount_path(args.evidence, 'la evidencia')
    sources = resolve_mount_path(args.sources, 'las fuentes TCK')
    base = resolve_mount_path(Path(__file__).resolve().parent, 'el harness')
    args.evidence.mkdir(parents=True)
    manifest = json.loads((args.gm01_evidence / 'manifest.json').read_text())
    receipt = {'status': 'blocked', 'gm01_manifest_sha256': hashlib.sha256(
        (args.gm01_evidence / 'manifest.json').read_bytes()).hexdigest(),
        'product_materials': manifest['backend']}
    network = 'muninn-bronze-' + secrets.token_hex(6)
    fixture = None
    cleanup = fixtures.Evidence(network=network)
    created = fixtures.docker('network', 'create', '--internal', network)
    if created.returncode:
        raise RuntimeError('No se creó la red interna')
    try:
        with tempfile.TemporaryDirectory(prefix='bronze-', dir=fixtures.CACHE) as directory:
            private = Path(directory)
            password = secrets.token_urlsafe(24)
            auth = fixtures.write_auth_file(private, password, 'bronze')
            auth = resolve_mount_path(auth, 'el secreto de autenticación')
            fixture = fixtures.start_fixture('bronze', network, auth, password)
            if not fixture.ready:
                raise RuntimeError('Neo4j efímera no estuvo disponible')
            # Give the newly created fixture a DNS name only within this network.
            fixtures.docker('network', 'disconnect', network, fixture.container)
            result = fixtures.docker('network', 'connect', '--alias', 'neo4j', network, fixture.container)
            if result.returncode:
                raise RuntimeError('No se configuró el alias privado')
            command = build_run_command(image=args.image, network=network, base=base,
                                        sources=sources, auth=auth, evidence=evidence)
            with (evidence / 'container.log').open('w') as out:
                code = subprocess.run(command, stdout=out, stderr=out).returncode
            receipt['image_id'] = fixtures.docker('image', 'inspect', args.image, '--format', '{{.Id}}').stdout.strip()
            if code:
                raise RuntimeError('Bronze falló; consultar logs sintéticos')
            summary = json.loads((evidence / 'results/summary.json').read_text())
            expected_binary = manifest['backend'].get('rust_binary_sha256')
            if expected_binary is None:
                expected_binary = manifest['backend'].get('binary_sha256')
            if expected_binary is None or summary['binary_sha256'] != expected_binary:
                raise RuntimeError('Identidad del binario no conciliada con GM-01')
            receipt.update(status='pass', bronze=summary)
    finally:
        if fixture is not None:
            fixtures.stop_fixture(fixture, cleanup)
        removal = fixtures.docker('network', 'rm', network)
        receipt['cleanup_status'] = 'pass' if removal.returncode == 0 else 'failed'
        (evidence / 'manifest.json').write_text(json.dumps(receipt, indent=2) + '\n')


if __name__ == '__main__':
    main()
