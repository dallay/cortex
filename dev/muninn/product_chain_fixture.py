"""Fixture desechable del producto actual; nunca conecta a una base del operador."""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import socket
import subprocess
import tempfile
import time

import httpx
from neo4j import GraphDatabase


class ProductFixture:
    def __init__(self, binary):
        self.binary = Path(binary).resolve()
        self.binary_sha256 = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.container = None
        self.process = None
        self.driver = None

    def start_bridge(self):
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        self.url = f'http://127.0.0.1:{port}'
        env = {**os.environ, 'NAM_NEO4J__URI': self.uri,
               'NAM_NEO4J__PASSWORD': self.password, 'NAM_EMBEDDING__PROVIDER': 'hash'}
        self.process = subprocess.Popen([str(self.binary), 'bridge', '--host', '127.0.0.1', '--port', str(port),
                                         '--allow-destructive-clear'], env=env,
                                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(120):
            if self.process.poll() is not None:
                raise RuntimeError('El producto no pudo iniciar')
            try:
                if httpx.post(self.url + '/setup', json={}, timeout=1).is_success:
                    return
            except httpx.HTTPError:
                pass
            time.sleep(.5)
        raise RuntimeError('Tiempo de inicio agotado')

    def stop_bridge(self):
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=10)
            self.process = None

    def call(self, method, **arguments):
        response = httpx.post(self.url + '/' + method, json=arguments, timeout=30)
        if not response.is_success:
            raise RuntimeError(f'{method}: HTTP {response.status_code}')
        return response.json() if response.content else None

    def query(self, cypher, **parameters):
        with self.driver.session() as session:
            return session.run(cypher, **parameters).data()

    def __enter__(self):
        self.password = secrets.token_urlsafe(30)
        try:
            # Docker receives authentication through a private temporary env file, not argv/logs.
            with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8') as auth:
                auth.write('NEO4J_AUTH=neo4j/' + self.password + '\n')
                auth.flush()
                result = subprocess.run(['docker', 'run', '-d', '--rm', '--publish', '127.0.0.1::7687',
                                         '--env-file', auth.name, 'neo4j:5.26-community'],
                                        capture_output=True, text=True, check=False)
            if result.returncode:
                raise RuntimeError('No se pudo crear la fixture Docker aislada')
            container = result.stdout.strip()
            if not re.fullmatch(r'[0-9a-f]{12,64}', container):
                raise RuntimeError('Docker devolvió un identificador de fixture inválido')
            self.container = container
            port = subprocess.check_output(['docker', 'port', self.container, '7687/tcp'], text=True).strip().rsplit(':', 1)[1]
            self.uri = f'bolt://127.0.0.1:{port}'
            self.driver = GraphDatabase.driver(self.uri, auth=('neo4j', self.password))
            for _ in range(120):
                try:
                    self.driver.verify_connectivity()
                    break
                except Exception:
                    time.sleep(.5)
            else:
                raise RuntimeError('Neo4j aislado no está disponible')
            if self.query('MATCH (n) RETURN count(n) AS count')[0]['count'] != 0:
                raise RuntimeError('La fixture recién creada no está vacía')
            self.image = subprocess.check_output(['docker', 'inspect', '--format', '{{.Image}}', self.container], text=True).strip()
            self.start_bridge()
            return self
        except BaseException as startup_error:
            try:
                self.__exit__(None, None, None)
            except BaseException as cleanup_error:
                raise BaseExceptionGroup(
                    'No se pudo iniciar ni limpiar la fixture Muninn',
                    [startup_error, cleanup_error],
                ) from None
            raise

    def __exit__(self, *_):
        errors = []
        try:
            self.stop_bridge()
        except BaseException as error:
            errors.append(error)
        if self.driver is not None:
            driver, self.driver = self.driver, None
            try:
                driver.close()
            except BaseException as error:
                errors.append(error)
        if self.container is not None:
            container = self.container
            # Only the ID returned by this instance's docker run may be removed.
            try:
                subprocess.run(['docker', 'rm', '-f', container], capture_output=True, check=True)
                self.container = None
            except BaseException as error:
                errors.append(error)
        if errors:
            raise BaseExceptionGroup('Falló la limpieza de la fixture Muninn', errors)
        return False
