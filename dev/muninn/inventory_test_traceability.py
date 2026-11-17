"""Trazabilidad estática declarada por TCK; nunca ejecución ni paridad."""
import ast
import csv
import hashlib
import io
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
LIMIT = 4 * 1024 * 1024
SPEC = re.compile(r'SPEC-[1-9][0-9]*\.[1-9][0-9]*\.[1-9][0-9]*\Z')
FILES = ('test_cross_memory.py', 'test_long_term.py', 'test_reasoning.py',
         'test_schema.py', 'test_short_term.py')
GAPS = {
    'api': 'Falta asociar el contrato de la operación con una prueba conductual específica.',
    'type': 'Falta asociar el tipo o campo con pruebas de su contrato observable.',
    'schema': 'Falta asociar la cláusula de esquema con aserciones específicas.',
    'mcp': 'Falta asociar la herramienta con pruebas de descubrimiento, esquema y comportamiento.',
    'ddl': 'Falta asociar la declaración DDL con pruebas de introspección y ciclo de vida.',
}


def bounded(path):
    with path.open('rb') as stream:
        raw = stream.read(LIMIT + 1)
    if len(raw) > LIMIT:
        raise ValueError('Archivo excede límite')
    return raw


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def extract(raw):
    """Solo docstrings de funciones test de nivel módulo/clase; no ejecutar AST."""
    if len(raw) > LIMIT:
        raise ValueError('Archivo excede límite')
    tree = ast.parse(raw.decode('utf-8'))
    links = {}
    nodes = set()
    classes = set()

    def visit(body, prefix=''):
        for node in body:
            if isinstance(node, ast.ClassDef):
                qualified_name = prefix + node.name
                if qualified_name in classes:
                    raise ValueError('Clase de prueba duplicada')
                classes.add(qualified_name)
                visit(node.body, qualified_name + '::')
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith('test_'):
                name = prefix + node.name
                if name in nodes:
                    raise ValueError('Nodo de prueba duplicado')
                nodes.add(name)
                doc = ast.get_docstring(node) or ''
                for match in re.finditer(r'SPEC-[^\s:,;()]+', doc):
                    identifier = match.group().rstrip('.')
                    if not SPEC.fullmatch(identifier):
                        raise ValueError('Identificador SPEC inválido')
                    if identifier in links:
                        raise ValueError('Declaración SPEC duplicada')
                    links[identifier] = name
    visit(tree.body)
    return links


def checked_extract(raw, expected):
    if digest(raw) != expected:
        raise ValueError('Fallo de integridad de fuente')
    return extract(raw)


def build(root=ROOT):
    integrity_raw = bounded(root / 'dev/muninn/source-integrity.json')
    references_raw = bounded(root / 'dev/muninn/references.json')
    integrity = json.loads(integrity_raw)['agent-memory-tck']
    reference = json.loads(references_raw)['agent-memory-tck']
    commit = integrity['commit']
    if commit != reference['commit'] or not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('Pins inconsistentes')
    matrix_raw = bounded(root / 'docs/muninn/specs/parity-matrix.csv')
    rows = list(csv.DictReader(io.StringIO(matrix_raw.decode('utf-8'))))
    ids = [row['id'] for row in rows]
    if not ids or len(ids) != len(set(ids)):
        raise ValueError('IDs de inventario vacíos o duplicados')
    associations = {}
    unmapped_declarations = []
    declared_ids = set()
    sources = []
    for name in FILES:
        relative = 'tck/tests/v1/' + name
        expected = integrity['files'][relative]
        raw = bounded(root / 'dev/muninn/.cache/sources/agent-memory-tck' / relative)
        found = checked_extract(raw, expected)
        sources.append({'path': relative, 'sha256': expected, 'size': len(raw)})
        for identifier, node in found.items():
            if identifier in declared_ids:
                raise ValueError('Declaración SPEC duplicada entre fuentes: ' + identifier)
            declared_ids.add(identifier)
            if identifier not in ids:
                unmapped_declarations.append({'id': identifier, 'path': relative, 'nodeid': node,
                                              'reason': 'Declaración upstream sin fila en el inventario; requiere revisión.'})
                continue
            associations[identifier] = {'repository': 'agent-memory-tck', 'commit': commit,
                                        'path': relative, 'nodeid': relative + '::' + node,
                                        'sha256': expected}
    result = []
    for row in rows:
        identifier = row['id']
        if identifier.startswith('SPEC-'):
            if not SPEC.fullmatch(identifier) or identifier not in associations:
                raise ValueError('Cláusula normativa sin asociación: ' + identifier)
            status, gap = 'upstream_declared_not_run', ''
        else:
            kind = identifier.split(':', 1)[0]
            if kind not in GAPS:
                raise ValueError('Grupo desconocido: ' + identifier)
            status, gap = 'unresolved', GAPS[kind]
        result.append({'id': identifier, 'source_ref': row['source_ref'], 'status': status,
                       'tests': [associations[identifier]] if identifier in associations else [],
                       'gap_reason': gap})
    return {'schema_version': 1, 'scope': 'static_upstream_declarations_not_execution',
            'execution': 'not_run', 'parity_claim': False,
            'inputs': {'matrix_sha256': digest(matrix_raw), 'integrity_sha256': digest(integrity_raw),
                       'references_sha256': digest(references_raw)},
            'sources': sources,
            'unmapped_declarations': unmapped_declarations,
            'exclusions': [{'path': 'tck/tests/v1/test_platinum.py',
                            'reason': 'Familia Platinum separada: reutiliza IDs SPEC; no unir por texto.'}],
            'summary': {'inventory_rows': len(rows), 'declared_links': len(associations),
                        'unresolved_rows': len(rows) - len(associations)},
            'entries': result}


def serialized(report):
    return (json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Comparar artefacto sin escribir')
    args = parser.parse_args()
    output = ROOT / 'docs/muninn/specs/inventory-test-traceability.json'
    payload = serialized(build())
    if args.check:
        if bounded(output) != payload:
            raise SystemExit('Artefacto desactualizado')
    else:
        output.write_bytes(payload)
    print(json.dumps(json.loads(payload)['summary']))
