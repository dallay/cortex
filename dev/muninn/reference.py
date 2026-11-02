"""Inventario estático de referencias fijadas; no importa ni ejecuta upstream."""

import ast
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import re
import subprocess
import sys


COLUMNS = ("id", "capability", "source_ref", "reference_mode", "parity_level",
           "muninn_location", "test_id", "status", "gap_reason")


def git(path, *args):
    """Ejecuta Git sin intérprete de comandos ni entrada interactiva."""
    return subprocess.check_output(
        ["git", "-C", str(path), *args], text=True, timeout=120,
        stdin=subprocess.DEVNULL,
    ).strip()


def verify_repository(path, expected_commit):
    """Rechaza commits distintos y modificaciones de archivos de referencia."""
    if git(path, "rev-parse", "HEAD") != expected_commit:
        raise ValueError(f"El commit no coincide en {path}")
    if git(path, "status", "--porcelain", "--untracked-files=no"):
        raise ValueError(f"Hay archivos modificados en {path}")


def public_symbols(source):
    """Devuelve nombre, línea y firma de métodos públicos y funciones."""
    result = []

    def visit(body, prefix=""):
        for node in body:
            if isinstance(node, ast.ClassDef):
                if not node.name.startswith("_"):
                    visit(node.body, prefix + node.name + ".")
            elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                if node.name.startswith("_") and node.name not in {
                    "__init__", "__aenter__", "__aexit__", "__enter__", "__exit__"
                }:
                    continue
                decorators = " ".join(ast.unparse(d) for d in node.decorator_list)
                signature = f"{decorators} {'async ' if isinstance(node, ast.AsyncFunctionDef) else ''}{node.name}({ast.unparse(node.args)})"
                if node.returns:
                    signature += " -> " + ast.unparse(node.returns)
                result.append((prefix + node.name, node.lineno, signature.strip()))

    visit(ast.parse(source).body)
    return result


def source_rows(repository, commit, path, source):
    """Genera evidencia estática, no una afirmación de compatibilidad."""
    rows = []
    by_id = {}

    def add(identifier, line, capability, mode):
        row = dict(zip(COLUMNS, (
            identifier, capability,
            f"https://github.com/neo4j-labs/{repository}/blob/{commit}/{path}#L{line}",
            mode, "por_verificar", "", "", "no_implementado",
            "Inventario estático; faltan contrato ejecutado e implementación Muninn",
        )))
        if identifier in by_id:
            previous = by_id[identifier]
            previous["capability"] += " | " + capability
            previous["source_ref"] += "; " + row["source_ref"]
        else:
            by_id[identifier] = row
            rows.append(row)

    if path.endswith(".md"):
        for number, line in enumerate(source.splitlines(), 1):
            match = re.match(r"- \*\*(SPEC-[\d.]+)\*\*: (.+)", line)
            if match:
                add(match[1], number, match[2], "normativo_TCK")
            elif line.startswith("| `"):
                add(f"schema:{path}:{number}", number, line, "tabla_TCK")
        return rows

    tree = ast.parse(source)
    for name, line, signature in public_symbols(source):
        add(f"api:{path}:{name}", line, signature, "declaracion_estatica")
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            if any(isinstance(d, ast.Call) and isinstance(d.func, ast.Attribute)
                   and d.func.attr == "tool" for d in node.decorator_list):
                add(f"mcp:{path}:{node.name}", node.lineno,
                    f"{node.name}({ast.unparse(node.args)})", "MCP_registro_por_verificar")
        elif isinstance(node, ast.ClassDef) and not node.name.startswith("_"):
            fields = [ast.unparse(field).split(" = ")[0] for field in node.body
                      if isinstance(field, ast.AnnAssign)]
            add(f"type:{path}:{node.name}", node.lineno,
                node.name + (": " + "; ".join(fields) if fields else ""),
                "tipo_estatico")
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            if re.search(r"\bCREATE\s+(?:CONSTRAINT|(?:VECTOR |FULLTEXT )?INDEX)\b",
                         node.value, re.IGNORECASE):
                digest = hashlib.sha256(node.value.encode()).hexdigest()[:16]
                add(f"ddl:{path}:{node.lineno}:{digest}", node.lineno,
                    node.value.strip(), "DDL_estatico")
    return rows


def build_inventory(root, references):
    """Lee únicamente objetos Git de los commits verificados."""
    rows = []
    for repository, reference in sorted(references.items()):
        path = root / repository
        commit = reference["commit"]
        verify_repository(path, commit)
        tracked = git(path, "ls-tree", "-r", "--name-only", commit).splitlines()
        selected = set()
        for prefix in reference["sources"]:
            matches = [name for name in tracked if name == prefix or name.startswith(prefix + "/")]
            if not matches:
                raise ValueError(f"Fuente obligatoria ausente: {repository}/{prefix}")
            selected.update(name for name in matches if name.endswith((".py", ".md")))
        for name in sorted(selected):
            source = git(path, "show", f"{commit}:{name}")
            rows.extend(source_rows(repository, commit, name, source))
    return sorted(rows, key=lambda row: row["id"])


def render_inventory(rows):
    """Serializa el CSV canónico con orden y saltos de línea estables."""
    output = io.StringIO(newline="")
    writer = csv.DictWriter(output, fieldnames=COLUMNS, lineterminator="\n")
    writer.writeheader()
    writer.writerows(rows)
    return output.getvalue()


def build_integrity(root, references):
    """Fija bytes importables para comprobarlos sin Git dentro del capturador."""
    result = {}
    for name, reference in sorted(references.items()):
        path = root / name
        commit = reference["commit"]
        verify_repository(path, commit)
        package_root = reference.get("package_root", ".")
        tracked = git(path, "ls-tree", "-r", "--name-only", commit).splitlines()
        files = {}
        for filename in tracked:
            if (filename.startswith(package_root + "/") or package_root == "."
                    or filename in {"LICENSE", "uv.lock", "pyproject.toml", "SPEC.md"}):
                content = subprocess.check_output(
                    ["git", "-C", str(path), "show", f"{commit}:{filename}"], timeout=120)
                files[filename] = hashlib.sha256(content).hexdigest()
        result[name] = {"commit": commit, "package_root": package_root, "files": files}
    return result


def verify_source_tree(root, integrity):
    """Rechaza bytes distintos y archivos adicionales dentro del paquete."""
    for name, reference in integrity.items():
        path = root / name
        for filename, expected in reference["files"].items():
            if hashlib.sha256((path / filename).read_bytes()).hexdigest() != expected:
                raise ValueError(f"Fallo de integridad en {name}/{filename}")
        package_root = reference["package_root"]
        actual = {p.relative_to(path).as_posix() for p in
                  (path / package_root).rglob("*") if p.is_file()}
        expected = {filename for filename in reference["files"]
                    if filename.startswith(package_root + "/") or package_root == "."}
        if actual != expected:
            raise ValueError(f"Archivos adicionales o ausentes en {name}")


def materialize_sources(root, integrity, destination):
    """Copia solo bytes autorizados; no expone cachés ni módulos hermanos."""
    verify_source_tree(root, integrity)
    for name, reference in integrity.items():
        for filename, expected in reference["files"].items():
            content = (root / name / filename).read_bytes()
            if hashlib.sha256(content).hexdigest() != expected:
                raise ValueError(f"Fallo de integridad al copiar {name}/{filename}")
            target = destination / name / filename
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sources", type=Path, required=True,
                        help="Directorio que contiene ambos repositorios fijados")
    parser.add_argument("--output", type=Path, required=True, help="CSV de salida")
    parser.add_argument("--check", action="store_true", help="Verifica sin escribir")
    parser.add_argument("--integrity-output", type=Path, help="Manifiesto de bytes importables")
    args = parser.parse_args(argv)
    references = json.loads(Path(__file__).with_name("references.json").read_text(encoding="utf-8"))
    try:
        rows = build_inventory(args.sources, references)
        output = render_inventory(rows)
        if args.check:
            if args.output.read_text(encoding="utf-8") != output:
                raise ValueError("El inventario no coincide con las fuentes fijadas")
        else:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(output, encoding="utf-8")
        if args.integrity_output:
            integrity = build_integrity(args.sources, references)
            verify_source_tree(args.sources, integrity)
            serialized = json.dumps(integrity, indent=2, sort_keys=True) + "\n"
            if args.check:
                if args.integrity_output.read_text(encoding="utf-8") != serialized:
                    raise ValueError("El manifiesto de integridad no coincide")
            else:
                args.integrity_output.write_text(serialized, encoding="utf-8")
    except (OSError, ValueError, SyntaxError, subprocess.SubprocessError) as error:
        print(f"Error de referencia: {error}", file=sys.stderr)
        return 1
    print(f"Inventario verificado: {len(rows)} filas; no demuestra paridad")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
