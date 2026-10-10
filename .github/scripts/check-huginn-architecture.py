"""Cargo-enforced inward boundaries plus source checks for standard-library escapes."""
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
ALLOWED = {
    "huginn-core": {"async-trait", "futures", "serde", "serde_json", "thiserror", "tokio", "tokio-util", "uuid"},
    "huginn-application": {"huginn-core", "async-trait", "futures", "serde_json", "tokio"},
}


def check(metadata):
    errors = []
    packages = {p["name"]: p for p in metadata["packages"]}
    for name, allowed in ALLOWED.items():
        package = packages[name]
        # Check all declarations, including optional and target-specific edges.
        # Tests live on the inward side too; no adapter back doors through dev deps.
        for dep in package["dependencies"]:
            if dep["name"] not in allowed:
                errors.append(f"{name} depends on forbidden {dep['name']}")
        for source in (Path(package["manifest_path"]).parent / "src").rglob("*.rs"):
            text = source.read_text()
            if (re.search(r"\b(?:std|tokio)\s*::\s*(?:fs|process|net)\b", text)
                    or re.search(r"\buse\s+(?:std|tokio)\s*::\s*\{[^;]*\b(?:fs|process|net)\b", text)):
                errors.append(f"{source.relative_to(ROOT)} uses concrete filesystem/process/network I/O")
            if re.search(r"\b(?:huginn_runtime|huginn_presentation|reqwest|rusqlite|rmcp|ratatui|crossterm)\s*::", text):
                errors.append(f"{source.relative_to(ROOT)} imports an adapter")
    return errors


def main():
    result = subprocess.run(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                            cwd=ROOT, check=True, capture_output=True, text=True)
    metadata = json.loads(result.stdout)
    errors = check(metadata)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    # Negative control: a renamed, optional, target-specific dependency still fails.
    injected = json.loads(result.stdout)
    package = next(p for p in injected["packages"] if p["name"] == "huginn-application")
    package["dependencies"].append({"name": "reqwest", "rename": "transport", "optional": True})
    if not check(injected):
        raise AssertionError("architecture guard failed its negative control")
    print("Huginn architecture boundaries passed (including negative control)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
