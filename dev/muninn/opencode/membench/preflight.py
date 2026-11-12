"""Preflight checks.

The preflight proves the environment is usable without running an inference.
Each check reports what it actually proved. A model that appears in the
catalog is not a model that will answer, so the states stay deliberately
narrow: ``CATALOG_VERIFIED`` never means "quota available".
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .compose import Topology, validate_rendered
from .docker import DockerRunner
from .errors import BenchError
from .mcpclient import McpStdioClient

PASS = "PASS"
FAIL = "FAIL"
NOT_TESTED = "NOT_TESTED"
CATALOG_VERIFIED = "CATALOG_VERIFIED"
AUTH_CONFIGURED = "AUTH_CONFIGURED"


@dataclass
class Check:
    """One preflight result."""

    name: str
    state: str
    detail: str = ""
    blocking: bool = True

    def to_manifest(self) -> dict[str, Any]:
        return {
            "name": self.name,
            "state": self.state,
            "detail": self.detail,
            "blocking": self.blocking,
        }


@dataclass
class Preflight:
    """The collected preflight results."""

    checks: list[Check] = field(default_factory=list)

    def add(self, name: str, state: str, detail: str = "", blocking: bool = True) -> None:
        self.checks.append(Check(name=name, state=state, detail=detail, blocking=blocking))

    @property
    def failures(self) -> list[Check]:
        return [check for check in self.checks if check.blocking and check.state == FAIL]

    @property
    def ok(self) -> bool:
        return not self.failures

    def to_manifest(self) -> list[dict[str, Any]]:
        return [check.to_manifest() for check in self.checks]


def model_in_catalog(catalog_output: str, model: str) -> bool:
    """Check whether the model identifier appears in the catalog output."""

    wanted = model.strip().lower()
    if not wanted:
        return False
    for line in catalog_output.splitlines():
        stripped = line.strip().lower()
        if not stripped:
            continue
        if stripped == wanted or stripped.startswith(wanted + " "):
            return True
        if stripped.startswith(wanted + "#"):
            return True
    return False


def auth_providers(auth_output: str) -> list[str]:
    """Extract provider names from ``opencode auth list`` output.

    Only the provider names are returned; credential material is discarded.
    """

    providers: list[str] = []
    for line in auth_output.splitlines():
        stripped = line.strip().lstrip("●").strip()
        if not stripped or stripped.lower().startswith("credentials"):
            continue
        if ":" in stripped:
            name = stripped.split(":", 1)[0].strip()
        else:
            name = stripped.split()[0]
        if name and name not in providers:
            providers.append(name)
    return providers


def check_docker(runner: DockerRunner, preflight: Preflight) -> None:
    """Verify that the Docker daemon is reachable."""

    result = runner.run(["docker", "info", "--format", "{{.ServerVersion}}"], timeout=60.0)
    if result.ok and result.stdout.strip():
        preflight.add("DOCKER_DAEMON", PASS, f"server version {result.stdout.strip()}")
    else:
        preflight.add("DOCKER_DAEMON", FAIL, result.summary() or "the daemon is not reachable")


def check_images(runner: DockerRunner, topology: Topology, preflight: Preflight) -> None:
    """Verify that the pinned images are present locally."""

    missing = [
        image
        for image in (topology.runner_image, topology.opencode_image, topology.neo4j_image)
        if not runner.image_exists(image)
    ]
    if missing:
        preflight.add(
            "IMAGES",
            FAIL,
            "missing local images: " + ", ".join(missing) + " (build or pull them first)",
        )
    else:
        preflight.add("IMAGES", PASS, "runner, OpenCode, and Neo4j images are present")


def check_topology(topology: Topology, preflight: Preflight) -> None:
    """Validate the rendered container topology."""

    violations = validate_rendered(topology.render())
    if violations:
        preflight.add("TOPOLOGY", FAIL, "; ".join(violations))
    else:
        preflight.add(
            "TOPOLOGY",
            PASS,
            "Neo4j is internal-only, no host ports are published, no host paths are mounted",
        )


def check_model_catalog(runner: DockerRunner, topology: Topology, model: str, preflight: Preflight) -> None:
    """Verify that OpenCode recognises the requested model identifier."""

    result = runner.compose_exec(
        "opencode",
        "models",
        env={"HOME": "/run/home/preflight", "OPENCODE_DB": "/run/home/preflight/opencode.db"},
        timeout=180.0,
    )
    if not result.ok:
        preflight.add("MODEL_CATALOG", FAIL, result.summary())
        return
    if model_in_catalog(result.stdout, model):
        preflight.add(
            "MODEL_CATALOG",
            CATALOG_VERIFIED,
            f"'{model}' appears in the catalog; availability and quota are not proven",
        )
    else:
        preflight.add(
            "MODEL_CATALOG",
            FAIL,
            f"'{model}' does not appear in the catalog; the run stops instead of switching models",
        )


def check_auth(runner: DockerRunner, topology: Topology, preflight: Preflight) -> None:
    """Verify that at least one provider credential is configured."""

    result = runner.compose_exec(
        "opencode",
        "auth",
        "list",
        env={"HOME": "/run/home/preflight", "OPENCODE_DB": "/run/home/preflight/opencode.db"},
        timeout=180.0,
    )
    if not result.ok:
        preflight.add("AUTH_CONFIGURED", FAIL, result.summary())
        return
    providers = auth_providers(result.stdout)
    if providers:
        preflight.add(
            "AUTH_CONFIGURED",
            AUTH_CONFIGURED,
            "providers: " + ", ".join(providers) + "; a real request is not proven",
        )
    else:
        preflight.add("AUTH_CONFIGURED", FAIL, "no provider credential is configured")


def check_mcp(runner: DockerRunner, topology: Topology, owner_scope: str, preflight: Preflight) -> None:
    """Verify the MCP handshake and a retrieval round trip inside the container."""

    argv = runner.compose(
        "exec",
        "-T",
        "runner",
        "/usr/local/bin/muninn",
        "mcp",
        "--owner-scope",
        owner_scope,
    )
    try:
        with McpStdioClient(argv, timeout=120.0) as client:
            client.initialize()
            tools = client.list_tools()
    except BenchError as error:
        preflight.add("MCP_HANDSHAKE", FAIL, str(error))
        return
    names = [tool.name for tool in tools]
    if not names:
        preflight.add("MCP_HANDSHAKE", FAIL, "the server advertised no tools")
        return
    preflight.add("MCP_HANDSHAKE", PASS, "tools: " + ", ".join(names))
    try:
        with McpStdioClient(argv, timeout=120.0) as client:
            client.initialize()
            result = client.call_tool(
                "muninn_search_preferences",
                {"query": "preflight connectivity probe", "limit": 1},
            )
    except BenchError as error:
        preflight.add("MCP_QUERY_ROUNDTRIP", FAIL, str(error))
        return
    preflight.add(
        "MCP_QUERY_ROUNDTRIP",
        PASS,
        "the retrieval tool answered against the disposable database",
    )


def run_static_preflight(runner: DockerRunner, topology: Topology) -> Preflight:
    """Run the checks that do not need a running container."""

    preflight = Preflight()
    check_docker(runner, preflight)
    check_images(runner, topology, preflight)
    check_topology(topology, preflight)
    return preflight


def run_runtime_preflight(
    runner: DockerRunner,
    topology: Topology,
    model: str,
    owner_scope: str,
    preflight: Preflight,
) -> Preflight:
    """Run the checks that need the running containers."""

    check_model_catalog(runner, topology, model, preflight)
    check_auth(runner, topology, preflight)
    check_mcp(runner, topology, owner_scope, preflight)
    preflight.add(
        "MODEL_INFERENCE",
        NOT_TESTED,
        "no inference runs during the preflight; a listed model does not prove a request will succeed",
        blocking=False,
    )
    return preflight


def run_preflight(
    runner: DockerRunner,
    topology: Topology,
    model: str,
    owner_scope: str,
) -> Preflight:
    """Execute every preflight check and return the collected results."""

    preflight = run_static_preflight(runner, topology)
    if preflight.failures:
        return preflight
    return run_runtime_preflight(runner, topology, model, owner_scope, preflight)
