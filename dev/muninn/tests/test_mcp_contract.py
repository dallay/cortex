"""Regresiones de integridad del discovery, sin dependencias MCP externas."""

import importlib.util
from pathlib import Path
import unittest


def load_contract():
    path = Path(__file__).resolve().parents[1] / "mcp_contract.py"
    if not path.is_file():
        raise AssertionError("Falta el contrato del discovery MCP")
    spec = importlib.util.spec_from_file_location("mcp_contract", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class DiscoveryTests(unittest.TestCase):
    def test_profile_gating_and_complete_surfaces_are_required(self):
        contract = load_contract()
        self.assertEqual(len(contract.expected_surface("core", "none")["tools"]), 6)
        self.assertEqual(len(contract.expected_surface("extended", "none")["tools"]), 16)
        self.assertEqual(len(contract.expected_surface("core", "nams")["tools"]), 6)
        self.assertEqual(len(contract.expected_surface("extended", "nams")["tools"]), 20)
        expected = contract.expected_surface("core", "none")
        payload = {
            "tools": {"tools": [{"name": name, "inputSchema": {"type": "object"}}
                                  for name in sorted(expected["tools"])]},
            "resources": {"resources": []},
            "resourceTemplates": {"resourceTemplates": [{"uriTemplate": uri} for uri in expected["templates"]]},
            "prompts": {"prompts": [{"name": name} for name in expected["prompts"]]},
        }
        contract.validate_surface(payload, "core", "none")
        removed = payload.pop("resources")
        with self.assertRaisesRegex(ValueError, "resources"):
            contract.validate_surface(payload, "core", "none")
        payload["resources"] = removed
        payload["tools"]["resultType"] = "input_required"
        with self.assertRaises(ValueError):
            contract.validate_surface(payload, "core", "none")
        payload["tools"]["resultType"] = "complete"
        payload["resourceTemplates"]["resourceTemplates"] = []
        with self.assertRaisesRegex(ValueError, "templates"):
            contract.validate_surface(payload, "core", "none")

    def test_duplicate_names_or_unconsumed_pages_cannot_pass(self):
        contract = load_contract()
        payload = {"tools": {"tools": [{"name": "memory_search"}, {"name": "memory_search"}]}}
        with self.assertRaises(ValueError):
            contract.validate_surface(payload, "core", "none")
        payload = {"tools": {"tools": [], "nextCursor": "another-page"}}
        with self.assertRaises(ValueError):
            contract.validate_surface(payload, "core", "none")

    def test_network_guard_allows_only_the_local_mcp_endpoint(self):
        contract = load_contract()
        audit = contract.NetworkAudit()
        audit("socket.connect", (None, ("127.0.0.1", 8765)))
        server = contract.NetworkAudit(role="server-http")
        server("socket.bind", (None, ("127.0.0.1", 8765)))
        for event, args in (
            ("socket.connect", (None, ("127.0.0.1", 9))),
            ("socket.connect", (None, ("203.0.113.1", 8765))),
            ("socket.bind", (None, ("0.0.0.0", 8765))),
            ("socket.getaddrinfo", ("memory.example.invalid", 443, 0, 0, 0)),
        ):
            with self.assertRaises(PermissionError):
                audit(event, args)
        self.assertEqual(audit.snapshot()["blocked"], 4)
        self.assertEqual(audit.snapshot()["connects_allowed"], 1)

    def test_audit_is_persisted_before_a_block_without_waiting_for_shutdown(self):
        contract = load_contract()
        snapshots = []
        audit = contract.NetworkAudit(sink=snapshots.append, role="server-http")
        audit("socket.bind", (None, ("127.0.0.1", 8765)))
        self.assertEqual(snapshots[-1]["binds_allowed"], 1)
        with self.assertRaises(PermissionError):
            audit("socket.connect", (None, ("127.0.0.1", 9)))
        self.assertEqual(snapshots[-1]["blocked"], 1)

    def test_server_roles_reject_outgoing_connections_and_other_socket_operations(self):
        contract = load_contract()
        for role in ("server-http", "server-stdio"):
            audit = contract.NetworkAudit(role=role)
            for event, arguments in (
                ("socket.connect", (None, ("127.0.0.1", 8765))),
                ("socket.getaddrinfo", ("localhost", 8765, 0, 0, 0)),
                ("socket.sendto", (None, ("127.0.0.1", 8765))),
                ("socket.gethostbyname", ("localhost",)),
            ):
                with self.assertRaises(PermissionError):
                    audit(event, arguments)
            self.assertEqual(audit.snapshot()["connects_allowed"], 0)
            self.assertEqual(len(audit.snapshot()["denials"]), 4)


if __name__ == "__main__":
    unittest.main()
