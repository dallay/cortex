"""Offline lock correspondence for the CPython 3.13/macOS ARM64 oracle.

Run from the repository root with Python >=3.11 and -B. No imports from the
oracle environment, package resolution, network requests, or model access.
"""

import ast
import hashlib
from pathlib import Path
import re
import tomllib
import unittest
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[3]
LOCK = ROOT / "dev/muninn/.cache/sources/agent-memory/uv.lock"
REQUIREMENTS = ROOT / "dev/muninn/requirements-minilm-cpu.txt"
LOCK_SHA256 = "30a0c40d5bba6ceecf9cfd6a280a066ad055bb091bbb2d6ac540c845ec943553"
ENV = {
    "python_full_version": "3.13.18",
    "python_version": "3.13",
    "sys_platform": "darwin",
    "platform_system": "Darwin",
    "platform_machine": "arm64",
    "os_name": "posix",
    "implementation_name": "cpython",
}


def applies(marker):
    """Evaluate only the marker grammar used by this pinned dependency closure."""
    def value(node):
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return node.value
        if isinstance(node, ast.Name):
            return ENV[node.id]  # Unknown environment fields must fail closed.
        raise ValueError(ast.dump(node))

    def visit(node):
        if isinstance(node, ast.BoolOp):
            values = [visit(item) for item in node.values]
            if isinstance(node.op, ast.And):
                return all(values)
            if isinstance(node.op, ast.Or):
                return any(values)
        if isinstance(node, ast.Compare) and len(node.ops) == 1:
            left, right = value(node.left), value(node.comparators[0])
            if isinstance(node.left, ast.Name) and node.left.id.startswith("python_"):
                if right.endswith(".*"):
                    if isinstance(node.ops[0], ast.Eq):
                        return left.startswith(right[:-1])
                    if isinstance(node.ops[0], ast.NotEq):
                        return not left.startswith(right[:-1])
                    raise ValueError("Unsupported wildcard comparison")
                left = tuple(map(int, left.split(".")))
                right = tuple(map(int, right.split(".")))
                width = max(len(left), len(right))
                left += (0,) * (width - len(left))
                right += (0,) * (width - len(right))
            op = node.ops[0]
            if isinstance(op, ast.Eq):
                return left == right
            if isinstance(op, ast.NotEq):
                return left != right
            if isinstance(op, ast.Lt):
                return left < right
            if isinstance(op, ast.LtE):
                return left <= right
            if isinstance(op, ast.Gt):
                return left > right
            if isinstance(op, ast.GtE):
                return left >= right
        raise ValueError(ast.dump(node))

    return visit(ast.parse(marker, mode="eval").body)


def selected_packages(lock):
    selected = {}
    for package in lock["package"]:
        markers = package.get("resolution-markers", [])
        if not markers or any(applies(marker) for marker in markers):
            name = package["name"]
            if name in selected:
                raise ValueError(f"Ambiguous lock package: {name}")
            selected[name] = package
    return selected


def wheel_names(package, digest):
    return [unquote(urlsplit(wheel["url"]).path.rsplit("/", 1)[-1])
            for wheel in package.get("wheels", [])
            if wheel["hash"] == f"sha256:{digest}"]


def compatible_wheel(filename):
    _, python_tag, abi, platform = filename.removesuffix(".whl").rsplit("-", 3)
    if platform == "any":
        return "py3" in python_tag.split(".") and abi == "none"
    if not re.fullmatch(r"macosx_\d+_\d+_arm64", platform):
        return False
    if tuple(map(int, platform.split("_")[1:3])) > (14, 0):
        return False
    if python_tag == "cp313" and abi == "cp313":
        return True
    return (abi == "abi3" and python_tag.startswith("cp")
            and 32 <= int(python_tag[2:]) <= 313)


class CpuRequirementsTests(unittest.TestCase):
    def test_wheel_platform_respects_minimum_macos_14_contract(self):
        for version, expected in (("14_0", True), ("15_0", False), ("27_2", False)):
            with self.subTest(macos=version):
                filename = f"example-1.0-cp313-cp313-macosx_{version}_arm64.whl"
                self.assertEqual(compatible_wheel(filename), expected)

    def test_complete_hashed_closure_matches_pinned_upstream_lock(self):
        lock_bytes = LOCK.read_bytes()
        self.assertEqual(hashlib.sha256(lock_bytes).hexdigest(), LOCK_SHA256)
        packages = selected_packages(tomllib.loads(lock_bytes.decode()))
        text = REQUIREMENTS.read_text()
        self.assertIn(LOCK_SHA256, text)
        pins = {}
        for line in text.splitlines():
            if not line or line.startswith("#"):
                continue
            match = re.fullmatch(r"([A-Za-z0-9_.-]+)==([^ ]+) --hash=sha256:([a-f0-9]{64})", line)
            self.assertIsNotNone(match, line)
            name, version, digest = match.groups()
            name = re.sub(r"[-_.]+", "-", name).lower()
            self.assertNotIn(name, pins)
            self.assertIn(name, packages)
            self.assertEqual(version, packages[name]["version"], name)
            filenames = wheel_names(packages[name], digest)
            self.assertTrue(any(compatible_wheel(item) for item in filenames), name)
            pins[name] = version
        # Derive closure from the lock, not a duplicated list of expected pins.
        pending = ["sentence-transformers"]
        closure = set()
        while pending:
            name = pending.pop()
            if name in closure:
                continue
            closure.add(name)
            for dep in packages[name].get("dependencies", []):
                if "marker" in dep and not applies(dep["marker"]):
                    continue
                self.assertNotIn("extra", dep, dep)
                if "version" in dep:
                    self.assertEqual(dep["version"], packages[dep["name"]]["version"])
                pending.append(dep["name"])
        self.assertEqual(set(pins), closure)


if __name__ == "__main__":
    unittest.main()
