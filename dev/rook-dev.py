#!/usr/bin/env python3
"""Run the Rook backend and dashboard together for local development."""

from __future__ import annotations

import asyncio
import os
import shutil
import signal
import socket
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


async def stop_process_group(process: asyncio.subprocess.Process) -> None:
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            await process.wait()
            return
        try:
            await asyncio.wait_for(process.wait(), timeout=SHUTDOWN_TIMEOUT)
        except asyncio.TimeoutError:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                # El grupo de procesos ya terminó antes de enviar SIGKILL.
                pass
            await process.wait()
            return

        deadline = asyncio.get_running_loop().time() + SHUTDOWN_TIMEOUT
        while asyncio.get_running_loop().time() < deadline:
            try:
                os.killpg(process.pid, 0)
            except ProcessLookupError:
                return
            await asyncio.sleep(0.05)
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            # El grupo de procesos terminó durante la espera activa.
            pass
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
    waiters: dict[asyncio.Task[int], str] = {}
    stop_waiter: asyncio.Task[bool] | None = None
    try:
        backend = await asyncio.create_subprocess_exec(
            "cargo",
            "watch",
            "--no-process-group",
            "-x",
            "run",
            cwd=ROOK_DIR,
            start_new_session=os.name == "posix",
        )
        children.append(("Rook backend", backend))
        dashboard = await asyncio.create_subprocess_exec(
            "pnpm",
            "exec",
            "vite",
            "--strictPort",
            cwd=DASHBOARD_DIR,
            start_new_session=os.name == "posix",
        )
        children.append(("Dashboard", dashboard))
        print(
            "Rook: http://127.0.0.1:3773 | Dashboard: http://localhost:4747/dashboard/",
            flush=True,
        )

        waiters = {
            asyncio.create_task(process.wait()): name
            for name, process in children
        }
        stop_waiter = asyncio.create_task(stop_event.wait())
        done, _ = await asyncio.wait(
            [*waiters, stop_waiter], return_when=asyncio.FIRST_COMPLETED
        )
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
            *(stop_process_group(process) for _, process in children),
            return_exceptions=True,
        )
        await asyncio.gather(*waiters, return_exceptions=True)


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
