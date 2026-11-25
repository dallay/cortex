import importlib.util
import signal
import unittest
from pathlib import Path
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "rook_dev", Path(__file__).with_name("rook-dev.py")
)
rook_dev = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(rook_dev)


class FakeProcess:
    def __init__(self, pid: int) -> None:
        self.pid = pid
        self.returncode = None

    async def wait(self) -> int:
        self.returncode = 0
        return 0


class StopProcessGroupTests(unittest.IsolatedAsyncioTestCase):
    @unittest.skipUnless(rook_dev.os.name == "posix", "requires POSIX process groups")
    async def test_stops_watcher_and_escalates_only_live_command_groups(self) -> None:
        process_table = """\
100 1 100
101 100 101
102 101 101
201 102 201
300 1 300
"""
        events: list[tuple[int, int]] = []
        kill_attempts: set[int] = set()
        active_groups: set[int] = set()

        def killpg(pgid: int, signum: int) -> None:
            if signum == 0:
                if pgid in active_groups:
                    return
                raise ProcessLookupError
            events.append((pgid, signum))
            if signum == signal.SIGTERM and pgid == 401:
                active_groups.add(pgid)
            if signum == signal.SIGKILL:
                kill_attempts.add(pgid)
                if pgid not in active_groups:
                    raise ProcessLookupError
                active_groups.remove(pgid)

        def check_output(*args, **kwargs) -> str:
            self.assertIn((100, signal.SIGSTOP), events)
            return process_table

        with (
            patch("subprocess.check_output", side_effect=check_output),
            patch.object(rook_dev.os, "killpg", side_effect=killpg),
            patch.object(rook_dev, "SHUTDOWN_TIMEOUT", 0.01),
        ):
            await rook_dev.stop_process_group(FakeProcess(100), {401})

        self.assertIn((100, signal.SIGCONT), events)
        self.assertEqual(
            {pgid for pgid, signum in events if signum == signal.SIGTERM},
            {100, 101, 201, 401},
        )
        self.assertEqual(kill_attempts, {401})
        self.assertFalse(active_groups)


if __name__ == "__main__":
    unittest.main()