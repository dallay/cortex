"""Command line entry point for the Muninn memory benchmark harness."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Sequence

from . import __version__
from .compose import Topology
from .docker import DockerRunner
from .errors import BenchError
from .metrics import MODES
from .runner import DEFAULT_EVIDENCE_ROOT, Bench, RunOptions
from .scenario import load_scenario


def _split(value: str) -> tuple[str, ...]:
    return tuple(item.strip() for item in value.split(",") if item.strip())


def build_parser() -> argparse.ArgumentParser:
    """Build the argument parser."""

    parser = argparse.ArgumentParser(
        prog="membench",
        description=(
            "Run the Muninn memory benchmark: OpenCode in Docker against a disposable "
            "Muninn instance with a fixed synthetic fixture."
        ),
    )
    parser.add_argument("--version", action="version", version=__version__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    def add_common(target: argparse.ArgumentParser) -> None:
        target.add_argument("--model", required=True, help="model identifier, provider/model")
        target.add_argument("--scenario", required=True, type=Path, help="scenario file")
        target.add_argument(
            "--evidence-root",
            type=Path,
            default=DEFAULT_EVIDENCE_ROOT,
            help="directory that receives the evidence of the run",
        )
        target.add_argument(
            "--embedding-provider",
            default="minilm",
            choices=("minilm", "hash", "none"),
            help="embedding provider used by Muninn during the run",
        )
        target.add_argument("--embedding-bundle-path", type=Path, default=None)
        target.add_argument("--embedding-bundle-id", default=None)
        target.add_argument("--timeout", type=float, default=900.0)

    plan = subparsers.add_parser("plan", help="print the run plan without touching Docker")
    plan.add_argument("--scenario", required=True, type=Path)
    plan.add_argument("--repetitions", type=int, default=5)
    plan.add_argument("--modes", default=",".join(MODES))
    plan.add_argument("--conditions", default="C0,C1,C2")

    preflight = subparsers.add_parser(
        "preflight", help="verify the environment without running an inference"
    )
    add_common(preflight)
    preflight.add_argument("--repetitions", type=int, default=1)
    preflight.add_argument("--modes", default=",".join(MODES))
    preflight.add_argument("--conditions", default="C0,C1,C2")

    run = subparsers.add_parser("run", help="execute the benchmark")
    add_common(run)
    run.add_argument("--repetitions", type=int, default=5)
    run.add_argument("--modes", default=",".join(MODES))
    run.add_argument("--conditions", default="C0,C1,C2")
    run.add_argument(
        "--authorize-inference",
        action="store_true",
        help="required: authorizes the model inferences of this run",
    )
    run.add_argument(
        "--keep-containers",
        action="store_true",
        help="keep the disposable containers after the run for inspection",
    )

    auth = subparsers.add_parser("auth", help="manage the provider credentials of the runner")
    auth_sub = auth.add_subparsers(dest="auth_command", required=True)
    auth_import = auth_sub.add_parser("import", help="import an OpenCode credential export")
    auth_import.add_argument("--file", required=True, type=Path)
    return parser


def _options(arguments: argparse.Namespace) -> RunOptions:
    return RunOptions(
        model=arguments.model,
        scenario_path=arguments.scenario,
        evidence_root=arguments.evidence_root,
        repetitions=arguments.repetitions,
        timeout=arguments.timeout,
        modes=_split(arguments.modes),
        conditions=_split(arguments.conditions),
        embedding_provider=arguments.embedding_provider,
        embedding_bundle_path=arguments.embedding_bundle_path,
        embedding_bundle_id=arguments.embedding_bundle_id,
        authorize_inference=bool(getattr(arguments, "authorize_inference", False)),
        keep_containers=bool(getattr(arguments, "keep_containers", False)),
    )


def command_plan(arguments: argparse.Namespace) -> int:
    """Print the run plan."""

    scenario = load_scenario(arguments.scenario)
    options = RunOptions(
        model="provider/model",
        scenario_path=arguments.scenario,
        repetitions=arguments.repetitions,
        modes=_split(arguments.modes),
        conditions=_split(arguments.conditions),
    )
    bench = Bench(options, scenario)
    entries = bench.plan()
    print(json.dumps({"runs": len(entries), "entries": entries}, indent=2))
    return 0


def command_preflight(arguments: argparse.Namespace) -> int:
    """Run the preflight checks."""

    scenario = load_scenario(arguments.scenario)
    bench = Bench(_options(arguments), scenario)
    bench.prepare()
    try:
        preflight = bench.preflight_static()
        for check in preflight.checks:
            print(f"[{check.state}] {check.name}: {check.detail}")
        if not preflight.ok:
            print("preflight failed; the run stops here")
            return 1
        bench.start()
        before = len(preflight.checks)
        preflight = bench.preflight_runtime()
        for check in preflight.checks[before:]:
            print(f"[{check.state}] {check.name}: {check.detail}")
        if not preflight.ok:
            print("preflight failed; the run stops here")
            return 1
        print("preflight passed; no inference was executed")
        return 0
    finally:
        bench.stop()
        bench.cleanup()


def command_run(arguments: argparse.Namespace) -> int:
    """Execute the benchmark."""

    scenario = load_scenario(arguments.scenario)
    options = _options(arguments)
    bench = Bench(options, scenario)
    bench.prepare()
    entries = bench.plan()
    if not options.authorize_inference:
        print(
            "The plan contains "
            f"{len(entries)} runs, each of which performs a model inference. "
            "Re-run with --authorize-inference to execute them."
        )
        return 2
    try:
        preflight = bench.preflight_static()
        for check in preflight.checks:
            print(f"[{check.state}] {check.name}: {check.detail}")
        if not preflight.ok:
            print("preflight failed; the run stops here")
            return 1
        bench.start()
        before = len(preflight.checks)
        preflight = bench.preflight_runtime()
        for check in preflight.checks[before:]:
            print(f"[{check.state}] {check.name}: {check.detail}")
        if not preflight.ok:
            print("preflight failed; the run stops here")
            return 1
        results, manifest = bench.execute()
        evidence = bench.write_evidence(results, manifest)
        print(f"evidence written to {evidence}")
        return 0
    finally:
        bench.stop()
        bench.cleanup()


def command_auth(arguments: argparse.Namespace) -> int:
    """Import provider credentials into the runner's auth volume."""

    if arguments.auth_command != "import":
        return 2
    if not arguments.file.is_file():
        print(f"credential file not found: {arguments.file}")
        return 1
    compose_dir = Path(__file__).resolve().parent.parent
    topology = Topology(
        project="membench-auth",
        compose_dir=compose_dir,
        run_dir=Path("/tmp"),
        scenario_file=compose_dir / "scenarios" / "synthetic-memory-recall-v2.json",
        config_dir=Path("/tmp"),
        env_file=Path("/tmp/bench.env"),
    )
    if not topology.env_file.is_file():
        topology.env_file.write_text("BENCH_NEO4J_PASSWORD=unused\n", encoding="utf-8")
    runner = DockerRunner(topology)
    result = runner.compose_run(
        "run",
        "--rm",
        "-T",
        "-v",
        f"{arguments.file}:/tmp/auth.json:ro",
        "-e",
        "HOME=/opencode/auth",
        "--entrypoint",
        "opencode",
        "runner",
        "auth",
        "import",
        "/tmp/auth.json",
        timeout=300.0,
    )
    if not result.ok:
        print(result.summary())
        return 1
    print("credentials imported into the runner auth volume")
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    """Run the command line interface."""

    parser = build_parser()
    arguments = parser.parse_args(list(argv) if argv is not None else None)
    try:
        if arguments.command == "plan":
            return command_plan(arguments)
        if arguments.command == "preflight":
            return command_preflight(arguments)
        if arguments.command == "run":
            return command_run(arguments)
        if arguments.command == "auth":
            return command_auth(arguments)
    except BenchError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    parser.error(f"unknown command '{arguments.command}'")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
