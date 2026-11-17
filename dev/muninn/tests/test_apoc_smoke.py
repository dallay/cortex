"""No se aprueba una activación por la mera existencia del JAR."""

import importlib.util
from pathlib import Path
import unittest


class ApocTests(unittest.TestCase):
    def test_requires_an_actual_matching_function_result(self):
        path = Path(__file__).resolve().parents[1] / "apoc_smoke.py"
        self.assertTrue(path.is_file(), "Falta la prueba funcional APOC")
        spec = importlib.util.spec_from_file_location("apoc_smoke", path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertTrue(module.valid_result([{"version":"5.26.32"}], "5.26.32"))
        self.assertFalse(module.valid_result([], "5.26.32"))
        self.assertFalse(module.valid_result([{"version":"other"}], "5.26.32"))


if __name__ == "__main__":
    unittest.main()
