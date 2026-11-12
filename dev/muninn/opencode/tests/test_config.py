"""Tests for the rendered OpenCode configuration."""

from __future__ import annotations

import json
import unittest

from membench.config import (
    AGENT_NAME,
    forced_tool_prompt,
    render_config,
    write_config,
)


class ConfigTests(unittest.TestCase):
    def test_mcp_server_uses_the_owner_scope(self) -> None:
        config = render_config("provider/model", "bench-owner")
        payload = json.loads(config.text)
        server = payload["mcp"]["servers"]["muninn"]
        self.assertEqual(server["type"], "local")
        self.assertIn("--owner-scope", server["command"])
        self.assertIn("bench-owner", server["command"])

    def test_oracle_run_has_no_mcp_server(self) -> None:
        config = render_config("provider/model", "", mcp_enabled=False)
        payload = json.loads(config.text)
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
            write_config(config, target)
            self.assertTrue((target / ".opencode" / "opencode.jsonc").is_file())
            self.assertTrue((target / ".opencode" / "agents" / f"{AGENT_NAME}.md").is_file())

    def test_forced_prompt_names_the_tool_and_the_arguments(self) -> None:
        prompt = forced_tool_prompt("Question?", "muninn_search_preferences", {"query": "report"})
        self.assertIn("muninn_search_preferences", prompt)
        self.assertIn('"query": "report"', prompt)

    def test_digest_changes_with_the_scope(self) -> None:
        first = render_config("provider/model", "scope-a")
        second = render_config("provider/model", "scope-b")
        self.assertNotEqual(first.digest(), second.digest())


if __name__ == "__main__":
    unittest.main()
