"""Valida recibos de artefactos; no descarga pesos ni ejecuta modelos."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

from reference import verify_repository


PROFILE_FILES = {
    "fastino/gliner2.5-base-v1": {"config.json", "encoder_config/config.json", "tokenizer.json",
                                "tokenizer_config.json", "README.md", "model.safetensors"},
    "sentence-transformers/all-MiniLM-L6-v2": {
        "config.json", "config_sentence_transformers.json", "modules.json", "1_Pooling/config.json",
        "sentence_bert_config.json", "tokenizer.json", "tokenizer_config.json", "special_tokens_map.json",
        "vocab.txt", "README.md", "model.safetensors"},
    "explosion/en_core_web_sm": {"en_core_web_sm-3.8.0-py3-none-any.whl"},
}


def safe_path(path):
    if not isinstance(path, str) or not path or "\\" in path or path.startswith("/"):
        raise ValueError("Ruta de artefacto no relativa")
    if any(part in {"", ".", ".."} for part in path.split("/")):
        raise ValueError("Ruta de artefacto no canónica")
    return path


def file_record(path, size, *, lfs_sha256=None, content=None, cache_file=None):
    safe_path(path)
    if type(size) is not int or size <= 0:
        raise ValueError("Tamaño de artefacto inválido")
    if lfs_sha256 is not None and not re.fullmatch(r"[0-9a-f]{64}", lfs_sha256):
        raise ValueError("hash SHA-256 LFS inválido")
    if content is None:
        if lfs_sha256 is None:
            raise ValueError("Falta hash fuerte: un blob Git SHA-1 no es SHA-256")
        return {"path": path, "size": size, "sha256": lfs_sha256, "verification": "metadata_lfs_no_bytes"}
    digest = hashlib.sha256(content).hexdigest()
    if len(content) != size or lfs_sha256 is not None and digest != lfs_sha256:
        raise ValueError("Tamaño o hash no coincide con los bytes")
    if cache_file is None:
        raise ValueError("Falta ruta del recibo de bytes")
    return {"path": path, "size": size, "sha256": digest, "verification": "bytes_at_capture",
            "cache_file": safe_path(cache_file)}


def compatible_version(version, requirement):
    def parts(value):
        if not re.fullmatch(r"\d+\.\d+\.\d+", value):
            raise ValueError("Versión no soportada por esta comprobación acotada")
        return tuple(map(int, value.split(".")))

    current = parts(version)
    comparisons = []
    for clause in requirement.split(","):
        match = re.fullmatch(r"\s*(>=|<)(\d+\.\d+\.\d+)\s*", clause)
        if match is None:
            raise ValueError("Rango no soportado por esta comprobación acotada")
        comparisons.append(current >= parts(match[2]) if match[1] == ">=" else current < parts(match[2]))
    return all(comparisons)


def validate_manifest(manifest):
    if manifest["schema_version"] != 1 or not re.fullmatch(r"[0-9a-f]{40}", manifest["reference_commit"]):
        raise ValueError("Versión u origen de manifiesto inválido")
    if not re.fullmatch(r"[0-9a-f]{64}", manifest["upstream_lock_sha256"]):
        raise ValueError("Hash del lock inválido")
    if (manifest.get("profile_name") != "local-cpu-candidate-v1"
            or {m["id"] for m in manifest["models"]} != set(PROFILE_FILES)
            or len(manifest["models"]) != len(PROFILE_FILES)
            or len(manifest["plugins"]) != 1):
        raise ValueError("El perfil no contiene sus modelos y plugin obligatorios")
    ids, cache_files = set(), {}
    for model in manifest["models"]:
        identifier = model["id"]
        if not re.fullmatch(r"[\w.-]+/[\w.-]+", identifier) or any(p in {".", ".."} for p in identifier.split("/")):
            raise ValueError("ID de modelo inválido")
        if identifier in ids:
            raise ValueError("Modelo duplicado")
        ids.add(identifier)
        if not re.fullmatch(r"[0-9a-f]{40}", model["revision"]):
            raise ValueError("La revisión no es inmutable")
        if model["runtime_validation"] != "not_run":
            raise ValueError("Este recibo no acredita validación runtime")
        if model.get("framework_requirement") and not compatible_version(
                manifest["framework_packages"][model["framework"]], model["framework_requirement"]):
            raise ValueError("Modelo incompatible con la versión fijada del framework")
        seen = set()
        if not model["files"]:
            raise ValueError("Modelo sin artefactos")
        for item in model["files"]:
            safe_path(item["path"])
            if (item["path"] in seen or not re.fullmatch(r"[0-9a-f]{64}", item["sha256"])
                    or type(item["size"]) is not int or item["size"] <= 0):
                raise ValueError("Artefacto duplicado o hash/tamaño inválido")
            seen.add(item["path"])
            if item["verification"] == "bytes_at_capture":
                cache_file = safe_path(item["cache_file"])
                if cache_file in cache_files and cache_files[cache_file] != item["sha256"]:
                    raise ValueError("Dos artefactos incompatibles comparten ruta de caché")
                cache_files[cache_file] = item["sha256"]
            elif item["verification"] != "metadata_lfs_no_bytes" or "cache_file" in item:
                raise ValueError("Verificación de bytes no acreditada")
        if seen != PROFILE_FILES[identifier]:
            raise ValueError("El perfil no contiene sus archivos obligatorios")
    for plugin in manifest["plugins"]:
        if plugin.get("activation_verified") is not False:
            raise ValueError("Este recibo no acredita activación de plugins")
        path = plugin["path"]
        if (plugin.get("name") != "apoc-core" or not path.startswith("/var/lib/neo4j/labs/")
                or not path.endswith("-core.jar") or type(plugin["size"]) is not int or plugin["size"] <= 0):
            raise ValueError("Ruta o tamaño de plugin inválido")
        safe_path(path.removeprefix("/"))
        if (not re.search(r"@sha256:[0-9a-f]{64}$", plugin["image"])
                or not re.fullmatch(r"[0-9a-f]{64}", plugin["sha256"])
                or plugin["runtime_validation"] != "not_run"
                or plugin["verification"] != "bytes_in_image"):
            raise ValueError("Plugin sin referencia inmutable o con runtime no acreditado")


def verify_cached_file(item, root):
    root = root.resolve()
    path = (root / safe_path(item["cache_file"])).resolve()
    if not path.is_relative_to(root):
        raise ValueError("Ruta de caché fuera del directorio autorizado")
    if path.stat().st_size != item["size"]:
        raise ValueError("Tamaño de caché no coincide")
    with path.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if digest != item["sha256"]:
        raise ValueError("hash de caché no coincide")


def verify_cache(manifest, root):
    validate_manifest(manifest)
    checked = 0
    for model in manifest["models"]:
        for item in model["files"]:
            if item["verification"] != "bytes_at_capture":
                continue
            verify_cached_file(item, root)
            checked += 1
    return checked


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--cache", type=Path)
    args = parser.parse_args(argv)
    try:
        manifest = json.loads(args.input.read_text(encoding="utf-8"))
        validate_manifest(manifest)
        expected = json.loads(Path(__file__).with_name("references.json").read_text())["agent-memory"]["commit"]
        if manifest["reference_commit"] != expected:
            raise ValueError("La referencia no coincide con references.json")
        repo = args.sources / "agent-memory"
        verify_repository(repo, expected)
        data = subprocess.check_output(["git", "-C", str(repo), "show", f"{expected}:uv.lock"], timeout=120)
        if hashlib.sha256(data).hexdigest() != manifest["upstream_lock_sha256"]:
            raise ValueError("El lock fuente no coincide")
        packages = tomllib.loads(data.decode())["package"]
        for name, version in manifest["framework_packages"].items():
            if {p["version"] for p in packages if p["name"] == name} != {version}:
                raise ValueError("Versión ausente o ambigua en uv.lock: " + name)
        checked = verify_cache(manifest, args.cache) if args.cache else 0
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"Referencias bloqueadas: {error}", file=sys.stderr)
        return 1
    print(f"Referencias estructurales verificadas; {checked} archivos de caché comprobados; sin inferencia ni plugin activo")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
