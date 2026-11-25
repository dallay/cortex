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
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "load_profiles",
     "for file in fs::read_dir(root)? {", "codex-profile-store",
     "Operator-configured state root is canonicalized and owner-private before enumeration."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "save_account",
     "fs::File::open(&profile)?.sync_all()?;", "codex-profile-store",
     "The profile ID is a generated UUID v4 and validated before any profile path is joined."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "save_account",
     "fs::File::open(root)?.sync_all()", "codex-profile-store",
     "Operator-configured state root is canonicalized and owner-private before directory sync."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "save_account",
     "let profile = root.join(id);", "codex-profile-store",
     "The profile ID is validated as a UUID-shaped path component before joining the private root."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "save_account",
     "fs::DirBuilder::new().mode(0o700).create(&profile)?;", "codex-profile-store",
     "The joined profile path is confined to the private root by the validated profile ID."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "remove_account",
     "let profile = root.join(id);", "codex-profile-store",
     "The account ID is validated and the root is private and canonicalized before joining."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "remove_account",
     "fs::remove_dir_all(&profile)?;", "codex-profile-store",
     "The profile path is derived from a validated ID inside the owner-private root."),
    ("crates/infrastructure/providers-openai/src/codex/mod.rs", "remove_account",
     "fs::File::open(root)?.sync_all()", "codex-profile-store",
     "The canonical owner-private root is opened only to sync its directory entry."),
)
# Seven separately approved explicit-path library APIs. Reasons must remain sink-local.
# File, enclosing function, exact sink line, exact consecutive reason comments.
CANDLE = (
    ("third_party/candle-core/src/npy.rs", "read_npy",
     "let mut reader = File::open(path.as_ref())?;",
     ("Explicit caller-selected input file, not a root-confined API. Callers must authorize untrusted paths.",)),
    ("third_party/candle-core/src/npy.rs", "read_npz",
     "let zip_reader = BufReader::new(File::open(path.as_ref())?);",
     ("Explicit caller-selected archive; member names stay in memory, not host filesystem paths.",
      "Callers must authorize untrusted paths; this general archive loader does not confine a root.")),
    ("third_party/candle-core/src/npy.rs", "read_npz_by_name",
     "let zip_reader = BufReader::new(File::open(path.as_ref())?);",
     ("Explicit caller-selected archive; member names stay in memory, not host filesystem paths.",
      "Callers must authorize untrusted paths; this general archive loader does not confine a root.")),
    ("third_party/candle-core/src/npy.rs", "write_npy",
     "let mut f = File::create(path.as_ref())?;",
     ("Explicit caller-selected output file, not a root-confined API. Callers must authorize untrusted paths.",)),
    ("third_party/candle-core/src/npy.rs", "write_npz",
     "let mut zip = zip::ZipWriter::new(File::create(path.as_ref())?);",
     ("Explicit caller-selected output archive; member names are ZIP metadata, not host output paths.",
      "Callers must authorize untrusted paths; this general archive writer does not confine a root.")),
    ("third_party/candle-core/src/quantized/imatrix_file.rs", "load_imatrix",
     "let mut file = File::open(&fname).map_err(|e| {",
     ("Explicit caller-selected imatrix file; parsed entry names are not filesystem paths.",
      "Callers must authorize untrusted paths; this general file loader does not confine a root.")),
    ("third_party/candle-core/src/safetensors.rs", "load",
     "let data = std::fs::read(filename.as_ref())?;",
     ("Explicit caller-selected input file, not a root-confined API. Callers must authorize untrusted paths.",
      "Cortex MiniLM uses verified byte buffers instead of this path-based loader.")),
)
# Historical annotations: frozen inventory, NOT a new approval of their rationale.
OLD = (
    ("apps/rook/src/di.rs", "is_in_memory_sqlite_target",
     f"// nosemgrep: {RULE}", "fn is_in_memory_sqlite_target(path: &str) -> bool {"),
    ("apps/rook/src/di.rs", "resolve_api_key_secret",
     f"std::path::PathBuf::from(db_path.replacen('~', &home, 1)) // nosemgrep: {RULE}", ""),
    ("apps/rook/src/di.rs", "resolve_api_key_secret",
     f"std::path::PathBuf::from(db_path) // nosemgrep: {RULE}", ""),
    ("apps/rook/src/di.rs", "load_or_create_persisted_secret",
     f"let s = std::fs::read_to_string(secret_path) // nosemgrep: {RULE}", ""),
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
    actual_old, actual_new, actual_candle = [], [], []
    for filename in sorted(set(files) - {""}):
        if {"target", "node_modules"}.intersection(Path(filename).parts):
            continue
        lines = (root / filename).read_text().splitlines()
        function = ""
        for index, line in enumerate(lines):
            match = re.search(r"\bfn\s+(\w+)\s*(?:<|\()", line)
            if match:
                function = match[1]
            annotation = re.search(r"//\s*no(?:semgrep|sem)\b(.*)", line)
            if not annotation:
                continue
            require(annotation[1].strip().startswith(":") and annotation[1].strip()[1:].strip(),
                    ("unscoped suppression is not authorized", filename, index + 1))
            candle_file = any(row[0] == filename for row in CANDLE)
            if candle_file:
                require(line.strip() == f"// nosemgrep: {RULE}",
                        ("unapproved Candle rule", filename, index + 1))
                candidates = [row for row in CANDLE if row[:2] == (filename, function)]
                require(len(candidates) == 1, ("unapproved Candle function", filename, function))
                _, _, sink, reasons = candidates[0]
                require(index >= len(reasons) and
                        tuple(s.strip() for s in lines[index - len(reasons):index]) ==
                        tuple(f"// {reason}" for reason in reasons),
                        ("Candle reason changed", filename, function))
                require(index + 1 < len(lines) and lines[index + 1].strip() == sink,
                        ("Candle annotation detached from sink", filename, function))
                actual_candle.append(candidates[0])
                continue
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
    require(sorted(actual_candle) == sorted(CANDLE),
            ("Candle inventory mismatch: expected exactly seven exceptions", actual_candle))
    require(sorted(actual_old) == sorted(OLD), ("historical inventory changed", actual_old))
    require(sorted(actual_new) == sorted(NEW),
            ("authorized inventory mismatch: expected fifteen NEW exceptions", actual_new))
    evidence = (root / DOC).read_text()
    for _, _, _, anchor, _ in NEW:
        require(f"## {anchor}\n" in evidence, ("missing evidence anchor", anchor))
    print("Inventory: 15 new exact exceptions; 7 Candle exact exceptions; "
          "7 historical annotations unchanged", flush=True)


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
            require(result.stdout.strip(), (name, "semgrep produced no output", result.stderr))
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
