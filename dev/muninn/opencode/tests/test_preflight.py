"""Tests for the preflight helpers."""

from __future__ import annotations

import unittest

from membench.preflight import (
    NOT_TESTED,
    Preflight,
    auth_providers,
    check_model_catalog,
    model_in_catalog,
)


class _FakeResult:
    """Result of a command that was never executed."""

    ok = False
    stdout = ""
    stderr = ""

    def summary(self) -> str:
        return "the command was not executed"


class _FakeRunner:
    """Runner that records the commands it was asked to execute."""

    def __init__(self) -> None:
        self.commands: list[list[str]] = []

    def compose_exec(self, *args, **kwargs) -> _FakeResult:
        self.commands.append(list(args))
        return _FakeResult()


class ModelCatalogTests(unittest.TestCase):
    """The catalog check only applies to a plan that calls the model."""

    def test_catalog_is_not_queried_when_no_inference_is_required(self) -> None:
        runner = _FakeRunner()
        preflight = Preflight()
        check_model_catalog(runner, None, "provider/model", preflight, None, False)
        self.assertEqual(runner.commands, [])
        catalog = next(
            check for check in preflight.checks if check.name == "MODEL_CATALOG"
        )
        self.assertEqual(catalog.state, NOT_TESTED)
        self.assertFalse(catalog.blocking)
        self.assertTrue(preflight.ok)

    def test_catalog_is_queried_when_an_inference_is_required(self) -> None:
        runner = _FakeRunner()
        preflight = Preflight()
        check_model_catalog(runner, None, "provider/model", preflight, None, True)
        self.assertEqual(len(runner.commands), 1)


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
