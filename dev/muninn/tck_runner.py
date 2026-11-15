"""Ejecuta contratos TCK fijados contra el oráculo, nunca contra producción."""

import argparse
import ast
import asyncio
from collections import Counter
import hashlib
from importlib.metadata import version
import json
import os
from pathlib import Path
import platform
import re
import secrets
import sys
from tempfile import TemporaryDirectory


ADAPTATION_MANIFEST = "reference-relationship-properties.json"
ADAPTATION_PATCH = "reference-relationship-properties.patch.json"
ADAPTATION_MANIFEST_SHA256 = "673fa1f4a84adb97dc0ee8c8dd406eb378af2a5efcf56a5c98e3f0b570f48b4a"
ADAPTATION_PATCH_SHA256 = "09fe6866c13fc47435202fb1aaac1d6ceda74cdc2f829276addf13056132e768"


def load_reference_adaptation(base, integrity):
    """Carga el patch versionado y verifica que describe la fuente fijada."""
    manifest_path = base / ADAPTATION_MANIFEST
    patch_path = base / ADAPTATION_PATCH
    manifest_bytes = manifest_path.read_bytes()
    if hashlib.sha256(manifest_bytes).hexdigest() != ADAPTATION_MANIFEST_SHA256:
        raise ValueError("El hash del manifiesto de adaptación no coincide")
    manifest = json.loads(manifest_bytes)
    patch_bytes = patch_path.read_bytes()
    patch = json.loads(patch_bytes)
    reference = integrity.get(manifest.get("repository"), {})
    if manifest.get("base_commit") != reference.get("commit"):
        raise ValueError("El commit base de la adaptación no coincide con el pin TCK")
    if manifest.get("target") != "tck/reference/adapter.py":
        raise ValueError("La adaptación apunta a una ruta no permitida")
    expected_hash = reference.get("files", {}).get(manifest["target"])
    if manifest.get("upstream_sha256") != expected_hash:
        raise ValueError("El hash base de la adaptación no coincide con source-integrity")
    if hashlib.sha256(patch_bytes).hexdigest() != manifest.get("patch_sha256"):
        raise ValueError("El hash del patch de adaptación no coincide")
    if manifest.get("patch_file") != ADAPTATION_PATCH:
        raise ValueError("El manifiesto apunta a un patch no permitido")
    if not isinstance(patch.get("replacements"), list) or not patch["replacements"]:
        raise ValueError("El patch de adaptación no contiene reemplazos")
    manifest["patch"] = patch
    manifest["patch_sha256"] = hashlib.sha256(patch_bytes).hexdigest()
    return manifest


def apply_reference_adaptation(source_root, adaptation):
    """Aplica el patch solo al adaptador fijado y devuelve hashes separados."""
    relative = Path(adaptation["target"])
    if relative.is_absolute() or ".." in relative.parts:
        raise ValueError("Ruta de adaptación fuera del árbol de fuentes")
    root = Path(source_root).resolve()
    target = (root / relative).resolve()
    if not target.is_relative_to(root) or not target.is_file():
        raise ValueError("Ruta de adaptación fuera del árbol de fuentes")
    original = target.read_bytes()
    upstream_hash = hashlib.sha256(original).hexdigest()
    if upstream_hash != adaptation["upstream_sha256"]:
        raise ValueError("El hash del adaptador upstream no coincide")
    patched = original
    for replacement in adaptation["patch"]["replacements"]:
        old = replacement["old"].encode("utf-8")
        new = replacement["new"].encode("utf-8")
        count = patched.count(old)
        if count != 1:
            raise ValueError("El contexto del patch debe coincidir exactamente una vez")
        patched = patched.replace(old, new, 1)
    target.write_bytes(patched)
    return {
        "id": adaptation["id"],
        "applied": True,
        "base_commit": adaptation["base_commit"],
        "upstream_sha256": upstream_hash,
        "patch_sha256": adaptation["patch_sha256"],
        "adapted_sha256": hashlib.sha256(patched).hexdigest(),
        "scope": "oráculo adaptado; no es Muninn ni demuestra conformidad completa",
    }


async def claim_transaction(transaction, token):
    """Marca solo una base vacía; nunca borra datos para prepararla."""
    result = await transaction.run("MATCH (n) RETURN count(n) AS count")
    if (await result.single())["count"] != 0:
        raise ValueError("El TCK requiere una base sintética vacía")
    result = await transaction.run("CREATE (:MuninnTckGuard {name: 'owner', token: $token})", token=token)
    await result.consume()


async def guarded_reset(read, write, token):
    """Limpia fixtures solo si existe la marca privada de esta ejecución."""
    guard = await read(
        "MATCH (g:MuninnTckGuard) RETURN count(g) AS count, "
        "sum(CASE WHEN g.name = 'owner' AND g.token = $token THEN 1 ELSE 0 END) AS owned",
        {"token": token})
    if len(guard) != 1 or guard[0]["count"] != 1 or guard[0].get("owned") != 1:
        raise ValueError("Falta la marca exclusiva de la base sintética")
    result = await write("""
        MATCH (guard:MuninnTckGuard {name: 'owner', token: $token})
        MATCH (all_guards:MuninnTckGuard)
        WITH guard, count(all_guards) AS owners WHERE owners = 1
        CALL (guard) {
            MATCH (n) WHERE NOT n:MuninnTckGuard
            DETACH DELETE n
            RETURN count(*) AS deleted
        }
        RETURN deleted
        """, {"token": token})
    if len(result) != 1:
        raise ValueError("La marca desapareció; no se admite un borrado sin scope")


def assess_report(report, exit_code, expected=None, collected=None):
    """Un skip, un error o una suite vacía no demuestra conformidad."""
    counts = {name: report.get("summary", {}).get(name, 0) for name in
              ("passed", "failed", "error", "skipped", "xfailed", "xpassed", "total", "collected", "deselected")}
    if exit_code not in {0, 1} or counts["total"] == 0:
        verdict = "BLOCKED"
    elif exit_code == 1 or counts["failed"] or counts["error"]:
        verdict = "FAIL"
    elif counts["skipped"] or counts["xfailed"] or counts["xpassed"]:
        verdict = "INCOMPLETE"
    elif counts["passed"] == counts["total"]:
        verdict = "PASS"
    else:
        verdict = "BLOCKED"
    coverage_verified = False
    if expected is not None and collected is not None:
        cases = report.get("tests", [])
        ids = {case["nodeid"] for case in cases}
        outcomes = Counter(case["outcome"] for case in cases)
        coverage_verified = (
            set(collected) == expected["selected"] == ids
            and len(cases) == len(ids) == counts["total"]
            and counts["collected"] == len(expected["all"])
            and counts["deselected"] == len(expected["all"] - expected["selected"])
            and all(outcomes[name] == counts[name] for name in
                    ("passed", "failed", "error", "skipped", "xfailed", "xpassed"))
        )
        if not coverage_verified:
            verdict = "BLOCKED"
    elif verdict == "PASS":
        verdict = "BLOCKED"
    return {"verdict": verdict, "counts": counts, "pytest_exit_code": int(exit_code),
            "coverage_verified": coverage_verified}


def phase_diagnostic(phase, execution):
    """Conserva razones de fallo/skip; normaliza solo la ruta temporal."""
    result = dict(phase.get("crash", {}))
    if "longrepr" in phase:
        result["longrepr"] = phase["longrepr"]
    return {key: value.replace(str(execution), "<execution>") if isinstance(value, str) else value
            for key, value in result.items()}


def prepare_suite(tck_root, destination):
    """Conserva tests y fixtures compartidas; reemplaza solo el adaptador."""
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "v1").mkdir()
    shared = tck_root / "tck/tests/conftest.py"
    (destination / "conftest.py").write_bytes(shared.read_bytes())
    (destination / "v1/conftest.py").write_text(
        "from tck_fixture import adapter\n", encoding="utf-8")
    hashes = {}
    for source in sorted((tck_root / "tck/tests/v1").glob("test_*.py")):
        content = source.read_bytes()
        (destination / "v1" / source.name).write_bytes(content)
        hashes[source.relative_to(tck_root).as_posix()] = hashlib.sha256(content).hexdigest()
    if not hashes:
        raise ValueError("No hay tests oficiales en la referencia verificada")
    return hashes


def expected_collection(suite, profile):
    """Deriva los casos del código fijado, antes de aplicar filtros pytest."""
    result = set()
    for source in sorted((suite / "v1").glob("test_*.py")):
        prefix = source.relative_to(suite).as_posix()

        def visit(body, parents=()):
            for node in body:
                if isinstance(node, ast.ClassDef) and node.name.startswith("Test"):
                    visit(node.body, (*parents, node.name))
                elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test_"):
                    result.add("::".join((prefix, *parents, node.name)))

        visit(ast.parse(source.read_text(encoding="utf-8")).body)
    selected = result if profile == "adapted" else {
        name for name in result if ("delete" in name.lower() or "deletion" in name.lower())
        and "delete_conversation" not in name.lower()
    }
    return {"all": result, "selected": selected}


async def claim_database(password, token):
    from neo4j import AsyncGraphDatabase

    async with AsyncGraphDatabase.driver("bolt://neo4j:7687", auth=("neo4j", password)) as driver:
        # La unicidad serializa dos CREATE concurrentes del propietario singleton.
        await driver.execute_query(
            "CREATE CONSTRAINT muninn_tck_owner IF NOT EXISTS "
            "FOR (g:MuninnTckGuard) REQUIRE g.name IS UNIQUE", database_="neo4j")
        async with driver.session(database="neo4j") as session:
            await session.execute_write(claim_transaction, token)
        records, _, _ = await driver.execute_query(
            "CALL dbms.components() YIELD versions, edition RETURN versions, edition", database_="neo4j")
        return [record.data() for record in records]


class CollectionEvidence:
    """Observa la colección, sin modificar tests, marcas ni resultados."""

    def __init__(self, commit, suite):
        self.items = {}
        self.all_items = set()
        self.collected_items = []
        self.commit = commit
        self.suite = suite

    def capture_before_deselection(self, items):
        self.collected_items = list(items)
        self.all_items = {item.nodeid for item in items}

    def pytest_collection_finish(self, session):
        for item in session.items:
            self.items[item.nodeid] = {
                "tiers": [tier for tier in ("bronze", "silver", "gold", "platinum")
                          if item.get_closest_marker(tier) is not None],
                "clauses": sorted(set(re.findall(r"SPEC-[\d.]+", item.obj.__doc__ or ""))),
                "source_ref": f"https://github.com/neo4j-labs/agent-memory-tck/blob/{self.commit}/tck/tests/"
                              f"{item.path.relative_to(self.suite).as_posix()}#L{item.obj.__code__.co_firstlineno}",
            }


def main(argv=None):
    from reference import materialize_sources

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--profile", choices=("adapted", "native-delete"), required=True)
    parser.add_argument("--output", type=Path, required=True, help="Directorio nuevo para esta ejecución")
    args = parser.parse_args(argv)
    base = Path(__file__).resolve().parent
    stage = "integridad"
    try:
        references = json.loads((base / "references.json").read_text(encoding="utf-8"))
        integrity = json.loads((base / "source-integrity.json").read_text(encoding="utf-8"))
        commits = {name: item["commit"] for name, item in references.items()}
        if commits != {name: item["commit"] for name, item in integrity.items()}:
            raise ValueError("Commits incompatibles entre manifiestos")
        adaptation = load_reference_adaptation(base / "adaptations", integrity)
        args.output.mkdir(parents=True, exist_ok=False)
        with TemporaryDirectory(prefix="muninn-tck-") as execution:
            clean = Path(execution) / "sources"
            materialize_sources(args.sources, integrity, clean)
            adapted_source = apply_reference_adaptation(
                clean / adaptation["repository"], adaptation)
            suite = Path(execution) / "suite"
            test_hashes = prepare_suite(clean / "agent-memory-tck", suite)
            expected = expected_collection(suite, args.profile)
            sys.path[:0] = [str(clean / "agent-memory/src"), str(clean / "agent-memory-tck")]
            os.environ["PYTEST_DISABLE_PLUGIN_AUTOLOAD"] = "1"
            os.environ.pop("PYTEST_ADDOPTS", None)
            os.environ.pop("PYTEST_PLUGINS", None)
            auth = Path("/run/secrets/neo4j_auth").read_text(encoding="utf-8").strip()
            username, password = auth.split("/", 1)
            if username != "neo4j" or not password:
                raise ValueError("Secreto sintético inválido")
            token = secrets.token_hex(24)
            stage = "base_sintetica_vacia"
            neo4j = asyncio.run(claim_database(password, token))
            import pytest
            import tck_fixture

            tck_fixture.configure(args.profile, token, password)
            collection = CollectionEvidence(commits["agent-memory-tck"], suite)
            raw = args.output.resolve() / "pytest-report.json"
            options = [str(suite / "v1"), "-c", str(base / "tck-pytest.ini"),
                       "--rootdir", str(suite), "--import-mode=importlib", "--tb=short",
                       "--show-capture=no", "-q", "--json-report", f"--json-report-file={raw}",
                       "--json-report-omit=log,streams"]
            if args.profile == "native-delete":
                options += ["-k", "(delete or deletion) and not delete_conversation"]
            stage = "pytest"
            code = pytest.main(options, plugins=[
                "pytest_asyncio.plugin", "pytest_jsonreport.plugin", "pytest_metadata.plugin", collection])
            report = json.loads(raw.read_text(encoding="utf-8"))
            summary = assess_report(report, code, expected, set(collection.items))
            summary.update({
                "profile": args.profile, "reference_commits": commits, "neo4j": neo4j,
                "scope": "oraculo_con_adaptaciones_TCK; no es Muninn ni demuestra paridad completa",
                "reference_adapter_adaptation": adapted_source,
                "fixture_controls": {
                    "embedding": "tck.fixtures.mocks.MockEmbedder; SHA256, 1536 dimensiones",
                    "extraction": "none", "resolution": "none", "geocoding": False, "enrichment": False,
                    "reset": "base inicialmente vacía; marca privada por ejecución; solo datos sintéticos",
                    "delete_message": "nativo" if args.profile == "native-delete" else "Cypher correctivo del ReferenceAdapter",
                },
                "test_files_sha256": test_hashes,
                "local_files_sha256": {
                    name: hashlib.sha256((base / name).read_bytes()).hexdigest()
                    for name in ("tck_runner.py", "tck_fixture.py", "tck-pytest.ini", "requirements-tck.txt",
                                 "reference.py", "Dockerfile.tck", "compose.tck.yaml")
                },
                "adaptation_files_sha256": {
                    f"adaptations/{name}": hashlib.sha256(
                        (base / "adaptations" / name).read_bytes()).hexdigest()
                    for name in (ADAPTATION_MANIFEST, ADAPTATION_PATCH)
                },
                "runtime": {"python": platform.python_version(), "packages": {
                    name: version(name) for name in ("pytest", "pytest-asyncio", "pytest-json-report", "neo4j", "pydantic")}},
                "cases": [],
            })
            for case in report.get("tests", []):
                item = {"nodeid": case["nodeid"], "outcome": case["outcome"],
                        **collection.items.get(case["nodeid"], {})}
                if case["outcome"] != "passed":
                    item["diagnostics"] = {phase: phase_diagnostic(case[phase], execution) for phase in
                                           ("setup", "call", "teardown") if phase in case and
                                           case[phase].get("outcome") != "passed"}
                summary["cases"].append(item)
            (args.output / "summary.json").write_text(
                json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except Exception as error:
        print(f"TCK bloqueado en {stage} ({type(error).__name__}); no hay aprobación", file=sys.stderr)
        return 1
    print(f"TCK del oráculo: {summary['verdict']}; {summary['counts']}; no es conformidad de Muninn")
    return 0 if summary["verdict"] == "PASS" else 2


if __name__ == "__main__":
    raise SystemExit(main())
