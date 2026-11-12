"""Tests for the container topology rules."""

from __future__ import annotations

import unittest
from pathlib import Path

from membench.compose import Topology, validate_rendered


def topology() -> Topology:
    """Build a topology with placeholder paths."""

    return Topology(
        project="membench-test",
        compose_dir=Path("/tmp"),
        run_dir=Path("/tmp"),
        scenario_file=Path("/tmp/scenario.json"),
        config_dir=Path("/tmp"),
        env_file=Path("/tmp/bench.env"),
    )


class TopologyTests(unittest.TestCase):
    def test_rendered_topology_has_no_violations(self) -> None:
        self.assertEqual(validate_rendered(topology().render()), [])

    def test_internal_network_is_internal(self) -> None:
        rendered = topology().render()
        self.assertIn("internal: true", rendered)

    def test_no_port_is_published(self) -> None:
        rendered = topology().render()
        self.assertNotIn("ports:", rendered)

    def test_neo4j_is_not_on_the_egress_network(self) -> None:
        rendered = topology().render()
        neo4j_block = rendered.split("  neo4j:", 1)[1].split("  seed:", 1)[0]
        self.assertNotIn("bench_egress", neo4j_block)

    def test_runner_joins_both_networks(self) -> None:
        rendered = topology().render()
        runner_block = rendered.split("  runner:", 1)[1].split("\nnetworks:", 1)[0]
        self.assertIn("bench_db", runner_block)
        self.assertIn("bench_egress", runner_block)

    def test_docker_socket_is_rejected(self) -> None:
        self.assertTrue(validate_rendered("volumes:\n  - /var/run/docker.sock:/var/run/docker.sock"))

    def test_host_home_is_rejected(self) -> None:
        self.assertTrue(validate_rendered("volumes:\n  - /home/dev/.opencode:/home/x"))

    def test_published_ports_are_rejected(self) -> None:
        self.assertTrue(validate_rendered("  neo4j:\n    ports:\n      - 7474:7474"))


if __name__ == "__main__":
    unittest.main()
