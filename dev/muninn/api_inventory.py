"""Inventario consumible por backend: declaraciones, no conformidad runtime."""

import ast
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys

from reference import public_symbols, verify_repository


COLUMNS = ("operation", "backend", "kind", "contract_signature", "implementation_signature",
           "static_state", "contract_source", "implementation_source", "binding_source",
           "notes", "muninn_status", "runtime_status")


def class_members(source, class_name):
    tree = ast.parse(source)
    classes = [node for node in ast.walk(tree) if isinstance(node, ast.ClassDef) and node.name == class_name]
    if len(classes) != 1:
        raise ValueError(f"Clase ausente o ambigua: {class_name}")
    result = {}
    for node in classes[0].body:
        if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        if node.name.startswith("_") and node.name not in {"__init__", "__aenter__", "__aexit__"}:
            continue
        decorators = [ast.unparse(item) for item in node.decorator_list]
        body = list(node.body)
        if body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) and isinstance(body[0].value.value, str):
            body = body[1:]
        state = "declaracion_con_cuerpo"
        if any(name.endswith("abstractmethod") for name in decorators):
            state = "abstracto"
        elif len(body) == 1 and (isinstance(body[0], ast.Pass) or
                isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) and body[0].value.value is Ellipsis):
            state = "declaracion_contractual"
        elif body and isinstance(body[0], ast.Raise) and isinstance(body[0].exc, ast.Call):
            if ast.unparse(body[0].exc.func).endswith(("NotSupportedError", "NotImplementedError")):
                state = "rechazo_explicito"
        elif len(body) == 1 and isinstance(body[0], ast.Return):
            value = body[0].value
            if value is None or isinstance(value, ast.Constant) and value.value in (None, ""):
                state = "retorno_constante_vacio"
        signature = f"{'async ' if isinstance(node, ast.AsyncFunctionDef) else ''}{node.name}({ast.unparse(node.args)})"
        if node.returns is not None:
            signature += " -> " + ast.unparse(node.returns)
        member = {"line": node.lineno, "kind": "property" if "property" in decorators else "method",
                  "signature": signature, "state": state}
        if node.name in result:
            result[node.name]["signature"] += " | " + signature
        else:
            result[node.name] = member
    return result


def profile_rows(read, bindings, notes, commit):
    def link(path, line):
        return f"https://github.com/neo4j-labs/agent-memory/blob/{commit}/{path}#L{line}"

    def members(spec):
        if not spec or spec.get("sentinel"):
            return {}
        result = {}
        if spec.get("base"):
            node = next(node for node in ast.walk(ast.parse(read(spec["path"])))
                        if isinstance(node, ast.ClassDef) and node.name == spec["class"])
            bases = {ast.unparse(base.value if isinstance(base, ast.Subscript) else base).rsplit(".", 1)[-1]
                     for base in node.bases}
            if spec["base"]["class"] not in bases:
                raise ValueError("La herencia configurada no existe en la referencia")
            result.update(members(spec["base"]))
        for name, member in class_members(read(spec["path"]), spec["class"]).items():
            result[name] = {**member, "source": link(spec["path"], member["line"])}
        return result

    rows, used_notes = [], set()
    sentinel_source = None
    if bindings.get("unsupported"):
        spec = bindings["unsupported"]
        nodes = [node for node in ast.walk(ast.parse(read(spec["path"])))
                 if isinstance(node, ast.ClassDef) and node.name == spec["class"]]
        if len(nodes) != 1:
            raise ValueError("Sentinela ausente o ambiguo")
        sentinel_source = link(spec["path"], nodes[0].lineno)
    if sentinel_source is None and any(spec.get("sentinel") for value in bindings["accessors"].values()
                                       for key, spec in value.items() if key in {"bolt", "nams"}):
        raise ValueError("Sentinela sin fuente verificada")

    def add(operation, backend, declaration, implementation, binding_source, sentinel=None, condition=None):
        state = implementation["state"] if implementation else "sin_binding"
        if sentinel:
            state = "rechazo_resolver_atributo"
        row = dict(zip(COLUMNS, (
            operation, backend, (implementation or declaration or {}).get("kind", "no_declarado"),
            declaration.get("signature", "") if declaration else "",
            implementation.get("signature", "") if implementation else "",
            state, declaration.get("source", "") if declaration else "",
            sentinel or (implementation.get("source", "") if implementation else ""),
            binding_source, "", "no_implementado", "no_verificado",
        )))
        key = f"{backend}:{operation}"
        if key in notes:
            used_notes.add(key)
            row["notes"] = notes[key]["note"]
            row["static_state"] = notes[key].get("state", state)
        if condition:
            row["notes"] = (row["notes"] + " " + condition.get("method_note", "")).strip()
            if condition.get("conditional") and row["static_state"] == "declaracion_con_cuerpo":
                row["static_state"] = "condicional"
        rows.append(row)

    facade = members(bindings["facade"])
    if bindings.get("factory"):
        spec = bindings["factory"]
        declarations = [item for item in public_symbols(read(spec["path"])) if item[0] == spec["name"]]
        if not declarations:
            raise ValueError("Fábrica pública ausente")
        member = {"kind": "factory", "signature": " | ".join(item[2] for item in declarations),
                  "state": "fabrica_declarada", "source": "; ".join(link(spec["path"], item[1]) for item in declarations)}
        for backend in ("bolt", "nams"):
            add(spec["name"], backend, None, member, member["source"])
    for name, member in facade.items():
        for backend in ("bolt", "nams"):
            add("client." + name, backend, None, member, member["source"])
    for accessor, configs in bindings["accessors"].items():
        contract = members(configs.get("contract"))
        concrete = {backend: members(configs[backend]) for backend in ("bolt", "nams")}
        names = (set(contract) | set(concrete["bolt"]) | set(concrete["nams"])) - {
            "__init__", "__aenter__", "__aexit__"}
        for name in sorted(names):
            for backend in ("bolt", "nams"):
                spec = configs[backend]
                add(f"client.{accessor}.{name}", backend, contract.get(name), concrete[backend].get(name),
                    facade[accessor]["source"], sentinel_source if spec.get("sentinel") else None, spec)
    if set(notes) != used_notes:
        raise ValueError("Hay una nota sin operación correspondiente: " + ", ".join(sorted(set(notes) - used_notes)))
    return sorted(rows, key=lambda row: (row["operation"], row["backend"]))


def exported_names(source, commit, path):
    exports = []
    for node in ast.parse(source).body:
        if isinstance(node, ast.Assign) and any(isinstance(t, ast.Name) and t.id == "__all__" for t in node.targets):
            if not isinstance(node.value, ast.List):
                raise ValueError("__all__ no es una lista estática")
            for item in node.value.elts:
                name = ast.literal_eval(item)
                if not isinstance(name, str):
                    raise ValueError("Export no textual")
                exports.append({"name": name, "kind": "export_declarado_no_capacidad",
                                "source_ref": f"https://github.com/neo4j-labs/agent-memory/blob/{commit}/{path}#L{item.lineno}"})
    if not exports or len(exports) != len({item["name"] for item in exports}):
        raise ValueError("Exports ausentes o duplicados")
    return exports


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    base = Path(__file__).resolve().parent
    try:
        reference = json.loads((base / "references.json").read_text())["agent-memory"]
        commit = reference["commit"]
        repo = args.sources / "agent-memory"
        verify_repository(repo, commit)
        cache = {}

        def read(path):
            if path not in cache:
                cache[path] = subprocess.check_output(
                    ["git", "-C", str(repo), "show", f"{commit}:{path}"], timeout=120)
            return cache[path].decode("utf-8")

        bindings = json.loads((base / "api-bindings.json").read_text())
        notes = json.loads((base / "api-notes.json").read_text())
        rows = profile_rows(read, bindings, notes, commit)
        exports = exported_names(read(bindings["facade"]["path"]), commit, bindings["facade"]["path"])
        metadata = {"scope": "Declaraciones y bindings estáticos; no certifica runtime ni Muninn",
                    "reference_commit": commit, "rows": len(rows), "exports": exports,
                    "source_sha256": {path: hashlib.sha256(data).hexdigest() for path, data in sorted(cache.items())},
                    "audit_inputs_sha256": {name: hashlib.sha256((base / name).read_bytes()).hexdigest()
                                            for name in ("api-bindings.json", "api-notes.json", "api_inventory.py")}}
        output = io.StringIO(newline="")
        writer = csv.DictWriter(output, fieldnames=COLUMNS, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)
        for path, content in ((args.output, output.getvalue()),
                              (args.metadata, json.dumps(metadata, ensure_ascii=False, sort_keys=True, indent=2) + "\n")):
            if args.check:
                if path.read_text(encoding="utf-8") != content:
                    raise ValueError(f"Inventario no coincide: {path}")
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")
    except (OSError, ValueError, KeyError, SyntaxError, subprocess.SubprocessError) as error:
        print(f"Inventario bloqueado: {error}", file=sys.stderr)
        return 1
    print(f"Inventario API: {len(rows)} filas, {len(exports)} exports; sin conformidad runtime")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
