"""Orquesta Bronze en una fixture nueva, internal y sin puertos publicados."""
import argparse
import hashlib
import json
from pathlib import Path
import secrets
import shutil
import subprocess
import tempfile

import schema_runner as fixtures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', required=True)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--gm01-evidence', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    if args.evidence.exists():
        raise ValueError('La evidencia debe ser nueva')
    args.evidence.mkdir(parents=True)
    evidence = args.evidence.resolve()
    base = Path(__file__).resolve().parent
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
            fixture = fixtures.start_fixture('bronze', network, auth, password)
            if not fixture.ready:
                raise RuntimeError('Neo4j efímera no estuvo disponible')
            # Give the newly created fixture a DNS name only within this network.
            fixtures.docker('network', 'disconnect', network, fixture.container)
            result = fixtures.docker('network', 'connect', '--alias', 'neo4j', network, fixture.container)
            if result.returncode:
                raise RuntimeError('No se configuró el alias privado')
            command = ['docker', 'run', '--rm', '--network', network, '--user', '1000:1000',
                       '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true',
                       '--mount', f'type=bind,source={base},target=/harness,readonly',
                       '--mount', f'type=bind,source={args.sources.resolve()},target=/sources/tck,readonly',
                       '--mount', f'type=bind,source={auth.resolve()},target=/run/secrets/neo4j_auth,readonly',
                       '--mount', f'type=bind,source={evidence},target=/evidence',
                       '--env', 'PYTHONPATH=/harness', '--env', 'PYTHONDONTWRITEBYTECODE=1',
                       args.image, '--sources', '/sources/tck', '--output', '/evidence/results']
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
