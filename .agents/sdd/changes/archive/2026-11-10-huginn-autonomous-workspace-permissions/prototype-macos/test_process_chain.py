#!/usr/bin/env python3
"""Focused executable test for the helper -> shell -> nested-child probe chain."""

import json
import pathlib
import shutil
import subprocess
import tempfile
import unittest


HERE = pathlib.Path(__file__).resolve().parent


class ProcessChainTest(unittest.TestCase):
    def test_shell_is_actor_and_quoted_helper_path_survives_spaces(self):
        with tempfile.TemporaryDirectory(prefix="huginn-shell-chain-") as temp:
            root = pathlib.Path(temp) / "fixture root"
            resource = root / "outside resource"
            resource.mkdir(parents=True)
            (root / ".prototype-fixture").write_text("disposable fixture root\n")
            (resource / "marker.txt").write_text("sentinel\n")
            helper = pathlib.Path(temp) / "helper path with spaces" / "Probe Helper"
            helper.parent.mkdir()
            subprocess.run(["swiftc", str(HERE / "Helper.swift"), "-o", str(helper)], check=True)

            result = subprocess.run(
                [str(helper), "--test-chain", str(root), str(resource)],
                capture_output=True, text=True, timeout=10, check=False,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            records = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual({record["role"] for record in records}, {"helper", "shell", "nested"})
            self.assertTrue(next(r for r in records if r["role"] == "helper")["succeeded"])
            self.assertTrue(next(r for r in records if r["role"] == "shell")["succeeded"])
            self.assertTrue(next(r for r in records if r["role"] == "nested")["succeeded"])
            self.assertEqual((resource / "marker.txt").read_text(), "replacement")


if __name__ == "__main__":
    unittest.main()
