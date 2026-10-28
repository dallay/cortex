#!/usr/bin/env python3
"""Check the path-rule scanner controls and the explicitly authorized inventory."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile


RULE = "rust.actix.path-traversal.tainted-path.tainted-path"
ROOT = Path(__file__).resolve().parents[2]
DOC = "docs/security/semgrep-path-exceptions.md"
# File, enclosing function, exact sink line, evidence anchor, specific reason.
NEW = (
    ("apps/huginn/src/config.rs", "load", "match std::fs::read_to_string(&resolved) {",
     "huginn-config", "Operator --config or fixed host config; intentionally outside workspace."),
    ("apps/huginn/src/ui_settings.rs", "load_or_init",
     'fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;',
     "huginn-ui-settings", "Host config/save path or migration seed; no model/UI filepath."),
    ("apps/rook/src/di.rs", "build",
     "SqliteUsageRepository::new(Path::new(&config.database.db_path))",
     "rook-usage-db", "Operator SQLite path; Path::new only converts; constructor opens authorized DB."),
    ("crates/huginn/runtime/src/tools.rs", "prepare",
     "Some(tokio::fs::read_to_string(&resolved).await?)",
     "runtime-prepare", "resolve_path confines preview read to workspace; stable-tree guarantee only."),
    ("crates/huginn/runtime/src/tools.rs", "execute_mutation",
     "let current = match tokio::fs::read_to_string(path).await {",
     "runtime-mutation", "Private caller revalidates workspace path before stale check; stable-tree only."),
    ("crates/huginn/runtime/src/tools.rs", "execute_read",
     "let text = tokio::fs::read_to_string(path).await?;",
     "runtime-read", "Private caller revalidates workspace path before read; stable-tree only."),
    ("crates/huginn/runtime/src/tools.rs", "execute_search",
     "let Ok(text) = tokio::fs::read_to_string(&file).await else {",
     "runtime-search", "Validated workspace root and regular non-link entries; stable-tree only."),
)
# Historical annotations: frozen inventory, NOT a new approval of their rationale.
OLD = (
    ("apps/rook/src/di.rs", "is_in_memory_sqlite_target",
     f"// nosemgrep: {RULE}", "fn is_in_memory_sqlite_target(path: &str) -> bool {"),
    ("apps/rook/src/di.rs", "resolve_api_key_secret",
     f"std::path::PathBuf::from(db_path.replacen('~', &home, 1)) // nosemgrep: {RULE}", ""),
    ("apps/rook/src/di.rs", "resolve_api_key_secret",
     f"std::path::PathBuf::from(db_path) // nosemgrep: {RULE}", ""),
    ("apps/rook/src/di.rs", "resolve_api_key_secret",
     f"let s = std::fs::read_to_string(&secret_path) // nosemgrep: {RULE}", ""),
    ("apps/rook/src/config.rs", "load",
     f"let content = std::fs::read_to_string(path)?; // nosemgrep: {RULE}", ""),
    ("apps/rook/src/main.rs", "load_config",
     f"// nosemgrep: {RULE}", "config::RookConfig::load(&config_path)"),
    ("apps/rook/src/main.rs", "load_config_with_path",
     f"// nosemgrep: {RULE}", "config::RookConfig::load(&config_path)"),
)


def require(condition, details):
    if not condition:
        raise ValueError(details)


def check_inventory(root=ROOT):
    files = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "*.rs"],
        cwd=root,
    ).decode().split("\0")
    actual_old, actual_new = [], []
    for filename in sorted(set(files) - {""}):
        if {"target", "node_modules"}.intersection(Path(filename).parts):
            continue
        lines = (root / filename).read_text().splitlines()
        function = ""
        for index, line in enumerate(lines):
            match = re.search(r"\bfn\s+(\w+)\s*\(", line)
            if match:
                function = match[1]
            annotation = re.search(r"//\s*no(?:semgrep|sem)\b(.*)", line)
            if not annotation:
                continue
            require(annotation[1].strip().startswith(":") and annotation[1].strip()[1:].strip(),
                    ("unscoped suppression is not authorized", filename, index + 1))
            if RULE not in line:
                continue
            following = next((s.strip() for s in lines[index + 1:]
                              if s.strip() and not s.lstrip().startswith("//")), "")
            if index >= 2 and lines[index - 1].strip().startswith(f"// Evidence: {DOC}#"):
                anchor = lines[index - 1].strip().removeprefix(f"// Evidence: {DOC}#")
                reason = lines[index - 2].strip().removeprefix("// ")
                require(line.strip() == f"// nosemgrep: {RULE}", "annotation changed")
                require(lines[index + 1].strip() == following, "annotation detached from sink")
                actual_new.append((filename, function, following, anchor, reason))
            else:
                next_function = re.match(r"fn\s+(\w+)\s*\(", following)
                actual_old.append((filename, next_function[1] if next_function else function,
                                   line.strip(), following if line.strip().startswith("//") else ""))
    require(sorted(actual_old) == sorted(OLD), ("historical inventory changed", actual_old))
    require(sorted(actual_new) == sorted(NEW),
            ("authorized inventory mismatch: expected seven NEW exceptions", actual_new))
    evidence = (root / DOC).read_text()
    for _, _, _, anchor, _ in NEW:
        require(f"## {anchor}\n" in evidence, ("missing evidence anchor", anchor))
    print("Inventory: 7 new exact exceptions; 7 historical annotations unchanged", flush=True)


def initialize_fixture_repository(fixtures, target):
    """Make the external fixture visible to Semgrep's default git-aware scan."""
    if not (fixtures / ".git").exists():
        subprocess.run(["git", "init", "-q", str(fixtures)], check=True)
    subprocess.run(
        ["git", "-C", str(fixtures), "add", "--", target.name],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["git", "-C", str(fixtures), "-c", "user.name=CI", "-c", "user.email=ci@example.invalid",
         "commit", "--quiet", "-m", "Add Semgrep control fixture"],
        check=True,
        stdout=subprocess.DEVNULL,
    )


def scanner_controls(output):
    """Use real remote rules, not a substitute rule that could mask drift."""
    with tempfile.TemporaryDirectory(prefix="semgrep-path-fixtures-") as directory:
        fixtures = Path(directory).resolve()
        if fixtures.is_relative_to(ROOT):
            raise ValueError("fixtures must be outside the source tree")
        raw = ('fn read(path: &str) -> std::io::Result<String> {\n'
               '    std::fs::read_to_string(path)\n}\n')
        annotated = (
            'fn read(path: &str) -> std::io::Result<String> {\n'
            f'    // nosemgrep: {RULE}\n'
            '    std::fs::read_to_string(path)\n}\n'
        )
        conversion = (
            'fn validate(path: &str) -> Result<(), Error> {\n'
            '    check_path(std::path::Path::new(path))\n}\n'
        )
        for name, source, disabled, expected in (
            ("positive", raw, False, 1),
            ("annotated", annotated, False, 0),
            ("disabled", annotated, True, 1),
            ("conversion", conversion, False, 0),
        ):
            target = fixtures / f"{name}.rs"
            target.write_text(source)
            initialize_fixture_repository(fixtures, target)
            argv = [
                "semgrep", "scan", "--project-root", str(fixtures),
                "--config", "p/rust", "--severity", "ERROR",
                "--json", "--error", "--metrics", "off", "--disable-version-check",
            ]
            if disabled:
                argv.append("--disable-nosem")
            result = subprocess.run(argv + [target.name], cwd=fixtures,
                                    capture_output=True, text=True, check=False)
            (output / f"{name}.json").write_text(result.stdout)
            (output / f"{name}.stderr").write_text(result.stderr)
            data = json.loads(result.stdout)
            scanned = {(fixtures / p).resolve() for p in data["paths"]["scanned"]}
            # 1.126 re-emits the result with --disable-nosem, but retains
            # is_ignored=true and exits 0 even with --error. Inspect JSON.
            active = [r for r in data["results"]
                      if disabled or not r["extra"].get("is_ignored", False)]
            require(scanned == {target}, (name, "file not scanned", scanned))
            require(not data["errors"], (name, data["errors"]))
            require(len(active) == expected, (name, "unexpected active findings", active))
            require(all(r["check_id"] == RULE for r in active), (name, active))
            expected_exit = 1 if expected and not disabled else 0
            require(result.returncode == expected_exit, (name, result.returncode))
            print(f"{name}: active={len(active)}, scanned={len(scanned)}, "
                  f"exit={result.returncode}, engine={data['version']}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    output = args.output_dir or Path(tempfile.mkdtemp(prefix="semgrep-path-evidence-"))
    require(not output.resolve().is_relative_to(ROOT), "evidence must be outside source tree")
    output.mkdir(parents=True, exist_ok=True)
    print(f"Evidence: {output.resolve()}", flush=True)
    scanner_controls(output)
    check_inventory()


if __name__ == "__main__":
    main()
