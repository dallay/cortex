"""Pruebas del ciclo de vida del capturador sin cargar FastMCP."""

import asyncio
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "mcp_probe.py"


class ProbeTests(unittest.TestCase):
    def test_child_outgoing_connections_and_unattributed_probe_failures_are_rejected(self):
        spec = importlib.util.spec_from_file_location("mcp_probe", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.path.insert(0, str(SCRIPT.parent))
        try:
            spec.loader.exec_module(module)
        finally:
            sys.path.pop(0)
        bad_child = {"blocked": 0, "connects_allowed": 1, "dns_allowed": 0, "binds_allowed": 1}
        with self.assertRaises(ValueError):
            module.validate_child_audit(bad_child, "http")
        with self.assertRaises(ValueError):
            module.validate_probe_failure({"blocked": 1})
        unrelated = RuntimeError("unrelated")
        self.assertFalse(module.caused_by_policy(unrelated))
        unrelated.__cause__ = module.NetworkPolicyError("blocked")
        self.assertTrue(module.caused_by_policy(unrelated))
        expected = {
            "backend": "nams", "validate_on_connect": True, "phase": "nams_probe",
            "role": "server-stdio", "finished": True, "exit_code": 1,
            "blocked": 1, "connects_allowed": 0, "dns_allowed": 0, "binds_allowed": 0,
            "startup_failure": {"type": "TransportError", "caused_by_policy": True},
            "denials": [{"event": "socket.connect", "host": "127.0.0.1", "port": 9,
                         "phase": "nams_probe"}],
        }
        module.validate_probe_failure(expected)
        expected["startup_failure"]["caused_by_policy"] = False
        with self.assertRaises(ValueError):
            module.validate_probe_failure(expected)

    def test_canary_leak_cannot_publish_a_completed_snapshot(self):
        self.assertTrue(SCRIPT.is_file())
        spec = importlib.util.spec_from_file_location("mcp_probe", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.path.insert(0, str(SCRIPT.parent))
        try:
            spec.loader.exec_module(module)
        finally:
            sys.path.pop(0)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            (output / "stderr.log").write_text("synthetic-canary", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "canary"):
                module.publish_snapshot(output, {"scope": "fixture"}, "synthetic-canary")
            self.assertFalse((output / "discovery.json").exists())
            (output / "stderr.log").write_text("no credentials", encoding="utf-8")
            module.publish_snapshot(output, {"scope": "fixture"}, "synthetic-canary")
            self.assertTrue((output / "discovery.json").is_file())

    def test_help_does_not_require_framework_or_open_a_server(self):
        self.assertTrue(SCRIPT.is_file(), "Falta el capturador MCP")
        result = subprocess.run([sys.executable, str(SCRIPT), "--help"], text=True,
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0)
        self.assertIn("--sources", result.stdout)
        self.assertIn("--output", result.stdout)


class ProcessTests(unittest.IsolatedAsyncioTestCase):
    @unittest.skipUnless(os.name == "posix", "Prueba de señales POSIX del runner Docker")
    async def test_forced_kill_is_reported_instead_of_claiming_clean_shutdown(self):
        spec = importlib.util.spec_from_file_location("mcp_probe", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.path.insert(0, str(SCRIPT.parent))
        try:
            spec.loader.exec_module(module)
        finally:
            sys.path.pop(0)
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c",
            "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print('ready',flush=True); time.sleep(60)",
            stdout=asyncio.subprocess.PIPE)
        try:
            await process.stdout.readline()
            status = await module.stop_process(process, grace_seconds=0.05)
            self.assertTrue(status["forced_kill"])
            self.assertEqual(status["return_code"], -9)
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()

    async def test_child_process_is_reaped_after_cleanup(self):
        self.assertTrue(SCRIPT.is_file(), "Falta el capturador MCP")
        spec = importlib.util.spec_from_file_location("mcp_probe", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        sys.path.insert(0, str(SCRIPT.parent))
        try:
            spec.loader.exec_module(module)
        finally:
            sys.path.pop(0)
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-c", "import time; time.sleep(60)")
        try:
            await module.stop_process(process)
            self.assertIsNotNone(process.returncode)
        finally:
            if process.returncode is None:
                process.kill()
                await process.wait()


if __name__ == "__main__":
    unittest.main()
