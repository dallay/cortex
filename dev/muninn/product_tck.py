"""TCK normativo sobre producto actual hash en Docker aislado; sin inferencia."""
import argparse
import csv
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from product_chain_fixture import ProductFixture
from product_tck_report import validate_profile_result
from product_tck_selector import select_profiles
from reference import verify_source_tree
from rust_tck_spike import (SILVER_SPLIT_PROFILES, load_normative_lock,
                            profile_selector as rust_profile_selector)
from tck_runner import prepare_suite


# El perfil combinado `silver` no se ejecuta aquí: mezclaría H2 (largo plazo) con
# los casos de reasoning de H3 en una sola evidencia.
GOLD_CROSS_MEMORY_PROFILE = 'gold-cross-memory'
GOLD_CROSS_MEMORY_FILE = 'v1/test_cross_memory.py'
GOLD_CROSS_MEMORY_CASES = 18
PROFILES = ('messages', 'chains', 'search', 'schema', 'bronze', *SILVER_SPLIT_PROFILES,
            GOLD_CROSS_MEMORY_PROFILE)


def profile_selector(profile, items, suite=None):
    """Select the pinned cross-memory Gold marker cases without broadening scope."""
    if profile != GOLD_CROSS_MEMORY_PROFILE:
        return rust_profile_selector(profile, items, suite)
    if suite is None:
        raise ValueError('El perfil Gold requiere el source suite fijado')
    selected = {item['nodeid'] for item in items
                if item['nodeid'].startswith(GOLD_CROSS_MEMORY_FILE + '::')
                and 'gold' in item['markers']}
    if len(selected) != GOLD_CROSS_MEMORY_CASES:
        raise ValueError(f'El inventario Gold cross-memory debe contener exactamente {GOLD_CROSS_MEMORY_CASES} IDs')
    if any(not nodeid.startswith(GOLD_CROSS_MEMORY_FILE + '::') for nodeid in selected):
        raise ValueError('El inventario Gold cross-memory contiene IDs fuera del archivo fijado')
    arguments = [str(Path(suite) / nodeid) for nodeid in sorted(selected)]
    return selected, arguments


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--sources', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--profile', action='append', choices=PROFILES, metavar='PROFILE',
                        help='Ejecuta solo el perfil indicado; se puede repetir')
    args = parser.parse_args()
    try:
        profiles = select_profiles(args.profile, PROFILES)
    except ValueError as error:
        parser.error(str(error))
    base = Path(__file__).resolve().parent
    lock = load_normative_lock(base)
    verify_source_tree(args.sources.parent, {args.sources.name: lock})
    args.output.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory() as directory, ProductFixture(args.binary) as fixture:
        suite = Path(directory) / 'suite'
        hashes = prepare_suite(args.sources, suite)
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
'''.replace('URL', repr(fixture.url)))
        inventory = args.output.resolve() / 'collection.json'
        (suite / 'inventory_plugin.py').write_text('''import json
from pathlib import Path
import pytest
@pytest.hookimpl(tryfirst=True)
def pytest_collection_modifyitems(items):
    Path(OUTPUT).write_text(json.dumps([{'nodeid': i.nodeid, 'markers': sorted({m.name for m in i.iter_markers()})} for i in items], indent=2))
'''.replace('OUTPUT', repr(str(inventory))))
        import os
        env = {**os.environ, 'PYTHONPATH': str(args.sources.resolve()) + ':' + str(suite),
               'PYTEST_DISABLE_PLUGIN_AUTOLOAD': '1', 'PYTHONDONTWRITEBYTECODE': '1'}
        env.pop('PYTEST_ADDOPTS', None)
        env.pop('PYTEST_PLUGINS', None)
        command = [sys.executable, '-m', 'pytest', '-c', str(base / 'tck-pytest.ini'),
                   '--rootdir', str(suite), '--import-mode=importlib', '-q', '--tb=short',
                   '-p', 'pytest_asyncio.plugin', '-p', 'pytest_jsonreport.plugin', '-p', 'pytest_metadata.plugin']
        with (args.output / 'collection.log').open('w') as log:
            code = subprocess.run(command + [str(suite / 'v1'), '--collect-only', '-p', 'inventory_plugin'], env=env, stdout=log, stderr=log).returncode
        if code:
            raise RuntimeError('No se pudo coleccionar TCK normativo')
        items = json.loads(inventory.read_text())
        results = {}
        for profile in profiles:
            selected, selector = profile_selector(profile, items, suite)
            report = args.output.resolve() / (profile + '.json')
            with (args.output / (profile + '.log')).open('w') as log:
                code = subprocess.run(command + selector + ['--json-report', '--json-report-file=' + str(report), '--json-report-omit=log,streams'], env=env, stdout=log, stderr=log).returncode
            data = json.loads(report.read_text())
            tests = data.get('tests', [])
            observed = [t.get('nodeid') for t in tests if isinstance(t, dict)]
            validation_error = None
            try:
                validate_profile_result(selected, code, data)
            except ValueError as error:
                validation_error = str(error)
            results[profile] = {'exit_code': code, 'selected': len(selected), 'observed': len(observed),
                                'collection_matches': selected == set(observed), 'summary': data.get('summary', {}),
                                'valid': validation_error is None, 'validation_error': validation_error}
        summary = {'commit': lock['commit'], 'historical_commit': lock['historical_commit'],
                   'binary_sha256': fixture.binary_sha256, 'neo4j_image_id': fixture.image,
                   'provider': 'hash', 'inference': False, 'official_test_hashes': hashes,
                   'collected': len(items), 'bronze': sum('bronze' in i['markers'] for i in items),
                   'silver': sum('silver' in i['markers'] for i in items), 'profiles': results}
        (args.output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        print(json.dumps(summary, indent=2))
        if any(not result['valid'] for result in results.values()):
            raise SystemExit(2)


if __name__ == '__main__':
    main()
