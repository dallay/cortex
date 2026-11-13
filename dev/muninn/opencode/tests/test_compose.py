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


def bundle_topology() -> Topology:
    """Build a topology that carries a pinned semantic bundle."""

    return Topology(
        project="membench-test",
        compose_dir=Path("/tmp"),
        run_dir=Path("/tmp"),
        scenario_file=Path("/tmp/scenario.json"),
        config_dir=Path("/tmp"),
        env_file=Path("/tmp/bench.env"),
        embedding_provider="minilm",
        embedding_bundle_path=Path("/host/cache/ee963db1"),
        embedding_bundle_id="ee963db1",
    )


def bundle_mount_target(rendered: str) -> str:
    """Return the container path where the bundle volume is mounted."""

    for line in rendered.splitlines():
        if "minilm-bundle:" in line and ":ro" in line:
            return line.split(":", 2)[1]
    raise AssertionError("the Compose file does not mount the bundle volume")


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

    def test_the_credential_directory_is_mounted_read_only(self) -> None:
        # The credential lives in a host directory owned by the operator, so
        # the container user can read it without a privileged volume.
        rendered = topology().render()
        self.assertIn(":/opencode/auth:ro", rendered)

    def test_docker_socket_is_rejected(self) -> None:
        self.assertTrue(validate_rendered("volumes:\n  - /var/run/docker.sock:/var/run/docker.sock"))

    def test_host_home_is_rejected(self) -> None:
        self.assertTrue(validate_rendered("volumes:\n  - /home/dev/.opencode:/home/x"))

    def test_published_ports_are_rejected(self) -> None:
        self.assertTrue(validate_rendered("  neo4j:\n    ports:\n      - 7474:7474"))


class BundleTopologyTests(unittest.TestCase):
    def test_the_bundle_is_not_mounted_from_the_host(self) -> None:
        # A bind mount from a macOS host is a virtual filesystem and the
        # artifact contract rejects it, so the bundle travels in a volume.
        rendered = bundle_topology().render()
        self.assertNotIn("/host/cache", rendered)
        self.assertIn("minilm-bundle:/bundle:ro", rendered)

    def test_the_bundle_path_is_the_mounted_root(self) -> None:
        # The contract opens the root and then the bundle directory inside
        # it, so the path must be the mount point and not the bundle itself.
        rendered = bundle_topology().render()
        target = bundle_mount_target(rendered)
        self.assertIn(f"NAM_EMBEDDING__BUNDLE_PATH: {target}", rendered)

    def test_every_service_that_loads_the_bundle_shares_the_same_path(self) -> None:
        rendered = bundle_topology().render()
        self.assertEqual(rendered.count("NAM_EMBEDDING__BUNDLE_PATH:"), 2)
        self.assertEqual(rendered.count("minilm-bundle:/bundle:ro"), 2)

    def test_the_bundle_identifier_is_passed_to_the_container(self) -> None:
        rendered = bundle_topology().render()
        self.assertIn("NAM_EMBEDDING__BUNDLE_ID: ee963db1", rendered)


if __name__ == "__main__":
    unittest.main()
