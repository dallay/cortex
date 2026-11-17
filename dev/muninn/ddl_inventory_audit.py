"""Clasifica inventario DDL fijado sin importar ni ejecutar fuentes upstream."""
import argparse
import ast
import csv
import hashlib
import io
import json
from pathlib import Path
import re

from inventory_test_traceability import bounded

ROOT = Path(__file__).resolve().parents[2]
REPORT = Path("docs/muninn/specs/ddl-inventory-audit.json")
DDL = re.compile(r"\bCREATE\s+(?:CONSTRAINT|(?:VECTOR |FULLTEXT )?INDEX)\b", re.I)


EXTENDED_DDL = re.compile(
    r"\bCREATE\s+(?:CONSTRAINT|(?:(?:VECTOR|FULLTEXT|TEXT|POINT)\s+)?INDEX)\b", re.I
)


def classify(raw, pattern=DDL):
    tree = ast.parse(raw)
    parents = {child: node for node in ast.walk(tree) for child in ast.iter_child_nodes(node)}
    docs = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
            if node.body and isinstance(node.body[0], ast.Expr):
                value = node.body[0].value
                if isinstance(value, ast.Constant) and isinstance(value.value, str):
                    docs.add(value)
    found = []
    for node in ast.walk(tree):
        if not (isinstance(node, ast.Constant) and isinstance(node.value, str)
                and pattern.search(node.value)):
            continue
        ancestors = []
        ancestor = parents.get(node)
        while ancestor is not None:
            ancestors.append(ancestor)
            ancestor = parents.get(ancestor)
        owner = next((item.name for item in ancestors
                      if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef))), "<module>")
        is_log = any(isinstance(item, ast.Call) and isinstance(item.func, ast.Attribute)
                     and isinstance(item.func.value, ast.Name)
                     and item.func.value.id == "logger"
                     and item.func.attr in {"debug", "info", "warning", "error", "exception"}
                     for item in ancestors)
        if node in docs:
            kind = "documentation_literal"
        elif is_log:
            kind = "log_literal"
        elif any(isinstance(item, ast.Raise) for item in ancestors):
            kind = "exception_literal"
        elif isinstance(parents.get(node), ast.JoinedStr):
            kind = "template_fragment"
        else:
            kind = "statement_literal"
        found.append({"line": node.lineno, "literal_sha256": hashlib.sha256(
            node.value.encode()).hexdigest(), "kind": kind, "owner": owner,
            "literal": node.value.strip()})
    return sorted(found, key=lambda row: (row["line"], row["literal_sha256"]))


def build(root=ROOT):
    matrix_path = Path("docs/muninn/specs/parity-matrix.csv")
    integrity_path = Path("dev/muninn/source-integrity.json")
    references_path = Path("dev/muninn/references.json")
    inputs = {}

    def read(relative):
        raw = bounded(root / relative)
        inputs[str(relative)] = hashlib.sha256(raw).hexdigest()
        return raw

    matrix = list(csv.DictReader(io.StringIO(read(matrix_path).decode())))
    integrity = json.loads(read(integrity_path))["agent-memory"]
    reference = json.loads(read(references_path))["agent-memory"]
    if reference["commit"] != integrity["commit"]:
        raise ValueError("Pins de fuente no coinciden")
    rows = [row for row in matrix if row["id"].startswith("ddl:")]
    if len({row["id"] for row in rows}) != len(rows):
        raise ValueError("ID DDL duplicado")
    source_paths = (
        "src/neo4j_agent_memory/graph/queries.py",
        "src/neo4j_agent_memory/graph/schema.py",
        "src/neo4j_agent_memory/schema/persistence.py",
    )
    parsed = {}
    expected_ids = set()
    for path in source_paths:
        raw = read(Path("dev/muninn/.cache/sources/agent-memory") / path)
        if hashlib.sha256(raw).hexdigest() != integrity["files"].get(path):
            raise ValueError("Hash de fuente DDL difiere del pin")
        parsed[path] = classify(raw)
        expected_ids.update(
            "ddl:" + path + ":" + str(item["line"]) + ":" + item["literal_sha256"][:16]
            for item in parsed[path]
        )
    if {row["id"] for row in rows} != expected_ids:
        raise ValueError("Inventario DDL incompleto o inesperado para los tres archivos fijados")
    entries = []
    for row in rows:
        _, path, line, suffix = row["id"].split(":")
        if path not in integrity["files"]:
            raise ValueError("Fuente DDL ausente del manifiesto")
        matches = [item for item in parsed[path] if item["line"] == int(line)
                   and item["literal_sha256"].startswith(suffix)]
        if len(matches) != 1 or matches[0]["literal"] != row["capability"]:
            raise ValueError("Fila DDL no corresponde al literal fijado")
        expected_ref = ("https://github.com/neo4j-labs/agent-memory/blob/"
                        + reference["commit"] + "/" + path + "#L" + line)
        if row["source_ref"] != expected_ref:
            raise ValueError("Referencia DDL no corresponde al pin")
        entries.append({"inventory_id": row["id"], "source_ref": row["source_ref"],
                        **matches[0], "behavior_status": "not_tested",
                        "disposition_status": "candidate_not_independently_reviewed",
                        "limits": "Clasificación de literal AST; no demuestra introspección, ejecución ni lifecycle Neo4j."})
    # Deliberately bounded to the same pinned files, not a repository-wide scan.
    supplementary = []
    for path in sorted(parsed):
        raw = read(Path("dev/muninn/.cache/sources/agent-memory") / path)
        if hashlib.sha256(raw).hexdigest() != integrity["files"][path]:
            raise ValueError("Hash de fuente suplementaria difiere del pin")
        legacy_keys = {(item["line"], item["literal_sha256"]) for item in parsed[path]}
        for item in classify(raw, pattern=EXTENDED_DDL):
            if (item["line"], item["literal_sha256"]) in legacy_keys:
                continue
            supplementary.append({
                **item, "repository_path": path,
                "source_sha256": integrity["files"][path],
                "source_ref": "https://github.com/neo4j-labs/agent-memory/blob/"
                              + reference["commit"] + "/" + path + "#L" + str(item["line"]),
                "inventory_status": "absent_from_frozen_inventory",
                "behavior_status": "not_tested",
                "disposition_status": "candidate_not_independently_reviewed",
            })
    return {"schema_version": 2, "parity_claim": False,
            "scope": "static_ddl_literal_classification_not_behavior",
            "reference_commit": reference["commit"],
            "inputs": inputs, "entries": entries,
            "supplementary_scope": "extended_pattern_same_three_pinned_files_only",
            "supplementary_candidates": supplementary}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    report = build()
    text = json.dumps(report, ensure_ascii=False, sort_keys=True, indent=2) + "\n"
    path = ROOT / REPORT
    if args.check:
        if bounded(path).decode() != text:
            raise SystemExit("Sidecar DDL difiere; regenerar y revisar")
    else:
        path.write_text(text)
    counts = {}
    for entry in report["entries"]:
        counts[entry["kind"]] = counts.get(entry["kind"], 0) + 1
    print(json.dumps(counts, sort_keys=True))


if __name__ == "__main__":
    main()
