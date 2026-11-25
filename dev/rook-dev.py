#!/usr/bin/env python3
"""Run the Rook backend and dashboard together for local development."""

from __future__ import annotations

import asyncio
import os
import shutil
import signal
import socket
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ROOK_DIR = ROOT / "apps" / "rook"
DASHBOARD_DIR = ROOK_DIR / "dashboard"
SHUTDOWN_TIMEOUT = 5.0


def port_is_open(host: str, port: int) -> bool:
    try:
        with socket.create_connection((host, port), timeout=0.25):
            return True
    except OSError:
        return False


def preflight() -> bool:
    missing = [
        name
        for name in ("cargo", "cargo-watch", "pnpm")
        if shutil.which(name) is None
    ]
    if missing:
        print(
            f"Faltan herramientas de desarrollo: {', '.join(missing)}",
            file=sys.stderr,
        )
        return False

    occupied = [
        (label, host, port)
        for label, host, port in (
            ("Rook", "127.0.0.1", 3773),
            ("Dashboard", "localhost", 4747),
        )
        if port_is_open(host, port)
    ]
    if occupied:
        details = ", ".join(
            f"{label} ({host}:{port})" for label, host, port in occupied
        )
        print(
            f"Puertos ocupados: {details}. Detén esos procesos y vuelve a ejecutar just rook.",
            file=sys.stderr,
        )
        return False
    return True


def process_group_is_alive(process_group: int) -> bool:
    try:
        os.killpg(process_group, 0)
        return True
    except ProcessLookupError:
        return False


def process_groups_for_descendants(pid: int) -> set[int]:
    output = subprocess.check_output(
        ("ps", "-A", "-o", "pid=", "-o", "ppid=", "-o", "pgid="),
        text=True,
    )
    children: dict[int, list[tuple[int, int]]] = {}
    for line in output.splitlines():
        try:
            child_pid, parent_pid, process_group = map(int, line.split())
        except ValueError:
            continue
        children.setdefault(parent_pid, []).append((child_pid, process_group))

    process_groups: set[int] = set()
    descendants = [pid]
    seen = {pid}
    while descendants:
        parent_pid = descendants.pop()
        for child_pid, process_group in children.get(parent_pid, []):
            if child_pid in seen:
                continue
            seen.add(child_pid)
            descendants.append(child_pid)
            process_groups.add(process_group)
    return process_groups


async def stop_process_group(
    process: asyncio.subprocess.Process, tracked_process_groups: set[int] | None = None
) -> None:
    if os.name == "posix":
        process_groups = set(tracked_process_groups or set())
        watcher_stopped = False

        if process.returncode is None:
            try:
                os.killpg(process.pid, signal.SIGSTOP)
            except ProcessLookupError:
                pass
            else:
                watcher_stopped = True
                process_groups.add(process.pid)

        def discover_process_groups() -> set[int]:
            try:
                return process_groups_for_descendants(process.pid)
            except (OSError, subprocess.SubprocessError) as error:
                print(
                    f"No se pudieron identificar los grupos de procesos descendientes: {error}",
                    file=sys.stderr,
                )
                return set()

        def signal_process_groups(groups: set[int], signum: int) -> None:
            for process_group in groups:
                try:
                    os.killpg(process_group, signum)
                except ProcessLookupError:
                    pass

        def active_process_groups(groups: set[int]) -> set[int]:
            return {
                process_group
                for process_group in groups
                if process_group_is_alive(process_group)
            }

        process_groups.update(discover_process_groups())
        signal_process_groups(process_groups, signal.SIGTERM)
        if watcher_stopped:
            try:
                os.killpg(process.pid, signal.SIGCONT)
            except ProcessLookupError:
                pass
        try:
            await asyncio.wait_for(process.wait(), timeout=SHUTDOWN_TIMEOUT)
        except asyncio.TimeoutError:
            signal_process_groups(
                active_process_groups(process_groups), signal.SIGKILL
            )
            await process.wait()
            signal_process_groups(
                active_process_groups(discover_process_groups()), signal.SIGKILL
            )
            return

        late_groups = discover_process_groups() - process_groups
        process_groups.update(late_groups)
        signal_process_groups(late_groups, signal.SIGTERM)

        deadline = asyncio.get_running_loop().time() + SHUTDOWN_TIMEOUT
        while asyncio.get_running_loop().time() < deadline:
            process_groups = active_process_groups(process_groups)
            if not process_groups:
                return
            await asyncio.sleep(0.05)
        signal_process_groups(process_groups, signal.SIGKILL)
        return

    if process.returncode is not None:
        return
    process.terminate()
    try:
        await asyncio.wait_for(process.wait(), timeout=SHUTDOWN_TIMEOUT)
    except asyncio.TimeoutError:
        process.kill()
        await process.wait()


async def main() -> int:
    if not preflight():
        return 1

    stop_event = asyncio.Event()
    loop = asyncio.get_running_loop()
    if os.name == "posix":
        for signum in (signal.SIGINT, signal.SIGTERM):
            loop.add_signal_handler(signum, stop_event.set)

    children: list[tuple[str, asyncio.subprocess.Process]] = []
    tracked_process_groups: dict[int, set[int]] = {}
    waiters: dict[asyncio.Task[int], str] = {}
    stop_waiter: asyncio.Task[bool] | None = None
    try:
        backend = await asyncio.create_subprocess_exec(
            "cargo",
            "watch",
            "-x",
            "run",
            cwd=ROOK_DIR,
            start_new_session=os.name == "posix",
        )
        children.append(("Rook backend", backend))
        tracked_process_groups[backend.pid] = set()
        dashboard = await asyncio.create_subprocess_exec(
            "pnpm",
            "exec",
            "vite",
            "--strictPort",
            cwd=DASHBOARD_DIR,
            start_new_session=os.name == "posix",
        )
        children.append(("Dashboard", dashboard))
        tracked_process_groups[dashboard.pid] = set()
        print(
            "Rook: http://127.0.0.1:3773 | Dashboard: http://localhost:4747/dashboard/",
            flush=True,
        )

        waiters = {
            asyncio.create_task(process.wait()): name
            for name, process in children
        }
        stop_waiter = asyncio.create_task(stop_event.wait())
        while True:
            if os.name == "posix":
                for _, process in children:
                    try:
                        tracked_groups = tracked_process_groups[process.pid]
                        tracked_groups.intersection_update(
                            process_group
                            for process_group in tracked_groups.copy()
                            if process_group_is_alive(process_group)
                        )
                        tracked_process_groups[process.pid].update(
                            process_groups_for_descendants(process.pid)
                        )
                    except (OSError, subprocess.SubprocessError) as error:
                        print(
                            f"No se pudieron identificar los grupos de procesos descendientes: {error}",
                            file=sys.stderr,
                        )
            done, _ = await asyncio.wait(
                [*waiters, stop_waiter],
                timeout=0.25,
                return_when=asyncio.FIRST_COMPLETED,
            )
            if done:
                break
        if stop_waiter in done:
            return 0

        finished = next(task for task in done if task is not stop_waiter)
        name = waiters[finished]
        status = finished.result()
        print(
            f"{name} terminó con código {status}; se detendrá el otro proceso.",
            file=sys.stderr,
        )
        return status or 1
    except FileNotFoundError as error:
        print(f"No se pudo iniciar {error.filename}: {error}", file=sys.stderr)
        return 127
    finally:
        if stop_waiter is not None:
            stop_waiter.cancel()
            await asyncio.gather(stop_waiter, return_exceptions=True)
        await asyncio.gather(
            *(
                stop_process_group(
                    process, tracked_process_groups.get(process.pid, set())
                )
                for _, process in children
            ),
            return_exceptions=True,
        )
        await asyncio.gather(*waiters, return_exceptions=True)


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
