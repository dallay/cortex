"""Tests for the rendered OpenCode configuration."""

from __future__ import annotations

import json
import unittest

from membench.config import (
    AGENT_NAME,
    exposed_tool_name,
    forced_tool_prompt,
    render_config,
    write_config,
)


def arguments_fragment(prompt: str) -> str:
    """Return the JSON argument object that the forced prompt renders."""

    marker = "with these arguments: "
    start = prompt.index(marker) + len(marker)
    end = prompt.index(". Then answer", start)
    return prompt[start:end]


class ConfigTests(unittest.TestCase):
    def test_mcp_server_uses_the_owner_scope(self) -> None:
        config = render_config("provider/model", "bench-owner")
        payload = json.loads(config.global_text)
        server = payload["mcp"]["servers"]["muninn"]
        self.assertEqual(server["type"], "local")
        self.assertIn("--owner-scope", server["command"])
        self.assertIn("bench-owner", server["command"])
        self.assertFalse(server["codemode"])
        self.assertEqual(server["protocol"], "2026-07-28")

    def test_mcp_server_lives_in_the_global_configuration(self) -> None:
        config = render_config("provider/model", "bench-owner")
        project = json.loads(config.text)
        self.assertNotIn("mcp", project)

    def test_global_configuration_has_no_schema_reference(self) -> None:
        config = render_config("provider/model", "bench-owner")
        self.assertNotIn("$schema", config.global_text)

    def test_oracle_run_has_no_mcp_server(self) -> None:
        config = render_config("provider/model", "", mcp_enabled=False)
        payload = json.loads(config.global_text)
        self.assertNotIn("mcp", payload)

    def test_agent_denies_every_tool_and_allows_only_muninn(self) -> None:
        config = render_config("provider/model", "bench-owner")
        self.assertIn('action: "*"', config.agent_text)
        self.assertIn('action: "muninn_*"', config.agent_text)
        self.assertIn("effect: deny", config.agent_text)
        self.assertIn("effect: allow", config.agent_text)

    def test_deny_rules_come_before_the_allow_rule(self) -> None:
        config = render_config("provider/model", "bench-owner")
        self.assertLess(config.agent_text.index('effect: deny'), config.agent_text.index('effect: allow'))

    def test_configuration_is_written_to_the_project_directory(self) -> None:
        import tempfile
        from pathlib import Path

        config = render_config("provider/model", "bench-owner")
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            write_config(config, target, "bench-owner")
            self.assertTrue((target / ".opencode" / "opencode.jsonc").is_file())
            self.assertTrue((target / ".opencode" / "agents" / f"{AGENT_NAME}.md").is_file())
            self.assertTrue((target / "global" / "opencode.json").is_file())

    def test_exposed_tool_name_joins_the_server_and_the_tool(self) -> None:
        self.assertEqual(
            exposed_tool_name("muninn_search_messages"),
            "muninn_muninn_search_messages",
        )

    def test_forced_prompt_names_the_tool_and_the_arguments(self) -> None:
        prompt = forced_tool_prompt("Question?", "muninn_search_preferences", {"query": "report"})
        self.assertIn("muninn_search_preferences", prompt)
        self.assertIn('"query": "report"', prompt)

    def test_forced_prompt_keeps_the_type_of_an_integer_argument(self) -> None:
        prompt = forced_tool_prompt(
            "Question?",
            "muninn_search_preferences",
            {"query": "report", "limit": 5},
        )
        fragment = arguments_fragment(prompt)
        self.assertNotIn('"5"', fragment)
        arguments = json.loads(fragment)
        self.assertEqual(arguments["limit"], 5)
        self.assertIsInstance(arguments["limit"], int)

    def test_forced_prompt_escapes_a_quoted_string_argument(self) -> None:
        prompt = forced_tool_prompt(
            "Question?",
            "muninn_search_preferences",
            {"query": 'a "quoted" value', "strict": True, "empty": None},
        )
        arguments = json.loads(arguments_fragment(prompt))
        self.assertEqual(arguments["query"], 'a "quoted" value')
        self.assertIs(arguments["strict"], True)
        self.assertIsNone(arguments["empty"])

    def test_forced_prompt_keeps_the_tool_name_and_the_prompt(self) -> None:
        prompt = forced_tool_prompt(
            "What is the title?", "muninn_search_messages", {"limit": 5}
        )
        self.assertIn("What is the title?", prompt)
        self.assertIn("muninn_search_messages", prompt)

    def test_digest_changes_with_the_scope(self) -> None:
        first = render_config("provider/model", "scope-a")
        second = render_config("provider/model", "scope-b")
        self.assertNotEqual(first.digest(), second.digest())


if __name__ == "__main__":
    unittest.main()
