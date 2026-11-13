"""Tests for the container command helpers."""

from __future__ import annotations

import unittest

from membench.docker import home_prepare_command


class HomePrepareTests(unittest.TestCase):
    def test_the_credential_store_is_copied_to_the_opencode_path(self) -> None:
        command = home_prepare_command("opencode models")
        self.assertIn(
            "/opencode/auth/.local/share/opencode",
            command,
        )
        self.assertIn(
            '"$HOME/.local/share/opencode/"',
            command,
        )

    def test_the_command_runs_after_the_preparation(self) -> None:
        command = home_prepare_command("opencode models")
        self.assertLess(command.index("/opencode/auth"), command.index("opencode models"))

    def test_the_preparation_is_fail_closed(self) -> None:
        self.assertIn("set -eu", home_prepare_command("opencode models"))

    def test_the_preparation_creates_the_home_directory(self) -> None:
        self.assertIn('mkdir -p "$HOME/.local/share/opencode"', home_prepare_command("true"))


if __name__ == "__main__":
    unittest.main()
