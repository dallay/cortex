"""Ejecuta cortes oficiales acotados contra Rust, nunca contra el oráculo."""

import argparse
import ast
import fcntl
import hashlib
from importlib.metadata import version
import json
import os
from pathlib import Path
import platform
import re
import sys
from tempfile import TemporaryDirectory

from reference import materialize_sources
from tck_runner import CollectionEvidence, assess_report, expected_collection, prepare_suite


NORMATIVE_COMMIT = "b6c72c36bb7486881b87053ee1610f2caf972fb6"


def load_normative_lock(base):
    # JSON is a strict subset of YAML; no additional parser is required.
    lock = json.loads((Path(base) / "reference.lock.yaml").read_text(encoding="utf-8"))
    if (lock.get("commit") != NORMATIVE_COMMIT or lock.get("package_root") != "tck"
            or not lock.get("files") or not lock.get("historical_commit")):
        raise ValueError("Baseline TCK normativo inválido")
    return lock


TARGET = "v1/test_short_term.py::TestAddMessage::test_add_message_creates_conversation_on_first_call"
IMAGE_REF = re.compile(r"[^@]+@sha256:([0-9a-f]{64})")
IMAGE_ID = re.compile(r"sha256:([0-9a-f]{64})")


def validate_image_receipt(reference, image_id):
    match = IMAGE_REF.fullmatch(reference or "")
    id_match = IMAGE_ID.fullmatch(image_id or "")
    if match is None or id_match is None or image_id != "sha256:" + match.group(1):
        raise ValueError("La referencia fijada y el ID de imagen no coinciden")
    return reference, image_id


def bronze_ids_from_suite(suite=None, items=None):
    """Derive Bronze IDs from pytest's inherited marker metadata."""
    if items is None:
        raise ValueError("El inventario Bronze requiere los items coleccionados por pytest")
    ids = {item.nodeid for item in items if item.get_closest_marker("bronze") is not None}
    if len(ids) != len([item for item in items if item.get_closest_marker("bronze") is not None]):
        raise ValueError("Node IDs Bronze duplicados")
    return ids


def schema_ids_from_suite(suite, collected_ids=None):
    """Return every collected behavioral case in the pinned schema module."""
    root = Path(suite)
    schema = root / "v1/test_schema.py"
    try:
        tree = ast.parse(schema.read_text(encoding="utf-8"))
    except (OSError, SyntaxError) as error:
        raise ValueError("No se pudo inspeccionar test_schema.py fijado") from error
    selected = set()

    def visit(nodes, classes=()):
        for node in nodes:
            if isinstance(node, ast.ClassDef) and node.name.startswith("Test"):
                visit(node.body, classes + (node.name,))
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name.startswith("test_"):
                selected.add("v1/test_schema.py::" + "::".join((*classes, node.name)))

    visit(tree.body)
    module_ids = {node_id for node_id in (collected_ids or ())
                  if node_id.startswith("v1/test_schema.py::")}
    if collected_ids is not None and selected != module_ids:
        raise ValueError("El AST no cubre exactamente la colección schema")
    return selected


SILVER_PROFILES = ("silver", "silver-long-term", "silver-reasoning")
SILVER_SPLIT_PROFILES = ("silver-long-term", "silver-reasoning")
SILVER_MODULES = {"silver-long-term": ("v1/test_long_term.py", 38),
                  "silver-reasoning": ("v1/test_reasoning.py", 29)}
SILVER_TOTAL = sum(count for _, count in SILVER_MODULES.values())


def silver_partition(silver_ids):
    """Divide el marcador fijado por módulo; fuera de ellos la partición es ambigua.

    H2 solo puede reclamar `silver-long-term`; los casos de `silver-reasoning`
    pertenecen a H3 y nunca cuentan como aceptación de la memoria de largo plazo.
    """
    ids = set(silver_ids)
    if len(ids) != SILVER_TOTAL:
        raise ValueError(f"El inventario Silver fijado debe contener exactamente {SILVER_TOTAL} IDs")
    partition = {}
    classified = set()
    for profile, (module, count) in SILVER_MODULES.items():
        partition[profile] = {node_id for node_id in ids if node_id.startswith(module + "::")}
        if len(partition[profile]) != count:
            raise ValueError("La partición Silver no coincide con el inventario fijado")
        classified.update(partition[profile])
    if classified != ids:
        raise ValueError("Hay casos Silver fuera de los módulos fijados")
    return partition


def profile_selector(profile, items, suite=None):
    """Argumentos de pytest de un perfil, incluida su raíz de colección.

    `items` es el inventario coleccionado con `nodeid` y `markers`. Los perfiles
    Silver se ejecutan sólo con rutas de node ID: si además se pasa la raíz, pytest
    colecciona la suite completa y la evidencia deja de coincidir con lo declarado.
    """
    if suite is None:
        raise ValueError("La selección requiere el source suite fijado")
    root = str(Path(suite) / "v1")
    ids = {item["nodeid"] for item in items}
    if profile == "bronze":
        bronze = {item["nodeid"] for item in items if "bronze" in item["markers"]}
        return bronze, [root, "-m", "bronze"]
    if profile in SILVER_PROFILES:
        silver_ids = {item["nodeid"] for item in items if "silver" in item["markers"]}
        selected, _ = select_cases(ids, profile, silver_ids=silver_ids)
        return selected, [str(Path(suite) / node_id) for node_id in sorted(selected)]
    selected, expression = select_cases(ids, profile, suite)
    return selected, [root, "-k", expression]


def select_cases(all_ids, profile, suite=None, bronze_ids=None, silver_ids=None):
    """Deriva IDs del código fijado; el reporte debe coincidir exactamente."""
    if TARGET not in all_ids:
        raise ValueError("Test fijado ausente")
    if profile == "smoke":
        return {TARGET}, "TestAddMessage and test_add_message_creates_conversation_on_first_call"
    if profile == "schema":
        if suite is None:
            raise ValueError("El perfil schema requiere el source suite fijado")
        expected = schema_ids_from_suite(suite)
        if not expected or not expected <= all_ids:
            raise ValueError("El inventario schema no coincide con la colección TCK")
        selected = schema_ids_from_suite(suite, all_ids)
        if selected != expected:
            raise ValueError("El inventario schema no coincide con la colección TCK")
        expression = " or ".join(sorted({node_id.split("::")[1] for node_id in selected}))
        return selected, expression
    if profile == "bronze":
        if bronze_ids is None or len(bronze_ids) != 93 or bronze_ids != all_ids.intersection(bronze_ids):
            raise ValueError("El inventario Bronze fijado debe contener exactamente 93 IDs coleccionados")
        if not all_ids:
            raise ValueError("La colección TCK está vacía")
        return set(bronze_ids), "bronze"
    if profile in SILVER_PROFILES:
        if silver_ids is None or silver_ids != all_ids.intersection(silver_ids):
            raise ValueError("El inventario Silver fijado debe contener exactamente 67 IDs coleccionados")
        partition = silver_partition(silver_ids)
        if profile == "silver":
            return set(silver_ids), None
        return partition[profile], None
    if profile not in ("messages", "sessions", "chains", "search"):
        raise ValueError("Perfil no permitido")
    groups = ("TestAddMessage", "TestGetConversation")
    if profile in ("sessions", "chains"):
        groups += ("TestListSessions", "TestClearSession", "TestDeleteMessage")
    if profile == "chains":
        groups += ("TestMessageChainStructure", "TestIdempotency")
    if profile == "search":
        groups = ("TestAddMessage", "TestSearchMessages")
    selected = set()
    for group in groups:
        prefix = f"v1/test_short_term.py::{group}::"
        cases = {nodeid for nodeid in all_ids if nodeid.startswith(prefix)}
        if not cases:
            raise ValueError("Falta un grupo requerido del corte")
        selected.update(cases)
    return selected, " or ".join(groups)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--profile", choices=("smoke", "messages", "sessions", "chains", "search", "schema", "bronze"), default="smoke")
    args = parser.parse_args()
    base = Path(__file__).resolve().parent
    try:
        bridge_image_ref = os.environ.get("MUNINN_BRIDGE_IMAGE_REF", "")
        tck_image_ref = os.environ.get("MUNINN_TCK_IMAGE_REF", "")
        bridge_image_ref, bridge_image_id = validate_image_receipt(
            bridge_image_ref, os.environ.get("MUNINN_BRIDGE_IMAGE_ID"))
        tck_image_ref, tck_image_id = validate_image_receipt(
            tck_image_ref, os.environ.get("MUNINN_TCK_IMAGE_ID"))
        args.output.mkdir(parents=True, exist_ok=False)
        with (args.output.parent / ".rust-bridge.lock").open("a") as lease, TemporaryDirectory(prefix="muninn-rust-tck-") as temporary:
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
            clean = Path(temporary) / "sources"
            normative = load_normative_lock(base)
            integrity = {"agent-memory-tck": {
                key: normative[key] for key in ("commit", "package_root", "files")}}
            references = {"agent-memory-tck": {"commit": normative["commit"]}}
            if args.sources == base / ".cache/sources":
                args.sources = base / ".cache/normative-sources"
            materialize_sources(args.sources, integrity, clean)
            sys.path.insert(0, str(clean / "agent-memory-tck"))
            suite = Path(temporary) / "suite"
            test_hashes = prepare_suite(clean / "agent-memory-tck", suite)
            (suite / "v1/conftest.py").write_text("from rust_bridge_fixture import adapter\n")
            expected = expected_collection(suite, "adapted")
            if args.profile == "schema":
                schema_module_ids = {node_id for node_id in expected["all"]
                                     if node_id.startswith("v1/test_schema.py::")}
                if schema_module_ids != schema_ids_from_suite(suite):
                    raise ValueError("El AST schema no coincide con el inventario TCK")
            if args.profile == "bronze":
                # Pytest applies inherited markers during collection; validate the
                # exact Bronze inventory from the pre-deselection hook below.
                expected["selected"] = set()
                expression = "bronze"
            else:
                expected["selected"], expression = select_cases(
                    expected["all"], args.profile, suite=suite)
            os.environ["PYTEST_DISABLE_PLUGIN_AUTOLOAD"] = "1"
            os.environ.pop("PYTEST_ADDOPTS", None)
            os.environ.pop("PYTEST_PLUGINS", None)
            import pytest
            import rust_bridge_fixture

            collection = CollectionEvidence(references["agent-memory-tck"]["commit"], suite)

            class PreselectionCapture:
                def __init__(self, evidence):
                    self.evidence = evidence

                @pytest.hookimpl(hookwrapper=True, tryfirst=True)
                def pytest_collection_modifyitems(self, session, config, items):
                    self.evidence.capture_before_deselection(items)
                    yield

            preselection = PreselectionCapture(collection)
            raw = args.output.resolve() / "pytest-report.json"
            code = pytest.main([str(suite / "v1"), "-c", str(base / "tck-pytest.ini"),
                               "--rootdir", str(suite), "--import-mode=importlib", "--tb=short", "-q",
                               "-k", "bronze" if args.profile == "bronze" else expression,
                               "--json-report", f"--json-report-file={raw}", "--json-report-omit=log,streams"],
                              plugins=["pytest_asyncio.plugin", "pytest_jsonreport.plugin", "pytest_metadata.plugin",
                                       collection, preselection])
            if collection.all_items != expected["all"]:
                raise ValueError("La colección pytest completa no coincide con el inventario esperado")
            if args.profile == "bronze":
                expected["selected"], expression = select_cases(
                    expected["all"], "bronze", bronze_ids=bronze_ids_from_suite(items=collection.collected_items))
                if set(collection.items) != expected["selected"]:
                    raise ValueError("La selección Bronze no coincide con los markers heredados")
            if args.profile == "schema" and schema_ids_from_suite(suite) != {
                    node_id for node_id in collection.all_items
                    if node_id.startswith("v1/test_schema.py::")}:
                raise ValueError("Los nodeids schema no coinciden con la colección pytest")
            report = json.loads(raw.read_text())
            result = assess_report(report, code, expected, set(collection.items))
            result["pytest_report_sha256"] = hashlib.sha256(raw.read_bytes()).hexdigest()
            result["summary_unsigned"] = True
            result.update({"target":TARGET if args.profile == "smoke" else None,
                           "profile":args.profile,"expected_ids":sorted(expected["selected"]),
                           "selected_ids":sorted(case["nodeid"] for case in report.get("tests", [])),
                           "case_metadata":{node_id:collection.items[node_id]
                                            for node_id in sorted(expected["selected"])
                                            if node_id in collection.items},
                           "runner_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                           "tck_runner_sha256":hashlib.sha256((base / "tck_runner.py").read_bytes()).hexdigest(),
                           "bridge_transport_fixture_sha256":hashlib.sha256(
                               Path(rust_bridge_fixture.__file__).read_bytes()).hexdigest(),
                           "execution_inputs_sha256":{
                               name:hashlib.sha256((base / name).read_bytes()).hexdigest()
                               for name in ("Dockerfile.bridge", "Dockerfile.bridge-tck", "compose.yaml",
                                            "compose.bridge.yaml", "tck-pytest.ini", "requirements-bridge.txt",
                                            "source-integrity.json", "references.json")},
                           "bridge_image_ref":bridge_image_ref,
                           "tck_image_ref":tck_image_ref,
                           "bridge_image_id":bridge_image_id,
                           "tck_image_id":tck_image_id,
                           "harness_mount_sha256":hashlib.sha256(
                               Path(__file__).read_bytes() + (base / "tck_runner.py").read_bytes()
                               + Path(rust_bridge_fixture.__file__).read_bytes()
                               + "".join(hashlib.sha256((base / name).read_bytes()).hexdigest()
                                        for name in ("Dockerfile.bridge", "Dockerfile.bridge-tck", "compose.yaml",
                                                     "compose.bridge.yaml", "tck-pytest.ini", "requirements-bridge.txt",
                                                     "source-integrity.json", "references.json")).encode()).hexdigest(),
                           "scope":("93 casos Bronze deduplicados; no G1 ni paridad completa"
                                   if args.profile == "bronze" else "Corte acotado; no G1 ni paridad completa"),
                           "reference_commit":references["agent-memory-tck"]["commit"],
                           "test_files_sha256":test_hashes,"transport_mediation":"Bearer del host; sin cambios de resultados de memoria",
                           "events":rust_bridge_fixture.EVENTS,"python":platform.python_version(),
                           "pytest":version("pytest")})
            token=Path('/run/secrets/bridge_token').read_text().strip()
            serialized=json.dumps(result,ensure_ascii=False,indent=2)+'\n'
            if token in serialized:raise ValueError('El token apareció en evidencia')
            (args.output/'summary.json').write_text(serialized)
    except Exception as error:
        print(f"Rust TCK bloqueado ({type(error).__name__}); no hay aprobación", file=sys.stderr)
        return 1
    print(f"Rust TCK: {result['verdict']} {result['counts']}; no demuestra paridad")
    return 0 if result['verdict']=='PASS' else 2


if __name__ == "__main__":
    raise SystemExit(main())
