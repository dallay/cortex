"""Cargo-enforced hexagonal boundaries for all Huginn crates plus Rook independence."""
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
ALLOWED = {
    "huginn-core": {"async-trait", "futures", "serde", "serde_json", "thiserror", "tokio", "tokio-util", "uuid"},
    "huginn-application": {"huginn-core", "async-trait", "futures", "serde_json", "tokio"},
    "huginn-plugin-api": {"huginn-core", "async-trait", "tokio"},
    "huginn-reference-extension": {"huginn-plugin-api", "async-trait", "tokio"},
    "huginn-runtime": {
        "huginn-application", "huginn-core",
        "async-trait", "futures", "serde", "serde_json", "tokio", "tokio-util",
        "reqwest", "bytes", "libc", "rusqlite", "uuid", "rmcp", "similar",
        "fs2", "process-wrap", "tempfile", "axum",
    },
    "huginn-presentation": {
        "huginn-application", "huginn-core", "huginn-plugin-api",
        "huginn-reference-extension",
        "async-trait", "futures", "tokio", "uuid",
        "ratatui", "crossterm", "unicode-width", "portable-pty", "tempfile",
    },
    # Composition root: only crate allowed to assemble concrete extensions.
    "huginn": {
        "huginn-application", "huginn-core", "huginn-runtime",
        "huginn-presentation", "huginn-reference-extension",
        "anyhow", "async-trait", "clap", "dirs", "serde", "serde_json",
        "tokio", "toml", "uuid", "portable-pty", "tempfile",
    },
}

# Test-support fixtures that deliberately wire concrete extensions outside the
# composition root. They are bin targets under tests/support, covered here so
# the target-based scan stays explicit instead of silently skipping them.
EXEMPT = {
    "crates/huginn/presentation/tests/support/fixture.rs",
    "crates/huginn/runtime/tests/support/session_lock_holder.rs",
}

FS_DIRECT = re.compile(r"\b(?:std|tokio)\s*::\s*(?:fs|process|net)\b")
FS_USE = re.compile(r"\buse\s+(?:std|tokio)\s*::\s*\{[^;]*\b(?:fs|process|net)\b")
ADAPTER_INWARD = re.compile(
    r"\b(?:huginn_runtime|huginn_presentation|huginn_reference_extension|huginn|"
    r"reqwest|rusqlite|rmcp|ratatui|crossterm)\s*::"
)
ROOK_IMPORT = re.compile(
    r"\b(?:rook|rook_core|rook_usecases|shared_kernel|transport_axum|"
    r"providers_\w+|cache_memory|audit_sqlite)\s*::"
)
PRESENTATION_CONCRETE = re.compile(r"\bhuginn_reference_extension\s*::")
RUNTIME_FORBIDDEN = re.compile(r"\b(?:huginn_presentation|huginn_reference_extension)\s*::")
# Also catches `use huginn_runtime as rt;` / `use huginn_reference_extension as ext;`
# which would otherwise bypass the crate-qualified path check.
PRESENTATION_FORBIDDEN = re.compile(
    r"\b(?:huginn_runtime|huginn_reference_extension)\s*::"
    r"|\buse\s+(?:huginn_runtime|huginn_reference_extension)\s+as\b"
)

INWARD = {"huginn-core", "huginn-application", "huginn-plugin-api", "huginn-reference-extension"}


def package_sources(package):
    """All Rust sources declared by Cargo targets (src_path), not just src/.

    Integration tests, examples and support bins live outside src/ and must be
    checked too. Layouts Cargo cannot resolve to a file are rejected loudly
    instead of being skipped silently.
    """
    files = {}
    for target in package.get("targets", []):
        src = Path(target["src_path"])
        if src.suffix == ".rs" and src.is_file():
            files[src.resolve()] = src
        elif src.is_dir():
            for rs in src.rglob("*.rs"):
                files[rs.resolve()] = rs
        else:
            raise AssertionError(
                f"unsupported target layout: {package['name']}:{target['name']} "
                f"src_path={target['src_path']}"
            )
    return list(files.values())


def check(metadata):
    errors = []
    packages = {p["name"]: p for p in metadata["packages"]}
    for name, allowed in ALLOWED.items():
        if name not in packages:
            errors.append(f"expected package {name} missing from workspace metadata")
            continue
        package = packages[name]
        # Check all declarations, including optional and target-specific edges.
        # Tests live on the inward side too; no adapter back doors through dev deps.
        for dep in package["dependencies"]:
            if dep["name"] not in allowed:
                errors.append(f"{name} depends on forbidden {dep['name']}")
        own = name.replace("-", "_")
        own_path = re.compile(r"\b" + re.escape(own) + r"\s*::")
        for source in package_sources(package):
            rel = source.relative_to(ROOT).as_posix()
            if rel in EXEMPT:
                continue
            text = source.read_text()
            # An integration test importing its own crate (e.g. smoke.rs) is
            # not a layering breach; strip self-paths before the adapter check.
            inward_text = own_path.sub("", text)
            if ROOK_IMPORT.search(text):
                errors.append(f"{rel} imports a Rook-side crate (Huginn must stay independent)")
            if name in INWARD:
                if FS_DIRECT.search(text) or FS_USE.search(text):
                    errors.append(f"{rel} uses concrete filesystem/process/network I/O")
                if ADAPTER_INWARD.search(inward_text):
                    # reference-extension itself never self-imports; any such import is a layering breach.
                    errors.append(f"{rel} imports an adapter")
            elif name == "huginn-presentation":
                if PRESENTATION_FORBIDDEN.search(text):
                    errors.append(
                        f"{rel} imports a concrete extension or runtime "
                        f"(allowed only in composition root apps/huginn and tests/support fixture)"
                    )
            elif name == "huginn-runtime":
                if RUNTIME_FORBIDDEN.search(text):
                    errors.append(f"{rel} imports presentation or a concrete extension")
            elif name == "huginn":
                # Composition root may assemble everything Huginn; only Rook is forbidden (checked above).
                pass
    return errors


def main():
    result = subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                            cwd=ROOT, check=True, capture_output=True, text=True)
    metadata = json.loads(result.stdout)
    errors = check(metadata)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    # Negative control 1: a renamed, optional, target-specific dependency still fails.
    injected = json.loads(result.stdout)
    package = next(p for p in injected["packages"] if p["name"] == "huginn-application")
    package["dependencies"].append({"name": "reqwest", "rename": "transport", "optional": True})
    if not check(injected):
        raise AssertionError("architecture guard failed its negative control (application->reqwest)")
    # Negative control 2: Rook independence — inward crate must not gain a Rook dep.
    injected2 = json.loads(result.stdout)
    core = next(p for p in injected2["packages"] if p["name"] == "huginn-core")
    core["dependencies"].append({"name": "rook-core"})
    if not check(injected2):
        raise AssertionError("architecture guard failed its Rook-independence control")
    # Negative control 3: presentation src must not import a concrete extension.
    # Simulated via direct regex check on the forbidden pattern.
    if not PRESENTATION_FORBIDDEN.search("use huginn_reference_extension::ReferenceExtension;"):
        raise AssertionError("architecture guard failed its presentation-extension control")
    # Negative control 4: aliased imports must not bypass the presentation guard.
    if not PRESENTATION_FORBIDDEN.search("use huginn_reference_extension as extension;"):
        raise AssertionError("architecture guard failed its aliased presentation-extension control")
    print("Huginn architecture boundaries passed (including negative controls)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
