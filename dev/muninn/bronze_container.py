"""Ejecuta Bronze dentro de una red internal y una base efímera del runner.

No crea contenedores ni acepta conexiones a bases externas por sí mismo.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import URLError
from urllib.request import Request, urlopen

from bronze_receipt import validate_bronze
from reference import verify_source_tree
from tck_runner import prepare_suite


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    lock = json.loads(Path('/harness/reference.lock.yaml').read_text())
    verify_source_tree(args.sources.parent, {args.sources.name: lock})
    args.output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='muninn-bronze-') as directory:
        private = Path(directory)
        suite = private / 'suite'
        hashes = prepare_suite(args.sources, suite)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        url = f'http://127.0.0.1:{port}'
        password = Path('/run/secrets/neo4j_auth').read_text().strip().split('/', 1)[1]
        env = {**os.environ, 'NAM_NEO4J__URI': 'bolt://neo4j:7687',
               'NAM_NEO4J__PASSWORD': password, 'NAM_EMBEDDING__PROVIDER': 'hash'}
        with (private / 'bridge.log').open('w') as log:
            process = subprocess.Popen(['/usr/local/bin/muninn', 'bridge', '--host', '127.0.0.1',
                                        '--port', str(port), '--allow-destructive-clear'],
                                       env=env, stdout=log, stderr=log)
            try:
                for _ in range(120):
                    if process.poll() is not None:
                        raise RuntimeError('Bridge no inició; diagnóstico privado no publicado')
                    try:
                        request = Request(url + '/setup', data=b'{}', headers={'Content-Type': 'application/json'})
                        with urlopen(request, timeout=1) as response:
                            if response.status == 200:
                                break
                    except (URLError, TimeoutError):
                        time.sleep(.5)
                else:
                    raise RuntimeError('Bridge no estuvo listo')
                (suite / 'v1/conftest.py').write_text('''import pytest_asyncio
from tck.adapters.http_bridge import HTTPBridgeAdapter
@pytest_asyncio.fixture
async def adapter():
    adapter = HTTPBridgeAdapter(base_url=URL)
    await adapter.setup()
    await adapter.clear_all_data()
    try:
        yield adapter
    finally:
        await adapter.clear_all_data()
        await adapter.teardown()
'''.replace('URL', repr(url)))
                inventory = args.output / 'collection.json'
                (suite / 'inventory_plugin.py').write_text('''import json
from pathlib import Path
import pytest
@pytest.hookimpl(tryfirst=True)
def pytest_collection_modifyitems(items):
    Path(OUTPUT).write_text(json.dumps([{'nodeid': i.nodeid, 'markers': sorted({m.name for m in i.iter_markers()})} for i in items], indent=2))
'''.replace('OUTPUT', repr(str(inventory))))
                test_env = {**os.environ, 'PYTHONPATH': str(args.sources) + ':' + str(suite),
                            'PYTEST_DISABLE_PLUGIN_AUTOLOAD': '1', 'PYTHONDONTWRITEBYTECODE': '1'}
                for key in ('PYTEST_ADDOPTS', 'PYTEST_PLUGINS'):
                    test_env.pop(key, None)
                command = [sys.executable, '-m', 'pytest', str(suite / 'v1'), '-c', '/harness/tck-pytest.ini',
                           '--rootdir', str(suite), '--import-mode=importlib', '-q', '--tb=short',
                           '-p', 'pytest_asyncio.plugin', '-p', 'pytest_jsonreport.plugin', '-p', 'pytest_metadata.plugin']
                with (args.output / 'collection.log').open('w') as out:
                    code = subprocess.run(command + ['--collect-only', '-p', 'inventory_plugin'],
                                          env=test_env, stdout=out, stderr=out).returncode
                if code:
                    raise RuntimeError('Colección normativa fallida')
                report = args.output / 'pytest-report.json'
                with (args.output / 'bronze.log').open('w') as out:
                    code = subprocess.run(command + ['-m', 'bronze', '--json-report',
                                          f'--json-report-file={report}', '--json-report-omit=log,streams'],
                                          env=test_env, stdout=out, stderr=out).returncode
                receipt = validate_bronze(json.loads(inventory.read_text()), json.loads(report.read_text()), code)
                receipt.update(status='pass', tck_commit=lock['commit'], test_hashes=hashes,
                               binary_sha256=hashlib.sha256(Path('/usr/local/bin/muninn').read_bytes()).hexdigest())
                (args.output / 'summary.json').write_text(json.dumps(receipt, indent=2) + '\n')
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)


if __name__ == '__main__':
    main()
