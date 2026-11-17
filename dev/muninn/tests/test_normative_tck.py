"""El baseline normativo no debe sustituirse por el inventario histórico."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest

BASE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BASE))
import rust_tck_spike as runner


class NormativeTests(unittest.TestCase):
    def test_normative_commit_and_history_are_explicit(self):
        lock = runner.load_normative_lock(BASE)
        self.assertEqual(lock['commit'], 'b6c72c36bb7486881b87053ee1610f2caf972fb6')
        self.assertEqual(lock['historical_commit'], 'e60b410f1046dc21db50f9aba48e171dc8a9b5a1')
        self.assertIn('tck/tests/v1/test_short_term.py', lock['files'])

    def test_normative_lock_rejects_changed_commit(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / 'reference.lock.yaml').write_text(json.dumps({'commit': '0' * 40}))
            with self.assertRaises(ValueError):
                runner.load_normative_lock(path)
