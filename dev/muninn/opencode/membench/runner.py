"""Orchestration of one benchmark run.

The runner owns the lifecycle: disposable database, seeded fixture, isolated
agent sessions, per-run evidence, and an aggregate report. It never scores
memory quality in general; it reports what happened in this scenario.
"""

from __future__ import annotations

import os
import secrets
import shutil
import tempfile
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Mapping, Sequence

from . import config as bench_config
from .compose import Topology, validate
from .docker import DockerRunner
from .errors import BenchError
from .events import parse_events
from .manifest import render_report, utc_stamp, write_json, write_text
from .mcpclient import McpStdioClient
from .metrics import (
    CONDITION_EMPTY,
    CONDITION_MEMORY,
    CONDITION_ORACLE,
    MODE_AUTONOMOUS,
    MODE_DIRECT,
    MODE_FORCED,
    MODE_ORACLE,
    RunResult,
    diagnose_case,
    evaluate_agent_run,
    evaluate_direct_retrieval,
    extract_hits,
    summarize,
)
from .preflight import Preflight, run_runtime_preflight, run_static_preflight
from .scenario import Case, Scenario

DEFAULT_EVIDENCE_ROOT = Path(".agents/rpi/plan/tasks/opencode-docker-muninn-evaluation/evidence")
SEMANTIC_PROVIDERS = ("minilm",)


@dataclass
class RunOptions:
    """Everything the operator controls for one run."""

    model: str
    scenario_path: Path
    evidence_root: Path = DEFAULT_EVIDENCE_ROOT
    repetitions: int = 5
    timeout: float = 900.0
    modes: tuple[str, ...] = (MODE_DIRECT, MODE_FORCED, MODE_AUTONOMOUS, MODE_ORACLE)
    conditions: tuple[str, ...] = (CONDITION_EMPTY, CONDITION_MEMORY, CONDITION_ORACLE)
    embedding_provider: str = "minilm"
    embedding_bundle_path: Path | None = None
    embedding_bundle_id: str | None = None
    authorize_inference: bool = False
    keep_containers: bool = False


@dataclass
class RunContext:
    """Mutable state of one run."""

    run_id: str = ""
    evidence_dir: Path = field(default_factory=Path)
    secret_dir: Path = field(default_factory=Path)
    config_dir: Path = field(default_factory=Path)
    seed_plan: Path = field(default_factory=Path)


def _scope_for(case: Case, condition: str) -> str | None:
    """Return the owner scope that a condition exposes to the agent."""

    if condition == CONDITION_EMPTY:
        return case.empty_owner
    if condition == CONDITION_MEMORY:
        return case.owner
    if condition == CONDITION_ORACLE:
        return None
    raise BenchError(f"unknown condition '{condition}'")


def _modes_for(condition: str, modes: Sequence[str]) -> tuple[str, ...]:
    """Return the modes that apply to a condition."""

    if condition == CONDITION_ORACLE:
        return tuple(mode for mode in modes if mode == MODE_ORACLE)
    return tuple(mode for mode in modes if mode != MODE_ORACLE)


class Bench:
    """Runs one benchmark scenario."""

    def __init__(self, options: RunOptions, scenario: Scenario):
        self.options = options
        self.scenario = scenario
        self.context = RunContext()
        self.topology: Topology | None = None
        self.runner: DockerRunner | None = None
        self.preflight = Preflight()

    # ------------------------------------------------------------------
    # preparation
    # ------------------------------------------------------------------
    def prepare(self) -> None:
        """Create the run directories, the secret file, and the configuration."""

        if self.options.embedding_provider not in SEMANTIC_PROVIDERS:
            semantic_cases = [case.id for case in self.scenario.cases if case.requires_semantic]
            if semantic_cases:
                raise BenchError(
                    "the embedding provider '"
                    + self.options.embedding_provider
                    + "' is not semantic; these cases cannot be evaluated: "
                    + ", ".join(semantic_cases)
                )
        if self.options.embedding_provider in SEMANTIC_PROVIDERS:
            if self.options.embedding_bundle_path is None:
                raise BenchError(
                    "a semantic embedder requires --embedding-bundle-path and --embedding-bundle-id"
                )
            if not self.options.embedding_bundle_path.is_dir():
                raise BenchError(
                    f"embedding bundle not found: {self.options.embedding_bundle_path}"
                )

        stamp = utc_stamp()
        self.context.run_id = f"{self.scenario.version}-{stamp}-{secrets.token_hex(3)}"
        self.context.evidence_dir = self.options.evidence_root / self.context.run_id
        self.context.evidence_dir.mkdir(parents=True, exist_ok=True)
        self.context.secret_dir = Path(tempfile.mkdtemp(prefix="membench-"))
        self.context.config_dir = self.context.secret_dir / "config"
        self.context.config_dir.mkdir(parents=True, exist_ok=True)

        password = secrets.token_urlsafe(24)
        env_file = self.context.secret_dir / "bench.env"
        env_file.write_text(
            f"BENCH_NEO4J_PASSWORD={password}\n"
            f"BENCH_UID={os.getuid()}\n"
            f"BENCH_GID={os.getgid()}\n",
            encoding="utf-8",
        )
        env_file.chmod(0o600)

        self.context.seed_plan = self.context.secret_dir / "seed-plan.json"
        write_json(self.context.seed_plan, self._seed_plan())
        # The container reads the fixture and the rendered configuration over
        # bind mounts, so they must be readable by the container user.
        self.context.seed_plan.chmod(0o644)
        self.context.config_dir.chmod(0o755)

        compose_dir = Path(__file__).resolve().parent.parent
        self.topology = Topology(
            project=f"membench-{self.context.run_id}".lower().replace("_", "-")[:60],
            compose_dir=compose_dir,
            run_dir=self.context.secret_dir,
            scenario_file=self.options.scenario_path.resolve(),
            config_dir=self.context.config_dir,
            env_file=env_file,
            owner_scope=self.scenario.owner,
            embedding_provider=self.options.embedding_provider,
            embedding_bundle_path=self.options.embedding_bundle_path,
            embedding_bundle_id=self.options.embedding_bundle_id,
        )
        validate(self.topology)
        write_text(self.topology.compose_file(), self.topology.render())
        self.runner = DockerRunner(self.topology)

    def _seed_plan(self) -> dict[str, Any]:
        """Build the ordered seed plan for the scenario."""

        records: list[dict[str, Any]] = []
        supersedes: list[dict[str, int]] = []
        index_by_key: dict[tuple[str, str, str | None, str], int] = {}
        index_by_text: dict[str, int] = {}
        for case in self.scenario.cases:
            for record in case.seed:
                key = (record.kind, record.owner, record.category, record.text)
                if key in index_by_key:
                    index = index_by_key[key]
                else:
                    index = len(records)
                    index_by_key[key] = index
                    index_by_text.setdefault(record.text, index)
                    records.append(
                        {
                            "index": index,
                            "kind": record.kind,
                            "owner": record.owner,
                            "category": record.category,
                            "text": record.text,
                            "session_id": record.session_id or f"bench-session-{index}",
                        }
                    )
                if record.supersedes:
                    old_index = index_by_text.get(record.supersedes)
                    if old_index is None:
                        raise BenchError(
                            f"case '{case.id}' supersedes a record that is not part of the fixture"
                        )
                    supersedes.append({"old_index": old_index, "new_index": index})
        return {"records": records, "supersedes": supersedes}

    # ------------------------------------------------------------------
    # lifecycle
    # ------------------------------------------------------------------
    def start(self) -> None:
        """Start the disposable database and the runner container."""

        assert self.runner is not None
        result = self.runner.compose_run("up", "-d", "--wait", "neo4j", "runner", timeout=900.0)
        if not result.ok:
            raise BenchError(f"could not start the containers: {result.summary()}")

    def stop(self) -> None:
        """Remove the containers created by this run."""

        if self.runner is None:
            return
        if self.options.keep_containers:
            return
        self.runner.compose_run("down", "--volumes", "--remove-orphans", timeout=300.0)

    def cleanup(self) -> None:
        """Remove the temporary secret directory."""

        if self.context.secret_dir.is_dir():
            shutil.rmtree(self.context.secret_dir, ignore_errors=True)

    def preflight_static(self) -> Preflight:
        """Run the checks that do not need a running container."""

        assert self.runner is not None and self.topology is not None
        self.preflight = run_static_preflight(self.runner, self.topology)
        return self.preflight

    def preflight_runtime(self) -> Preflight:
        """Run the checks that need the running containers."""

        assert self.runner is not None and self.topology is not None
        self.preflight = run_runtime_preflight(
            self.runner,
            self.topology,
            self.options.model,
            self.scenario.cases[0].owner,
            self.preflight,
        )
        return self.preflight

    def preflight_run(self) -> Preflight:
        """Run every preflight check."""

        self.preflight_static()
        if self.preflight.failures:
            return self.preflight
        return self.preflight_runtime()

    def seed(self) -> None:
        """Seed the disposable database with the scenario records."""

        assert self.runner is not None and self.topology is not None
        result = self.runner.compose_run(
            "run",
            "--rm",
            "-T",
            "-e",
            f"BENCH_SEED_PLAN=/run-seed/seed-plan.json",
            "seed",
            timeout=600.0,
        )
        if not result.ok:
            raise BenchError(f"seeding failed: {result.summary()}")

    # ------------------------------------------------------------------
    # execution
    # ------------------------------------------------------------------
    def _render_condition_config(self, label: str, scope: str | None) -> Path:
        assert self.context.config_dir is not None
        target = self.context.config_dir / label
        target.mkdir(parents=True, exist_ok=True)
        rendered = bench_config.render_config(
            self.options.model, scope or "", mcp_enabled=scope is not None
        )
        bench_config.write_config(rendered, target)
        return target

    def _run_agent(
        self,
        case: Case,
        mode: str,
        condition: str,
        repetition: int,
        prompt: str,
        scope: str | None,
    ) -> RunResult:
        """Run one OpenCode session and evaluate it."""

        assert self.runner is not None
        label = f"{case.id}-{mode}-{condition}-{repetition}"
        config_target = self._render_condition_config(label, scope)
        home = f"/run/home/{label}"
        env = {
            "HOME": home,
            "OPENCODE_DB": f"{home}/opencode.db",
            "BENCH_PROMPT": prompt,
            "BENCH_MODEL": self.options.model,
        }
        script = (
            "mkdir -p \"$HOME/.local/share/opencode\"; "
            "cp -a /opencode/auth/. \"$HOME/.local/share/opencode/\" 2>/dev/null || true; "
            "cd /run-config/config/" + label + " && "
            'exec opencode run --standalone --format json --archived '
            '--agent "$BENCH_AGENT" --model "$BENCH_MODEL" "$BENCH_PROMPT"'
        )
        env["BENCH_AGENT"] = bench_config.AGENT_NAME
        started = time.monotonic()
        result = self.runner.compose_exec(
            "sh",
            "-c",
            script,
            env=env,
            timeout=self.options.timeout,
        )
        latency_ms = int((time.monotonic() - started) * 1000)
        trace = parse_events(result.stdout)
        write_json(
            self.context.evidence_dir / "runs" / f"{label}-events.json",
            {
                "case_id": case.id,
                "mode": mode,
                "condition": condition,
                "repetition": repetition,
                "returncode": result.returncode,
                "trace": trace.to_manifest(),
                "stderr": result.stderr[-4000:],
            },
        )
        if result.returncode != 0 and not trace.tool_calls and not trace.text:
            trace.errors.append(f"opencode exited with status {result.returncode}")
        return evaluate_agent_run(
            case, mode, condition, repetition, trace, latency_ms=latency_ms
        )

    def _run_direct(self, case: Case, condition: str, repetition: int, scope: str) -> RunResult:
        """Query the MCP server directly, without a model."""

        assert self.runner is not None
        argv = self.runner.compose(
            "exec",
            "-T",
            "runner",
            "/usr/local/bin/muninn",
            "mcp",
            "--owner-scope",
            scope,
        )
        started = time.monotonic()
        try:
            with McpStdioClient(argv, timeout=self.options.timeout) as client:
                client.initialize()
                payload = client.call_tool(case.direct_tool, dict(case.direct_arguments))
        except BenchError as error:
            latency_ms = int((time.monotonic() - started) * 1000)
            return evaluate_direct_retrieval(
                case, condition, repetition, [], latency_ms=latency_ms, error=str(error)
            )
        latency_ms = int((time.monotonic() - started) * 1000)
        hits = extract_hits(_tool_text(payload))
        write_json(
            self.context.evidence_dir / "runs" / f"{case.id}-{MODE_DIRECT}-{condition}-{repetition}-direct.json",
            {"case_id": case.id, "condition": condition, "hit_count": len(hits)},
        )
        return evaluate_direct_retrieval(case, condition, repetition, hits, latency_ms=latency_ms)

    # ------------------------------------------------------------------
    # plan
    # ------------------------------------------------------------------
    def plan(self) -> list[dict[str, Any]]:
        """Describe the runs this invocation would perform."""

        entries: list[dict[str, Any]] = []
        for case in self.scenario.cases:
            for condition in self.options.conditions:
                for mode in _modes_for(condition, self.options.modes):
                    for repetition in range(1, self.options.repetitions + 1):
                        entries.append(
                            {
                                "case_id": case.id,
                                "mode": mode,
                                "condition": condition,
                                "repetition": repetition,
                            }
                        )
        return entries

    def execute(self) -> tuple[list[RunResult], dict[str, Any]]:
        """Execute the planned runs and return the results and the manifest."""

        assert self.runner is not None and self.topology is not None
        results: list[RunResult] = []
        repetitions = self.options.repetitions
        for repetition in range(1, repetitions + 1):
            # Each repetition starts from a fresh database and a fresh fixture.
            self.runner.compose_run("up", "-d", "--force-recreate", "--wait", "neo4j", timeout=900.0)
            self.seed()
            for case in self.scenario.cases:
                for condition in self.options.conditions:
                    scope = _scope_for(case, condition)
                    for mode in _modes_for(condition, self.options.modes):
                        if mode == MODE_DIRECT:
                            assert scope is not None
                            results.append(
                                self._run_direct(case, condition, repetition, scope)
                            )
                            continue
                        if mode == MODE_ORACLE:
                            prompt = self._oracle_prompt(case)
                            results.append(
                                self._run_agent(
                                    case, mode, CONDITION_ORACLE, repetition, prompt, None
                                )
                            )
                            continue
                        if mode == MODE_FORCED:
                            prompt = bench_config.forced_tool_prompt(
                                case.prompt, case.direct_tool, dict(case.direct_arguments)
                            )
                        else:
                            prompt = case.prompt
                        assert scope is not None
                        results.append(
                            self._run_agent(case, mode, condition, repetition, prompt, scope)
                        )
        manifest = self._manifest(results)
        return results, manifest

    def _oracle_prompt(self, case: Case) -> str:
        return (
            f"{case.prompt}\n\n"
            "The stored information is provided here directly:\n"
            f"{case.oracle_context}"
        )

    def _manifest(self, results: Sequence[RunResult]) -> dict[str, Any]:
        assert self.topology is not None
        diagnoses: dict[str, str] = {}
        for case in self.scenario.cases:
            case_results = [result for result in results if result.case_id == case.id]
            diagnoses[case.id] = diagnose_case(case_results)
        return {
            "run_id": self.context.run_id,
            "scenario_version": self.scenario.version,
            "scenario_path": str(self.options.scenario_path),
            "model": self.options.model,
            "repetitions": self.options.repetitions,
            "runner_image": self.topology.runner_image,
            "opencode_image": self.topology.opencode_image,
            "neo4j_image": self.topology.neo4j_image,
            "embedding_provider": self.options.embedding_provider,
            "owner_scope": self.scenario.owner,
            "empty_owner_scope": self.scenario.empty_owner,
            "preflight": self.preflight.to_manifest(),
            "summary": summarize(results),
            "case_diagnosis": diagnoses,
            "results": [result.to_manifest() for result in results],
            "limits": (
                "Results describe this synthetic scenario only. They are not a parity score "
                "and not evidence of global parity with any other memory system."
            ),
        }

    def write_evidence(self, results: Sequence[RunResult], manifest: Mapping[str, Any]) -> Path:
        """Write the manifest, the report, and the sanitized evidence."""

        write_json(self.context.evidence_dir / "run-manifest.json", manifest)
        write_text(
            self.context.evidence_dir / "report.md",
            render_report(manifest, [result.to_manifest() for result in results]),
        )
        return self.context.evidence_dir


def _tool_text(payload: Any) -> Any:
    """Extract the textual payload of an MCP tool response."""

    if isinstance(payload, Mapping):
        content = payload.get("content")
        if isinstance(content, list):
            texts = [
                item.get("text")
                for item in content
                if isinstance(item, Mapping) and isinstance(item.get("text"), str)
            ]
            if texts:
                return "\n".join(texts)
        return payload
    return payload
