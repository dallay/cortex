"""Valida el recibo normativo Bronze sin reinterpretar resultados pytest."""


def validate_bronze(items, report, exit_code):
    ids = [item['nodeid'] for item in items]
    expected = {item['nodeid'] for item in items if 'bronze' in item['markers']}
    tests = report.get('tests', [])
    actual = [test['nodeid'] for test in tests]
    if (exit_code != 0 or len(ids) != 189 or len(set(ids)) != 189
            or len(expected) != 93 or len(actual) != 93 or set(actual) != expected
            or any(test.get('outcome') != 'passed' for test in tests)):
        raise ValueError('Bronze requiere 189 IDs únicos, 93 seleccionados exactos y 93 PASS sin omisiones')
    return {'collected': 189, 'selected': 93, 'passed': 93, 'deselected': 96,
            'selected_ids': sorted(expected)}
