"""Validate curated API-to-test links without executing the upstream project."""
import argparse
import ast
import csv
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[2]
REPORT = ROOT / "docs/muninn/specs/api-test-traceability.json"
MAX_BYTES = 8 * 1024 * 1024
TCK_REPOSITORY = "agent-memory-tck"
TCK_COMMIT_PATTERN = re.compile(r"[0-9a-f]{40}\Z")


class TraceabilityError(ValueError):
    """The curated traceability sidecar is inconsistent with pinned inputs."""


def fail(message):
    raise TraceabilityError(message)


def read_bytes(path):
    try:
        with path.open("rb") as stream:
            raw = stream.read(MAX_BYTES + 1)
    except OSError as error:
        fail(f"No se puede leer una fuente de trazabilidad: {path.name}")
        raise AssertionError from error
    if len(raw) > MAX_BYTES:
        fail("Fuente de trazabilidad supera el límite")
    return raw


def sha256(raw):
    return hashlib.sha256(raw).hexdigest()


def strict_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            fail("JSON contiene una clave duplicada")
        result[key] = value
    return result


def read_json(path):
    try:
        return json.loads(read_bytes(path), object_pairs_hook=strict_object)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"JSON de trazabilidad inválido: {path.name}")
        raise AssertionError from error


def read_csv(path):
    try:
        return list(csv.DictReader(read_bytes(path).decode("utf-8").splitlines()))
    except (UnicodeDecodeError, csv.Error) as error:
        fail(f"CSV de trazabilidad inválido: {path.name}")
        raise AssertionError from error


def safe_relative(root, relative):
    if (not isinstance(relative, str) or not relative or Path(relative).is_absolute()
            or ".." in Path(relative).parts):
        fail("Ruta de entrada inválida")
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root.resolve())
    except ValueError:
        fail("Ruta de entrada fuera de la raíz")
    return candidate


def class_method(source, class_name, method_name):
    try:
        tree = ast.parse(source.decode("utf-8"))
    except (UnicodeDecodeError, SyntaxError) as error:
        fail("Fuente Python upstream inválida")
        raise AssertionError from error
    classes = [node for node in tree.body if isinstance(node, ast.ClassDef)
               and node.name == class_name]
    if len(classes) != 1:
        fail(f"Clase upstream ausente o ambigua: {class_name}")
    methods = [node for node in classes[0].body
               if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
               and node.name == method_name]
    if len(methods) != 1:
        fail(f"Método upstream ausente o ambiguo: {class_name}.{method_name}")
    return methods[0]


def has_call(node, expected):
    return any(isinstance(item, ast.Call) and ast.unparse(item.func) == expected
               for item in ast.walk(node))


def input_hashes(report, root):
    values = report["inputs"]
    if not isinstance(values, list) or not values:
        fail("Faltan hashes de entrada")
    seen = set()
    for item in values:
        if not isinstance(item, dict) or set(item) != {"path", "sha256"}:
            fail("Formato inválido de hash de entrada")
        path = item["path"]
        if path in seen:
            fail("Entrada duplicada")
        seen.add(path)
        digest = item["sha256"]
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            fail("SHA-256 de entrada inválido")
        if sha256(read_bytes(safe_relative(root, path))) != digest:
            fail(f"Hash de entrada distinto: {path}")
    required = {
        "docs/muninn/specs/parity-matrix.csv",
        "docs/muninn/specs/api-profile.csv",
        "docs/muninn/specs/tck-baseline.csv",
        "docs/muninn/specs/inventory-test-traceability.json",
        "dev/muninn/source-integrity.json",
        "dev/muninn/references.json",
        "dev/muninn/.cache/sources/agent-memory-tck/tck/tests/v1/test_cross_memory.py",
    }
    if not required.issubset(seen):
        fail("Faltan dependencias fijadas en inputs")
    return seen


def get_api_rows(entry, matrix_by_id, profile_by_operation):
    inventory_id = entry.get("inventory_id")
    inventory = matrix_by_id.get(inventory_id)
    if inventory is None or not inventory_id.startswith("api:"):
        fail(f"Fila API inexistente: {inventory_id}")
    operation = entry.get("operation")
    backend = entry.get("backend")
    profiles = profile_by_operation.get((operation, backend), [])
    if len(profiles) != 1:
        fail(f"Operación/backend ausente o ambigua: {operation}/{backend}")
    profile = profiles[0]
    if inventory["source_ref"] != profile["implementation_source"]:
        fail(f"La fuente API no coincide con el perfil: {inventory_id}")
    return inventory, profile


def verify_adapter(entry, integrity, root, method, expected_target):
    adapter = entry["adapter"]
    if set(adapter) != {"repository_path", "class", "method", "call_target"}:
        fail("Campos de adaptador inválidos")
    if (adapter["repository_path"] != "tck/reference/adapter.py"
            or adapter["class"] != "ReferenceAdapter"
            or adapter["method"] != method
            or adapter["call_target"] != expected_target):
        fail("Cadena declarada del adaptador no coincide")
    expected_hash = integrity[TCK_REPOSITORY]["files"].get(adapter["repository_path"])
    if not expected_hash:
        fail("Fuente del adaptador ausente del manifiesto")
    raw = read_bytes(safe_relative(root, "dev/muninn/.cache/sources/agent-memory-tck/"
                                   + adapter["repository_path"]))
    if sha256(raw) != expected_hash:
        fail("Hash del adaptador difiere del pin")
    node = class_method(raw, adapter["class"], adapter["method"])
    if not has_call(node, expected_target):
        fail("El adaptador no realiza la llamada de API declarada")
    return raw


def inventory_union_summary(inventory_ids, spec_entries, api_entries):
    """Count static inventory handling, never behavior or acceptance."""
    groups = {
        "mapped_reference_test": set(),
        "failed_reference_test": set(),
        "planned_test": set(),
        "rejected_association": set(),
    }
    linked = {entry["id"] for entry in spec_entries if entry.get("tests")}
    for entry in api_entries:
        if entry["kind"] not in groups:
            fail("Tipo de enlace desconocido")
        groups[entry["kind"]].add(entry["inventory_id"])
    handled = linked.union(*groups.values())
    if not handled.issubset(inventory_ids):
        fail("ID de trazabilidad ausente del inventario")
    linked |= groups["mapped_reference_test"]
    return {
        "inventory_rows": len(inventory_ids),
        "rows_with_test_links": len(linked),
        "rows_with_failure_diagnostics": len(groups["failed_reference_test"]),
        "rows_with_planned_tests": len(groups["planned_test"]),
        "rows_with_rejected_associations": len(groups["rejected_association"]),
        "rows_without_test_links": len(inventory_ids - linked),
        "rows_without_recorded_handling": len(inventory_ids - handled),
    }


def reference_baseline_outcomes(rows, test_id):
    """Return historical oracle outcomes, separate from product runs.

    Product reports can share an upstream node ID. They are separate evidence
    and must not alter the curated adapted/native baseline contract.
    """
    reference_profiles = {"adapted", "native-delete"}
    return {
        (row["profile"], row["outcome"], row["gap_id"])
        for row in rows
        if row["test_id"] == test_id and row["profile"] in reference_profiles
    }


def validate(report, root=ROOT):
    expected_keys = {"schema_version", "scope", "execution_this_run", "parity_claim",
                     "reference_commit", "tck_commit", "inputs", "entries"}
    if not isinstance(report, dict) or set(report) != expected_keys:
        fail("Esquema superior de trazabilidad inválido")
    if (report["schema_version"] != 1
            or report["scope"] != "partial_api_traceability_not_parity"
            or report["execution_this_run"] != "not_run"
            or report["parity_claim"] is not False):
        fail("Alcance de evidencia inválido")

    input_hashes(report, root)
    references = read_json(safe_relative(root, "dev/muninn/references.json"))
    integrity = read_json(safe_relative(root, "dev/muninn/source-integrity.json"))
    reference_commit = references["agent-memory"]["commit"]
    tck_commit = references[TCK_REPOSITORY]["commit"]
    if (report["reference_commit"] != reference_commit
            or not TCK_COMMIT_PATTERN.fullmatch(tck_commit)
            or report["tck_commit"] != tck_commit
            or integrity[TCK_REPOSITORY]["commit"] != tck_commit):
        fail("Pins de referencia incompatibles")

    matrix_rows = read_csv(safe_relative(root, "docs/muninn/specs/parity-matrix.csv"))
    matrix_by_id = {row["id"]: row for row in matrix_rows}
    if len(matrix_by_id) != len(matrix_rows):
        fail("IDs de inventario duplicados")
    profile_rows = read_csv(safe_relative(root, "docs/muninn/specs/api-profile.csv"))
    profile_by_operation = {}
    for row in profile_rows:
        profile_by_operation.setdefault((row["operation"], row["backend"]), []).append(row)
    baseline = read_csv(safe_relative(root, "docs/muninn/specs/tck-baseline.csv"))
    traceability = read_json(safe_relative(
        root, "docs/muninn/specs/inventory-test-traceability.json"))
    spec_tests = {}
    for item in traceability["entries"]:
        for test in item.get("tests", []):
            spec_tests[item["id"]] = test
    unmapped_specs = {
        (item["id"], item["path"], item["nodeid"])
        for item in traceability.get("unmapped_declarations", [])
    }

    if not isinstance(report["entries"], list):
        fail("entries debe ser una lista")
    seen = set()
    counts = {"mapped": 0, "reference_failed": 0, "planned": 0, "rejected": 0}
    for entry in report["entries"]:
        if not isinstance(entry, dict):
            fail("Entrada de trazabilidad inválida")
        kind = entry.get("kind")
        inventory, profile = get_api_rows(entry, matrix_by_id, profile_by_operation)
        if kind in {"mapped_reference_test", "failed_reference_test"}:
            evidence_id = entry.get("test", {}).get("nodeid")
        elif kind == "planned_test":
            evidence_id = entry.get("planned_test_id")
        else:
            evidence_id = entry.get("candidate_test_id")
        key = (entry["inventory_id"], kind, evidence_id)
        if key in seen:
            fail("Enlace API duplicado")
        seen.add(key)

        if kind in {"mapped_reference_test", "failed_reference_test"}:
            required_entry = {"kind", "inventory_id", "operation", "backend", "test",
                              "adapter", "reference_baseline", "coverage_limits"}
            if kind == "mapped_reference_test":
                required_entry.add("link_type")
                if entry.get("link_type") != "indirect_via_reference_adapter":
                    fail("Relación del enlace existente inválida")
            else:
                required_entry.add("failure_diagnostic")
            if set(entry) != required_entry:
                fail("Formato de enlace existente inválido")
            test = entry["test"]
            expected_test_fields = {"repository_path", "class", "function", "nodeid", "spec_id",
                                    "source_sha256", "assertion_scope"}
            if set(test) != expected_test_fields or not test["assertion_scope"]:
                fail("Campos de test existente inválidos")
            relative = test["repository_path"]
            expected_hash = integrity[TCK_REPOSITORY]["files"].get(relative)
            if not expected_hash or test["source_sha256"] != expected_hash:
                fail("Test source hash no coincide con el pin")
            test_raw = read_bytes(safe_relative(
                root, "dev/muninn/.cache/sources/agent-memory-tck/" + relative))
            if sha256(test_raw) != expected_hash:
                fail("Bytes del test difieren del pin")
            expected_nodeid = (relative + "::" + test["class"] + "::" + test["function"])
            if test["nodeid"] != expected_nodeid:
                fail("Node ID no coincide con clase y función")
            test_node = class_method(test_raw, test["class"], test["function"])
            doc = ast.get_docstring(test_node) or ""
            if not doc.startswith(test["spec_id"] + ":"):
                fail("El docstring no declara la cláusula indicada")
            spec_link = spec_tests.get(test["spec_id"])
            if spec_link:
                if spec_link["nodeid"] != test["nodeid"]:
                    fail("El enlace no coincide con el sidecar SPEC existente")
            elif (test["spec_id"], relative, test["class"] + "::" + test["function"]) not in unmapped_specs:
                fail("SPEC sin asociación ni diagnóstico de inventario")
            method = entry["operation"].rsplit(".", 1)[-1]
            if not has_call(test_node, "adapter." + method):
                fail("El test no invoca el método de adapter indicado")
            target = "self._client." + entry["operation"].removeprefix("client.")
            adapter_raw = verify_adapter(entry, integrity, root, method, target)
            baseline_id = relative.removeprefix("tck/tests/") + "::" + test["class"] + "::" + test["function"]
            matching = [row for row in baseline if row["profile"] == "adapted"
                        and row["test_id"] == baseline_id and test["spec_id"] in row["clauses"].split(";")]
            expected_outcome = "passed" if kind == "mapped_reference_test" else "failed"
            if (len(matching) != 1 or matching[0]["outcome"] != expected_outcome
                    or (kind == "failed_reference_test" and matching[0]["gap_id"] != "REF-002")):
                fail("El resultado histórico del baseline no coincide")
            baseline_record = entry["reference_baseline"]
            expected_baseline_fields = {"profile", "test_id", "outcome", "target"}
            if kind == "failed_reference_test":
                expected_baseline_fields.add("gap_id")
            if (set(baseline_record) != expected_baseline_fields
                    or baseline_record["profile"] != "adapted"
                    or baseline_record["test_id"] != baseline_id
                    or baseline_record["outcome"] != expected_outcome
                    or baseline_record["target"] != "upstream_reference_oracle_only"):
                fail("El baseline no está limitado al oráculo upstream")
            if kind == "failed_reference_test":
                diagnostic = entry["failure_diagnostic"]
                if (set(diagnostic) != {"gap_id", "stage", "assertions_reached", "known_caveat"}
                        or diagnostic["gap_id"] != "REF-002"
                        or diagnostic["stage"] != "adapter_result_conversion_after_api_call"
                        or diagnostic["assertions_reached"] is not False
                        or not diagnostic["known_caveat"]
                        or baseline_record["gap_id"] != "REF-002"):
                    fail("Diagnóstico del fallo adapter inválido")
                adapter_node = class_method(adapter_raw, "ReferenceAdapter", method)
                if not any(isinstance(item, ast.Attribute) and item.attr == "properties"
                           for item in ast.walk(adapter_node)):
                    fail("No se encontró el acceso incompatible del adapter documentado")
            if not isinstance(entry["coverage_limits"], list) or not entry["coverage_limits"]:
                fail("El enlace debe declarar límites de cobertura")
            counts["mapped" if kind == "mapped_reference_test" else "reference_failed"] += 1

        elif kind == "planned_test":
            required_entry = {"kind", "inventory_id", "operation", "backend", "planned_test_id",
                              "planned_assertions", "state", "coverage_limits"}
            if set(entry) != required_entry or entry["state"] != "planned_not_implemented":
                fail("Formato de prueba planificada inválido")
            planned = entry["planned_test_id"]
            if (not isinstance(planned, str) or not planned.startswith("dev/muninn/tests/")
                    or "::test_" not in planned or not entry["planned_assertions"]
                    or not entry["coverage_limits"]):
                fail("La propuesta de test debe tener ID y aserciones específicas")
            test_path = planned.split("::", 1)[0]
            if safe_relative(root, test_path).exists():
                fail("Una prueba marcada planificada ya existe")
            counts["planned"] += 1

        elif kind == "rejected_association":
            required_entry = {"kind", "inventory_id", "operation", "backend", "candidate_test_id",
                              "spec_id", "adapter", "disposition", "baseline_results", "reason"}
            if set(entry) != required_entry or entry["disposition"] != "not_evidence_for_api_method":
                fail("Formato de asociación rechazada inválido")
            method = entry["operation"].rsplit(".", 1)[-1]
            raw = verify_adapter(entry, integrity, root, method, entry["adapter"]["call_target"])
            node = class_method(raw, entry["adapter"]["class"], method)
            expected_api_call = "self._client.short_term." + method
            if has_call(node, expected_api_call) or not any(
                    isinstance(item, ast.Call) and ast.unparse(item.func).endswith("execute_write")
                    for item in ast.walk(node)):
                fail("La exclusión del adaptador no está demostrada por el AST")
            candidate = entry["candidate_test_id"]
            outcomes = reference_baseline_outcomes(baseline, candidate)
            if not {("adapted", "passed", ""), ("native-delete", "failed", "REF-001")} <= outcomes:
                fail("Falta registrar la divergencia de perfiles del test candidato")
            recorded = entry["baseline_results"]
            if (not isinstance(recorded, list)
                    or any(not isinstance(item, dict)
                           or set(item) != {"profile", "outcome", "gap_id"}
                           for item in recorded)):
                fail("Formato inválido de resultados baseline")
            normalized = sorted((item["profile"], item["outcome"], item["gap_id"])
                                for item in recorded)
            if normalized != sorted(outcomes):
                fail("Los outcomes registrados no coinciden con el baseline")
            candidate_parts = candidate.split("::")
            if len(candidate_parts) != 3 or not candidate_parts[0].startswith("v1/"):
                fail("Node ID candidato inválido")
            test_relative = "tck/tests/" + candidate_parts[0]
            expected_hash = integrity[TCK_REPOSITORY]["files"].get(test_relative)
            if not expected_hash:
                fail("Test candidato ausente del manifiesto")
            test_raw = read_bytes(safe_relative(
                root, "dev/muninn/.cache/sources/agent-memory-tck/" + test_relative))
            if sha256(test_raw) != expected_hash:
                fail("Hash del test candidato difiere del pin")
            test_node = class_method(test_raw, candidate_parts[1], candidate_parts[2])
            if not (ast.get_docstring(test_node) or "").startswith(entry["spec_id"] + ":"):
                fail("La prueba candidata no declara la cláusula registrada")
            if not has_call(test_node, "adapter." + method):
                fail("La prueba candidata no invoca el método de adapter")
            counts["rejected"] += 1
        else:
            fail("Tipo de enlace desconocido")

    if counts != {"mapped": 39, "reference_failed": 6, "planned": 3, "rejected": 1}:
        fail("Conteo de esta tranche cambió; requiere revisión explícita")
    mapped_operations = {
        entry["operation"] for entry in report["entries"]
        if entry.get("kind") == "mapped_reference_test"
    }
    if len(mapped_operations) != 20:
        fail("Conteo de operaciones API positivas cambió; requiere revisión explícita")
    counts["mapped_operations"] = len(mapped_operations)
    counts["inventory_union"] = inventory_union_summary(
        set(matrix_by_id), traceability["entries"], report["entries"]
    )
    return counts


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Valida el sidecar sin escribir")
    args = parser.parse_args(argv)
    if not args.check:
        parser.error("solo se admite --check; el sidecar se revisa manualmente")
    try:
        report = read_json(REPORT)
        counts = validate(report)
    except TraceabilityError as error:
        parser.exit(1, f"Trazabilidad inválida: {error}\n")
    print(json.dumps(counts, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
