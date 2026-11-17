"""Thin wrapper around the Docker CLI used by the harness."""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from typing import Mapping, Sequence

from .errors import BenchError


@dataclass(frozen=True)
class _ImageOnly:
    """A minimal topology stand-in for one-off container commands."""

    image: str


@dataclass(frozen=True)
class CommandResult:
    """The outcome of one command invocation."""

    argv: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str

    @property
    def ok(self) -> bool:
        return self.returncode == 0

    def summary(self, limit: int = 400) -> str:
        text = (self.stderr or self.stdout or "").strip()
        if len(text) > limit:
            text = text[:limit] + "..."
        return text.replace("\n", " ")


class DockerRunner:
    """Runs Docker commands for one benchmark topology."""

    def __init__(self, topology) -> None:  # type: ignore[no-untyped-def]
        self.topology = topology

    def compose(self, *args: str) -> list[str]:
        from .compose import compose_command

        return compose_command(self.topology, *args)

    @classmethod
    def for_image(cls, image: str) -> "DockerRunner":
        """Build a runner for one-off commands that do not need a topology."""

        return cls(_ImageOnly(image))

    @property
    def image(self) -> str:
        return getattr(self.topology, "image", "")

    def run(
        self,
        argv: Sequence[str],
        timeout: float = 900.0,
        stdin: str | None = None,
        env: Mapping[str, str] | None = None,
    ) -> CommandResult:
        """Run a command and capture its output."""

        try:
            completed = subprocess.run(  # noqa: S603 - argv is built by the harness
                list(argv),
                input=stdin,
                capture_output=True,
                text=True,
                timeout=timeout,
                env=None,
            )
        except subprocess.TimeoutExpired as error:
            return CommandResult(tuple(argv), 124, error.stdout or "", "timeout")
        except OSError as error:
            raise BenchError(f"could not run {argv[0]}: {error}") from error
        return CommandResult(tuple(argv), completed.returncode, completed.stdout, completed.stderr)

    def compose_run(self, *args: str, timeout: float = 900.0, stdin: str | None = None) -> CommandResult:
        return self.run(self.compose(*args), timeout=timeout, stdin=stdin)

    def compose_exec(
        self,
        *args: str,
        service: str = "runner",
        env: Mapping[str, str] | None = None,
        timeout: float = 900.0,
        stdin: str | None = None,
    ) -> CommandResult:
        """Run a command inside a service of the topology."""

        argv = self.compose("exec", "-T")
        for key, value in (env or {}).items():
            argv.extend(["-e", f"{key}={value}"])
        argv.append(service)
        argv.extend(args)
        return self.run(argv, timeout=timeout, stdin=stdin)

    def image_exists(self, image: str) -> bool:
        result = self.run(["docker", "image", "inspect", image], timeout=60.0)
        return result.ok


HOME_PREPARE_SCRIPT = """
set -eu
mkdir -p "$HOME/.local/share/opencode"
# OpenCode keeps the provider credential in its SQLite store, so the whole
# state directory is copied into the ephemeral home of the run.
if [ -d /opencode/auth/.local/share/opencode ]; then
  cp -a /opencode/auth/.local/share/opencode/. "$HOME/.local/share/opencode/" 2>/dev/null || true
  chmod 700 "$HOME/.local/share/opencode"
fi
# OpenCode only registers the MCP servers declared in the global
# configuration, so it is copied into the home of the run.
if [ -n "${BENCH_CONFIG_DIR:-}" ] && [ -f "$BENCH_CONFIG_DIR/global/opencode.json" ]; then
  mkdir -p "$HOME/.config/opencode"
  cp "$BENCH_CONFIG_DIR/global/opencode.json" "$HOME/.config/opencode/opencode.json"
  chmod 600 "$HOME/.config/opencode/opencode.json"
fi
"""


def home_prepare_command(command: str) -> str:
    """Return a shell command that prepares the ephemeral home and runs a command.

    The credential lives in a host directory owned by the operator. Each run
    gets a fresh home on tmpfs; the OpenCode state directory that holds the
    credential is copied into it before the command starts, and the copy is
    discarded with the container.
    """

    return f"{HOME_PREPARE_SCRIPT}\n{command}"
