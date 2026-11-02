"""Pruebas de las herramientas de referencia sin servicios externos."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "reference.py"


def load_reference():
    spec = importlib.util.spec_from_file_location("reference", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class InventoryTests(unittest.TestCase):
    def test_overloads_have_one_stable_inventory_id_with_all_signatures(self):
        module = load_reference()
        rows = module.source_rows("agent-memory", "a" * 40, "api.py",
            "def connect(settings: str): ...\ndef connect(settings: int): ...\n")
        self.assertEqual(len(rows), 1)
        self.assertIn("settings: str", rows[0]["capability"])
        self.assertIn("settings: int", rows[0]["capability"])
        self.assertIn("#L1", rows[0]["source_ref"])
        self.assertIn("#L2", rows[0]["source_ref"])

    def test_inventory_keeps_defaults_properties_and_lifecycle(self):
        self.assertTrue(SCRIPT.is_file(), "Falta la herramienta de inventario")
        spec = importlib.util.spec_from_file_location("reference", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        source = '''
class MemoryClient:
    @property
    def short_term(self) -> str:
        return "memory"
    async def get_context(self, query: str, *, limit: int = 10) -> str:
        return query
    async def __aenter__(self):
        return self
    def _private(self):
        pass
'''
        symbols = module.public_symbols(source)
        self.assertEqual([s[0] for s in symbols], [
            "MemoryClient.short_term", "MemoryClient.get_context", "MemoryClient.__aenter__"
        ])
        self.assertIn("limit: int=10", symbols[1][2])
        self.assertIn("property", symbols[0][2])

    def test_rows_include_nested_mcp_and_normative_clauses_without_execution(self):
        module = load_reference()
        source = '''
raise RuntimeError("no ejecutar")
def _register_core_tools(mcp):
    @mcp.tool(annotations=READ_ANNOTATIONS)
    async def memory_search(query: str, limit: int = 10):
        return []
'''
        rows = module.source_rows("agent-memory", "a" * 40, "mcp/_tools.py", source)
        self.assertEqual(len(rows), 1)
        self.assertIn("memory_search", rows[0]["capability"])
        self.assertIn("#L5", rows[0]["source_ref"])
        self.assertEqual(rows[0]["status"], "no_implementado")
        clauses = module.source_rows("agent-memory-tck", "b" * 40, "SPEC.md",
            "- **SPEC-1.1.1**: A message MUST create a node.\n")
        self.assertEqual(clauses[0]["id"], "SPEC-1.1.1")
        self.assertIn("MUST", clauses[0]["capability"])


class RepositoryTests(unittest.TestCase):
    def test_integrity_rejects_unverified_bytecode(self):
        module = load_reference()
        import hashlib

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "fixture/src"
            package.mkdir(parents=True)
            (package / "api.py").write_bytes(b"trusted\n")
            lock = {"fixture": {"package_root": "src", "files": {
                "src/api.py": hashlib.sha256(b"trusted\n").hexdigest()
            }}}
            cache = package / "__pycache__"
            cache.mkdir()
            (cache / "api.cpython-313.pyc").write_bytes(b"unverified bytecode")
            with self.assertRaisesRegex(ValueError, "adicional"):
                module.verify_source_tree(root, lock)

    def test_execution_copy_excludes_sibling_modules_and_keeps_verified_resources(self):
        module = load_reference()
        import hashlib

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "fixture/src/package"
            package.mkdir(parents=True)
            files = {"src/package/__init__.py": b"VALUE = 'trusted'\n",
                     "src/package/schema.json": b'{"type":"PERSON"}\n'}
            for name, content in files.items():
                (root / "fixture" / name).write_bytes(content)
            (package.parent / "neo4j.py").write_text("raise RuntimeError('shadow')", encoding="utf-8")
            lock = {"fixture": {"package_root": "src/package", "files": {
                name: hashlib.sha256(content).hexdigest() for name, content in files.items()
            }}}
            destination = root / "execution"
            module.materialize_sources(root, lock, destination)
            self.assertFalse((destination / "fixture/src/neo4j.py").exists())
            for name, content in files.items():
                self.assertEqual((destination / "fixture" / name).read_bytes(), content)

    def test_integrity_check_rejects_modified_and_extra_python_before_import(self):
        module = load_reference()
        import hashlib

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = root / "fixture/src"
            package.mkdir(parents=True)
            source = package / "api.py"
            source.write_bytes(b"original\n")
            lock = {"fixture": {"package_root": "src", "files": {
                "src/api.py": hashlib.sha256(source.read_bytes()).hexdigest()
            }}}
            module.verify_source_tree(root, lock)
            source.write_bytes(b"modified\n")
            with self.assertRaisesRegex(ValueError, "integridad"):
                module.verify_source_tree(root, lock)
            source.write_bytes(b"original\n")
            (package / "injected.py").write_text("raise RuntimeError()", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "adicional"):
                module.verify_source_tree(root, lock)

    def test_inventory_requires_every_declared_source_and_is_deterministic(self):
        module = load_reference()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "fixture"
            repo.mkdir()

            def git(*args):
                return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()

            git("init", "--quiet")
            (repo / "api.py").write_text("def get_context(query: str):\n    pass\n", encoding="utf-8")
            git("add", "api.py")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "test: fixture")
            refs = {"fixture": {"commit": git("rev-parse", "HEAD"), "sources": ["api.py"]}}
            first = module.build_inventory(root, refs)
            self.assertEqual(first, module.build_inventory(root, refs))
            self.assertEqual(len(first), 1)
            refs["fixture"]["sources"].append("missing.py")
            with self.assertRaisesRegex(ValueError, "missing.py"):
                module.build_inventory(root, refs)

    def test_verification_rejects_wrong_commit_and_modified_sources(self):
        module = load_reference()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", "-C", directory, *args], text=True).strip()

            git("init", "--quiet")
            (path / "source.py").write_text("original\n", encoding="utf-8")
            git("add", "source.py")
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "test: fixture")
            sha = git("rev-parse", "HEAD")
            module.verify_repository(path, sha)
            with self.assertRaisesRegex(ValueError, "commit"):
                module.verify_repository(path, "0" * 40)
            (path / "source.py").write_text("modified\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "modificad"):
                module.verify_repository(path, sha)


if __name__ == "__main__":
    unittest.main()
