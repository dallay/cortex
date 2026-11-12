"""Tests for the preflight helpers."""

from __future__ import annotations

import unittest

from membench.preflight import auth_providers, model_in_catalog


class CatalogTests(unittest.TestCase):
    def test_model_is_found_in_the_catalog(self) -> None:
        catalog = "opencode/grok-code\nprovider/model\nother/model\n"
        self.assertTrue(model_in_catalog(catalog, "provider/model"))
        self.assertTrue(model_in_catalog(catalog, "other/model"))
        self.assertFalse(model_in_catalog(catalog, "missing/model"))

    def test_a_similar_identifier_is_not_a_match(self) -> None:
        catalog = "provider/model-variant\n"
        self.assertFalse(model_in_catalog(catalog, "provider/model"))

    def test_a_qualified_catalog_line_is_a_match(self) -> None:
        self.assertTrue(model_in_catalog("provider/model (1M context)\n", "provider/model"))

    def test_empty_model_is_never_found(self) -> None:
        self.assertFalse(model_in_catalog("provider/model", "  "))


class AuthTests(unittest.TestCase):
    def test_provider_names_are_extracted(self) -> None:
        output = "Credentials\n  ● provider  api-key\n  ● other  oauth\n"
        self.assertEqual(auth_providers(output), ["provider", "other"])

    def test_empty_output_yields_no_providers(self) -> None:
        self.assertEqual(auth_providers(""), [])


if __name__ == "__main__":
    unittest.main()
