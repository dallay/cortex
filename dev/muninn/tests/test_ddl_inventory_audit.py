"""Auditoría estática; no importa ni ejecuta upstream."""
import csv
import io
import unittest
from unittest import mock

import ddl_inventory_audit as audit


class DdlAuditTests(unittest.TestCase):
    def test_distinguishes_docstrings_templates_statements_and_logs(self):
        raw = b'''QUERY = "CREATE INDEX example"
def query(name):
    """Generate CREATE INDEX query."""
    return f"CREATE INDEX {name}"
def log(logger):
    logger.info("Created constraint: %s", "name")
'''
        found = audit.classify(raw)
        self.assertEqual([row["kind"] for row in found], [
            "statement_literal", "documentation_literal", "template_fragment",
        ])
        self.assertEqual(found[2]["owner"], "query")
        self.assertEqual(found[1]["line"], 3)
        self.assertEqual(found[2]["literal"], "CREATE INDEX")

    def test_log_containing_create_is_not_a_statement(self):
        found = audit.classify(b'def log(logger):\n    logger.info("CREATE INDEX attempted")\n')
        self.assertEqual(found[0]["kind"], "log_literal")

    def test_exception_fragment_is_not_a_query_template(self):
        found = audit.classify(
            b'def fail(name):\n    raise ValueError(f"Failed to create index {name}")\n'
        )
        self.assertEqual(found[0]["kind"], "exception_literal")

    def test_extended_pattern_detects_text_and_point_without_changing_legacy(self):
        raw = b'TEXT = "CREATE TEXT INDEX example"\nPOINT = "CREATE POINT INDEX geo"\n'
        self.assertEqual(audit.classify(raw), [])
        found = audit.classify(raw, pattern=audit.EXTENDED_DDL)
        self.assertEqual(len(found), 2)
        self.assertTrue(all(row["kind"] == "statement_literal" for row in found))

    def test_supplementary_candidates_are_not_inventory_rows(self):
        report = audit.build()
        candidates = report["supplementary_candidates"]
        self.assertEqual(len(candidates), 4)
        self.assertEqual({row["owner"] for row in candidates},
                         {"create_text_index_query", "create_point_index_query"})
        self.assertEqual({row["kind"] for row in candidates},
                         {"documentation_literal", "template_fragment"})
        for row in candidates:
            self.assertEqual(row["inventory_status"], "absent_from_frozen_inventory")
            self.assertEqual(row["behavior_status"], "not_tested")

    def test_rejects_omitted_original_ddl_row(self):
        original = audit.bounded
        matrix = audit.ROOT / "docs/muninn/specs/parity-matrix.csv"
        rows = list(csv.DictReader(io.StringIO(original(matrix).decode())))
        omitted = next(row for row in rows if row["id"].startswith("ddl:"))
        stream = io.StringIO()
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(row for row in rows if row is not omitted)
        altered = stream.getvalue().encode()

        def read(path):
            return altered if path == matrix else original(path)

        with mock.patch.object(audit, "bounded", side_effect=read):
            with self.assertRaisesRegex(ValueError, "incompleto"):
                audit.build()

    def test_rejects_invalid_source(self):
        with self.assertRaises(SyntaxError):
            audit.classify(b'def broken(')

    def test_frozen_inventory_reconciles_without_behavior_claim(self):
        report = audit.build()
        self.assertEqual(len(report["entries"]), 11)
        self.assertFalse(report["parity_claim"])
        counts = {}
        for entry in report["entries"]:
            counts[entry["kind"]] = counts.get(entry["kind"], 0) + 1
            self.assertEqual(entry["behavior_status"], "not_tested")
        self.assertEqual(counts, {
            "statement_literal": 2, "template_fragment": 3,
            "documentation_literal": 4, "exception_literal": 2,
        })
