#!/usr/bin/env python3
"""Run the Muninn non-vector schema scenarios against disposable Neo4j fixtures.

Every scenario gets its own NEW Neo4j container with ephemeral tmpfs storage, so
no scenario can observe another scenario's schema or data. The stack runs on an
internal-only Docker network: no ports are published and no fixture is reachable
from the host or the internet.

The Rust integration test is the subject under test. This runner only provides
fixtures, isolation and evidence capture.
"""

from __future__ import annotations

import argparse
import json
import os
import secrets
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CACHE = ROOT / 'dev/muninn/.cache'
RUST_IMAGE = 'rust:1.99.1-slim@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247'
NEO4J_IMAGE = 'neo4j:5.26-community'
TARGET_VOLUME = 'dallay630-linux-target'
TEST_BINARY_PREFIX = 'debug/deps/schema_non_vector-'
TEST_NAME = 'non_vector_schema_preserves_data_and_rejects_incompatible_objects'
SCENARIOS = ('clean', 'constraint', 'composite', 'range', 'text')


def discover_test_binary() -> str:
    """Locate the compiled integration test binary inside the target volume."""
    listing = docker(
        'run', '--rm',
        '--volume', f'{TARGET_VOLUME}:/target:ro',
        RUST_IMAGE,
        'sh', '-c', f'ls /target/{TEST_BINARY_PREFIX}* 2>/dev/null | grep -v "\\.d$" | head -1',
    )
    candidate = listing.stdout.strip()
    if not candidate:
        raise RuntimeError('the compiled schema_non_vector test binary is not in the target volume')
    return candidate.removeprefix('/target/')


def run(command: list[str], *, capture: bool = True, check: bool = False) -> subprocess.CompletedProcess:
    return subprocess.run(
        command,
        cwd=ROOT,
        capture_output=capture,
        text=True,
        check=check,
    )


def docker(*arguments: str, capture: bool = True) -> subprocess.CompletedProcess:
    return run(['docker', *arguments], capture=capture)


@dataclass
class Fixture:
    """One disposable Neo4j container bound to a single scenario."""

    scenario: str
    container: str
    password: str
    network: str = ''
    ready: bool = False
    error: str = ''
    started_at: str = ''
    stopped: bool = False


@dataclass
class Evidence:
    """Machine-readable record of one runner execution."""

    network: str = ''
    scenarios: list[dict] = field(default_factory=list)
    artifacts: dict = field(default_factory=dict)
    cleanup: list[str] = field(default_factory=list)
    cleanup_errors: list[str] = field(default_factory=list)

    def to_json(self) -> str:
        return json.dumps(self.__dict__, indent=2, sort_keys=True) + '\n'


def relative_to_root(path: Path) -> str:
    """Report evidence paths relative to the checkout when possible."""
    try:
        return str(path.resolve().relative_to(ROOT))
    except ValueError:
        return str(path)


def short_id() -> str:
    return secrets.token_hex(4)


def wait_for_bolt(container: str, password: str, timeout: float = 150.0) -> tuple[bool, str]:
    """Wait until the fixture accepts Bolt connections.

    Returns the readiness state and the diagnostic reason when it fails.
    """
    deadline = time.monotonic() + timeout
    last_error = ''
    while time.monotonic() < deadline:
        state = docker('inspect', '--format', '{{.State.Status}} {{.State.ExitCode}} {{.State.OOMKilled}}', container)
        if state.returncode != 0:
            return False, f'fixture container disappeared: {state.stderr.strip()}'
        status = state.stdout.strip()
        if status.startswith('exited') or status.startswith('dead'):
            logs = docker('logs', '--tail', '30', container)
            return False, f'fixture container stopped ({status}):\n{logs.stdout.strip()}\n{logs.stderr.strip()}'
        result = docker(
            'exec', container,
            'cypher-shell', '-u', 'neo4j', '-p', password,
            'RETURN 1',
        )
        if result.returncode == 0:
            return True, ''
        last_error = (result.stderr or result.stdout).strip()[-500:]
        time.sleep(3)
    return False, f'fixture did not accept Bolt connections in time: {last_error}'


def write_auth_file(directory: Path, password: str, scenario: str) -> Path:
    """Write the Neo4j auth file used by the fixture (ephemeral, read-only)."""
    path = directory / f'neo4j_auth-{scenario}'
    path.write_text(f'neo4j/{password}\n', encoding='utf-8')
    path.chmod(0o444)
    return path


def start_fixture(scenario: str, network: str, auth_file: Path, password: str) -> Fixture:
    """Start one NEW Neo4j container with ephemeral storage."""
    container = f'muninn-schema-{scenario}-{short_id()}'
    fixture = Fixture(scenario=scenario, container=container, password=password)
    fixture.started_at = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    result = docker(
        'run', '--detach', '--rm',
        '--name', container,
        '--network', network,
        '--user', '7474:7474',
        '--cap-drop', 'ALL',
        '--security-opt', 'no-new-privileges:true',
        '--memory', '2g',
        '--tmpfs', '/data:uid=7474,gid=7474,mode=0700',
        '--tmpfs', '/logs:uid=7474,gid=7474,mode=0700',
        '--mount', f'type=bind,source={auth_file},target=/run/secrets/neo4j_auth,readonly',
        '--env', 'NEO4J_AUTH_FILE=/run/secrets/neo4j_auth',
        '--env', 'NEO4J_server_memory_heap_initial__size=256m',
        '--env', 'NEO4J_server_memory_heap_max__size=512m',
        '--env', 'NEO4J_server_memory_pagecache_size=256m',
        NEO4J_IMAGE,
    )
    if result.returncode != 0:
        fixture.error = f'container start failed: {result.stderr.strip()}'
        return fixture
    os.environ['MUNINN_SCHEMA_PROBE_PASSWORD'] = password
    fixture.ready, fixture.error = wait_for_bolt(container, password)
    return fixture


def stop_fixture(fixture: Fixture, evidence: Evidence) -> None:
    if fixture.stopped:
        return
    docker('rm', '--force', fixture.container)
    fixture.stopped = True
    evidence.cleanup.append(fixture.container)


def run_scenario(fixture: Fixture, evidence_dir: Path, evidence: Evidence, test_binary: str, suffix: str = '') -> dict:
    """Execute the Rust integration test against the fixture."""
    record: dict = {
        'scenario': fixture.scenario,
        'container': fixture.container,
        'started_at': fixture.started_at,
        'fixture_ready': fixture.ready,
    }
    log_path = evidence_dir / f'scenario-{fixture.scenario}{suffix}.log'
    if not fixture.ready:
        record['status'] = 'BLOCKED'
        record['error'] = fixture.error
        log_path.write_text(fixture.error + '\n', encoding='utf-8')
        evidence.scenarios.append(record)
        return record

    command = [
        'docker', 'run', '--rm',
        '--network', fixture.network,
        '--env', 'MUNINN_SCHEMA_ISOLATED_TEST=1',
        '--env', f'MUNINN_SCHEMA_SCENARIO={fixture.scenario}',
        '--env', f'MUNINN_SCHEMA_TEST_URI=bolt://{fixture.container}:7687',
        '--env', f'MUNINN_SCHEMA_TEST_PASSWORD={fixture.password}',
        '--volume', f'{TARGET_VOLUME}:/target:ro',
        RUST_IMAGE,
        f'/target/{test_binary}',
        '--ignored', '--exact', TEST_NAME, '--nocapture',
    ]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    log_path.write_text(result.stdout + '\n' + result.stderr, encoding='utf-8')
    record['exit_code'] = result.returncode
    record['status'] = 'PASS' if result.returncode == 0 else 'FAIL'
    record['log'] = relative_to_root(log_path)
    if result.returncode != 0:
        record['error'] = (result.stderr.strip() or result.stdout.strip())[-2000:]
    evidence.scenarios.append(record)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', default=None, help='Directory for scenario logs and summary')
    parser.add_argument('--scenarios', default=','.join(SCENARIOS))
    parser.add_argument('--keep-network', action='store_true')
    arguments = parser.parse_args()

    if shutil.which('docker') is None:
        print('docker is required', file=sys.stderr)
        return 2

    scenarios = [item.strip() for item in arguments.scenarios.split(',') if item.strip()]
    unknown = [item for item in scenarios if item not in SCENARIOS]
    if unknown:
        print(f'unknown scenarios: {unknown}', file=sys.stderr)
        return 2

    stamp = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())
    evidence_dir = Path(arguments.evidence).resolve() if arguments.evidence else ROOT / f'.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/schema-isolated-{stamp}'
    evidence_dir.mkdir(parents=True, exist_ok=True)

    network = f'muninn-schema-{stamp}-{short_id()}-internal'
    created = docker('network', 'create', '--internal', network)
    if created.returncode != 0:
        print(f'cannot create internal network: {created.stderr.strip()}', file=sys.stderr)
        return 2

    evidence = Evidence(network=network)
    secret_dir = CACHE / f'schema-{stamp}-{short_id()}'
    secret_dir.mkdir(parents=True, exist_ok=True)
    secret_dir.chmod(0o700)

    try:
        test_binary = discover_test_binary()
    except RuntimeError as error:
        print(str(error), file=sys.stderr)
        docker('network', 'rm', network)
        return 2

    try:
        for scenario in scenarios:
            password = secrets.token_urlsafe(24)
            auth_file = write_auth_file(secret_dir, password, scenario)
            fixture = start_fixture(scenario, network, auth_file, password)
            fixture.network = network
            try:
                record = run_scenario(fixture, evidence_dir, evidence, test_binary)
            finally:
                stop_fixture(fixture, evidence)
            if record['status'] == 'BLOCKED':
                # A fixture that never started is an environment problem, not a
                # schema result. Retry once with a fresh disposable fixture.
                retry_password = secrets.token_urlsafe(24)
                retry_auth = write_auth_file(secret_dir, retry_password, f'{scenario}-retry')
                retry = start_fixture(fcenario, network, retry_auth, retry_password)
                retry.network = network
                try:
                    retry_record = run_scenario(retry, evidence_dir, evidence, test_binary, suffix='-retry')
                finally:
                    stop_fixture(retry, evidence)
                if retry_record['status'] != 'BLOCKED':
                    retry_record['retried_after_block'] = record['error']
                    evidence.scenarios[-1] = retry_record
    finally:
        if not arguments.keep_network:
            removed = docker('network', 'rm', network)
            if removed.returncode != 0:
                evidence.cleanup_errors.append(removed.stderr.strip())
            else:
                evidence.cleanup.append(network)
        shutil.rmtree(secret_dir, ignore_errors=True)

    summary_path = evidence_dir / 'summary.json'
    summary_path.write_text(evidence.to_json(), encoding='utf-8')
    evidence.artifacts['summary'] = relative_to_root(summary_path)

    failed = [item for item in evidence.scenarios if item['status'] != 'PASS']
    print(f'evidence: {summary_path}')
    for item in evidence.scenarios:
        print(f"{item['scenario']}: {item['status']}")
    if evidence.cleanup_errors:
        print(f'cleanup errors: {evidence.cleanup_errors}')
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
