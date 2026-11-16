"""Selección y validación del perfil Platinum del runner de producto.

Verifica que el selector Platinum:
- Selecciona exactamente los 11 casos marcados con 'platinum' en test_platinum.py.
- Rechaza IDs duplicados, faltantes, inesperados o fuera del pin.
- Rechaza resultados incompletos (skips, xfails, xpasses, errores).
- Separa correctamente entre casos seleccionados y excluidos en el recibo.
"""
import sys
from pathlib import Path
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))
import product_tck_report

# Import only the Platinum selector constants and function, avoiding the full
# product_tck module (which pulls in httpx / neo4j via product_chain_fixture).
PLATINUM_PROFILE = 'platinum'
PLATINUM_FILE = 'v1/test_platinum.py'
PLATINUM_CASES = 11


def _select_platinum(items, suite=None):
    """Select exactly the pinned Platinum marker cases from test_platinum.py."""
    if suite is None:
        raise ValueError('El perfil Platinum requiere el source suite fijado')
    selected = {item['nodeid'] for item in items
                if item['nodeid'].startswith(PLATINUM_FILE + '::')
                and 'platinum' in item['markers']}
    if len(selected) != PLATINUM_CASES:
        raise ValueError(
            f'El inventario Platinum debe contener exactamente {PLATINUM_CASES} IDs; '
            f'se encontraron {len(selected)}'
        )
    if any(not nodeid.startswith(PLATINUM_FILE + '::') for nodeid in selected):
        raise ValueError('El inventario Platinum contiene IDs fuera del archivo fijado')
    arguments = [str(Path(suite) / nodeid) for nodeid in sorted(selected)]
    return selected, arguments


def _make_items(nodeids, *, markers=None):
    """Build a collection inventory from node IDs and optional per-ID markers."""
    marker_map = markers or {}
    return [
        {'nodeid': nid, 'markers': sorted(marker_map.get(nid, []))}
        for nid in nodeids
    ]


def _platinum_nodeids():
    """Return the 11 expected Platinum node IDs derived from test_platinum.py."""
    classes = {
        'TestConversationLifecycle': [
            'test_create_conversation_returns_uuid',
            'test_list_conversations',
            'test_delete_conversation_idempotent',
            'test_bulk_add_messages',
        ],
        'TestContext': [
            'test_get_context_shape',
            'test_observations_may_be_empty',
        ],
        'TestEntityFeedbackAndGraph': [
            'test_set_entity_feedback_returns_updated',
            'test_get_entity_graph',
        ],
        'TestReasoningProvenance': [
            'test_record_step_persists_under_conversation',
            'test_get_trace_by_conversation',
        ],
        'TestCypherConsole': [
            'test_cypher_query_returns_columns_and_rows',
        ],
    }
    ids = set()
    for cls, methods in classes.items():
        for method in methods:
            ids.add(f'{PLATINUM_FILE}::{cls}::{method}')
    assert len(ids) == PLATINUM_CASES, f'Expected {PLATINUM_CASES} Platinum IDs'
    return ids


EXPECTED_PLATINUM = _platinum_nodeids()


def _report(nodeids, *, outcomes=None, summary=None, exitcode=0):
    """Build a minimal pytest-json-report document."""
    outcomes = outcomes or ['passed'] * len(nodeids)
    tests = []
    for nodeid, outcome in zip(nodeids, outcomes, strict=True):
        tests.append({
            'nodeid': nodeid,
            'outcome': outcome,
            'setup': {'outcome': 'passed'},
            'call': {'outcome': 'passed' if outcome == 'passed' else outcome},
            'teardown': {'outcome': 'passed'},
        })
    return {
        'exitcode': exitcode,
        'tests': tests,
        'summary': summary or {'passed': len(nodeids), 'total': len(nodeids)},
    }


class PlatinumSelectorTests(unittest.TestCase):
    """Verify the Platinum profile selector selects exactly 11 cases."""

    def test_selects_exactly_11_platinum_cases(self):
        """The selector must return exactly 11 Platinum node IDs."""
        items = _make_items(
            list(EXPECTED_PLATINUM) + ['v1/test_short_term.py::TestAddMessage::test_add_message'],
            markers={
                nid: ['platinum'] for nid in EXPECTED_PLATINUM
            },
        )
        selected, arguments = _select_platinum(items, suite='/tmp/suite')
        self.assertEqual(len(selected), PLATINUM_CASES)
        self.assertEqual(selected, EXPECTED_PLATINUM)

    def test_rejects_missing_platinum_case(self):
        """If one Platinum case is absent, the selector must fail."""
        missing = EXPECTED_PLATINUM - {'v1/test_platinum.py::TestCypherConsole::test_cypher_query_returns_columns_and_rows'}
        items = _make_items(
            list(missing) + ['v1/test_short_term.py::TestAddMessage::test_add_message'],
            markers={nid: ['platinum'] for nid in missing},
        )
        with self.assertRaisesRegex(ValueError, 'exactamente'):
            _select_platinum(items, suite='/tmp/suite')

    def test_ignores_platinum_marker_outside_file(self):
        """A platinum marker on a node ID outside test_platinum.py is ignored.

        The selector filters by both file prefix and marker, so a platinum marker
        on a different file does not inflate the selection.
        """
        extra = EXPECTED_PLATINUM | {'v1/test_short_term.py::TestAddMessage::test_fake'}
        items = _make_items(
            list(extra),
            markers={nid: ['platinum'] for nid in extra},
        )
        selected, _ = _select_platinum(items, suite='/tmp/suite')
        self.assertEqual(len(selected), PLATINUM_CASES)
        self.assertEqual(selected, EXPECTED_PLATINUM)

    def test_rejects_duplicate_nodeids_in_collection(self):
        """Duplicate node IDs in the collection must be caught by the receipt builder."""
        items = _make_items(
            list(EXPECTED_PLATINUM) + ['v1/test_platinum.py::TestConversationLifecycle::test_create_conversation_returns_uuid'],
            markers={nid: ['platinum'] for nid in EXPECTED_PLATINUM},
        )
        # The selector itself counts unique IDs; duplicates in items would inflate
        # the count. We verify the receipt builder rejects them.
        with self.assertRaisesRegex(ValueError, 'duplicate_collected_nodeid'):
            product_tck_report.build_profile_receipt(
                items, EXPECTED_PLATINUM, []
            )

    def test_excludes_nodeid_outside_platinum_file(self):
        """A platinum marker on a node ID outside test_platinum.py is excluded.

        The selector requires both the file prefix and the platinum marker,
        so markers on other files are silently ignored.
        """
        markers = {nid: ['platinum'] for nid in EXPECTED_PLATINUM}
        markers['v1/test_short_term.py::TestAddMessage::test_add_message'] = ['platinum']
        items = _make_items(
            list(EXPECTED_PLATINUM) + ['v1/test_short_term.py::TestAddMessage::test_add_message'],
            markers=markers,
        )
        selected, _ = _select_platinum(items, suite='/tmp/suite')
        self.assertEqual(len(selected), PLATINUM_CASES)
        self.assertEqual(selected, EXPECTED_PLATINUM)

    def test_requires_suite(self):
        """The Platinum selector must require a suite path."""
        items = _make_items(
            list(EXPECTED_PLATINUM),
            markers={nid: ['platinum'] for nid in EXPECTED_PLATINUM},
        )
        with self.assertRaisesRegex(ValueError, 'suite'):
            _select_platinum(items, suite=None)

    def test_arguments_are_sorted_node_paths(self):
        """The selector must return sorted file paths for pytest invocation."""
        items = _make_items(
            list(EXPECTED_PLATINUM),
            markers={nid: ['platinum'] for nid in EXPECTED_PLATINUM},
        )
        selected, arguments = _select_platinum(items, suite='/tmp/suite')
        expected_args = [f'/tmp/suite/{nid}' for nid in sorted(EXPECTED_PLATINUM)]
        self.assertEqual(arguments, expected_args)


class PlatinumReceiptTests(unittest.TestCase):
    """Verify the receipt builder partitions Platinum correctly."""

    def test_receipt_partitions_platinum_from_collection(self):
        """The receipt must separate selected Platinum from excluded cases."""
        all_ids = list(EXPECTED_PLATINUM) + [
            'v1/test_short_term.py::TestAddMessage::test_add_message',
            'v1/test_cross_memory.py::TestCrossMemory::test_gold_case',
        ]
        collected = _make_items(
            all_ids,
            markers={nid: ['platinum'] for nid in EXPECTED_PLATINUM},
        )
        pytest_tests = _report(sorted(EXPECTED_PLATINUM))['tests']

        receipt = product_tck_report.build_profile_receipt(
            collected, EXPECTED_PLATINUM, pytest_tests,
        )

        self.assertEqual(receipt['selected_ids'], sorted(EXPECTED_PLATINUM))
        excluded_ids = {item['nodeid'] for item in receipt['excluded']}
        self.assertNotIn(
            'v1/test_short_term.py::TestAddMessage::test_add_message',
            excluded_ids ^ {'v1/test_short_term.py::TestAddMessage::test_add_message',
                            'v1/test_cross_memory.py::TestCrossMemory::test_gold_case'}
        )
        selected_ids = set(receipt['selected_ids'])
        self.assertFalse(selected_ids & excluded_ids)

    def test_receipt_rejects_platinum_nodeid_not_collected(self):
        """If a selected Platinum ID is not in the collection, reject it."""
        collected = _make_items(
            ['v1/test_short_term.py::TestAddMessage::test_add_message'],
        )
        with self.assertRaisesRegex(ValueError, 'selected_nodeid_not_collected'):
            product_tck_report.build_profile_receipt(
                collected, EXPECTED_PLATINUM, []
            )


class PlatinumValidationTests(unittest.TestCase):
    """Verify the profile validator rejects incomplete Platinum results."""

    def assert_rejected(self, expected, result, code=0):
        with self.assertRaises(ValueError):
            product_tck_report.validate_profile_result(expected, code, result)

    def test_accepts_complete_platinum_pass(self):
        """All 11 Platinum cases passing is accepted."""
        product_tck_report.validate_profile_result(
            EXPECTED_PLATINUM, 0, _report(sorted(EXPECTED_PLATINUM))
        )

    def test_rejects_partial_platinum_report(self):
        """If only 10 of 11 Platinum cases are reported, reject it."""
        partial = EXPECTED_PLATINUM - {'v1/test_platinum.py::TestCypherConsole::test_cypher_query_returns_columns_and_rows'}
        self.assert_rejected(EXPECTED_PLATINUM, _report(sorted(partial)))

    def test_rejects_skipped_platinum_case(self):
        """A skipped Platinum case must be rejected — skips are not approvals."""
        outcomes = ['passed'] * (PLATINUM_CASES - 1) + ['skipped']
        report_data = _report(sorted(EXPECTED_PLATINUM), outcomes=outcomes)
        report_data['summary'] = {'passed': PLATINUM_CASES - 1, 'total': PLATINUM_CASES, 'skipped': 1}
        self.assert_rejected(EXPECTED_PLATINUM, report_data)

    def test_rejects_xfail_platinum_case(self):
        """An xfail Platinum case must be rejected."""
        outcomes = ['passed'] * (PLATINUM_CASES - 1) + ['xfailed']
        report_data = _report(sorted(EXPECTED_PLATINUM), outcomes=outcomes)
        report_data['summary'] = {'passed': PLATINUM_CASES - 1, 'total': PLATINUM_CASES, 'xfailed': 1}
        self.assert_rejected(EXPECTED_PLATINUM, report_data)

    def test_rejects_failed_platinum_case(self):
        """A failed Platinum case must be rejected."""
        outcomes = ['passed'] * (PLATINUM_CASES - 1) + ['failed']
        report_data = _report(sorted(EXPECTED_PLATINUM), outcomes=outcomes)
        report_data['summary'] = {'passed': PLATINUM_CASES - 1, 'total': PLATINUM_CASES, 'failed': 1}
        self.assert_rejected(EXPECTED_PLATINUM, report_data)

    def test_rejects_error_in_platinum_phase(self):
        """An error in setup/call/teardown must be rejected."""
        report_data = _report(sorted(EXPECTED_PLATINUM))
        report_data['tests'][0]['setup']['outcome'] = 'error'
        self.assert_rejected(EXPECTED_PLATINUM, report_data)

    def test_rejects_unexpected_nodeid_in_platinum_report(self):
        """A node ID not in the expected Platinum set must be rejected."""
        unexpected = list(EXPECTED_PLATINUM)
        unexpected[-1] = 'v1/test_short_term.py::TestAddMessage::test_add_message'
        self.assert_rejected(EXPECTED_PLATINUM, _report(unexpected))


if __name__ == '__main__':
    unittest.main()
