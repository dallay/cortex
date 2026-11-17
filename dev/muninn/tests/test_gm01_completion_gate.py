"""La aceptación solo se deriva de la cobertura ejecutada sin errores."""
import unittest


class CompletionGateTests(unittest.TestCase):
    def test_missing_cases_stays_incomplete(self):
        from gm01_lifecycle import finalize_lifecycle
        report = {'cases': [], 'errors': [], 'status': 'incomplete'}
        finalize_lifecycle(report)
        self.assertEqual(report['status'], 'incomplete')
        self.assertTrue(report['pending'])

    def test_empty_success_cases_cannot_establish_acceptance(self):
        from gm01_lifecycle import finalize_lifecycle
        cases = [{'id': f'{prefix}-{suffix}', 'errors': []}
                 for prefix in ('create', 'graph', 'delete-effects', 'delete')
                 for suffix in ('first', 'middle', 'last')]
        for prefix, count in [('restart-live', 6), ('clear-session', 6), ('append-', 6), ('invalid-role', 6)]:
            cases.extend({'id': prefix, 'errors': []} for _ in range(count))
        cases.append({'id': 'interleaved-sessions', 'errors': []})
        report = {'cases': cases, 'errors': []}
        finalize_lifecycle(report)
        self.assertEqual(report['status'], 'incomplete')
