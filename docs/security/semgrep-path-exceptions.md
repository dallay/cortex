# Targeted Path Exceptions in Semgrep

## Scope and Authorization

The user authorized exactly seven new exceptions for the rule
`rust.actix.path-traversal.tainted-path.tainted-path`, per expression rather than
per file. Lens and Mirror accepted the triage and residual risk formulation:
`ses_e828d89f0ffeUAnQyVDAzESwf7` and `ses_e828d899bffegiooi6Zl3B671x`.
Lens and Mirror granted `VERDICT CLEAN` to the entire tree rooted at `ee44ab8`.
This B1 unit rebuilds only the scanner on `origin/main` (`5945002`), without
F1/F2 hooks or guards. Its evidence is verified independently, not inherited.

No changes are made to I/O, validators, rules, severity, exclusions, or baseline.
An annotation only affects the immediately following expression. Changing the
expression, function, reason, link, or inventory requires a new review.
These exceptions do not authorize arbitrary paths supplied by the model.

In all four runtime cases, canonical validation blocks path traversal via
arguments within a stable tree. Concurrent substitution of files or symlinks
between validation and I/O remains an **unresolved** residual risk. There is
no atomic confinement. Metadata and stale content checks do not eliminate
that race condition. The exceptions do not claim otherwise.

## huginn-config

- File: `apps/huginn/src/config.rs`; function `Config::load`.
- Expression: `match std::fs::read_to_string(&resolved)`.
- Origin: operator option `--config` or fixed host configuration paths.
  Reading outside the workspace is intentional. Not a tool/model path.
- `PathBuf` represents a path; it does not sanitize or grant authorization by itself.

## huginn-ui-settings

- File: `apps/huginn/src/ui_settings.rs`; private function `load_or_init`.
- Expression: `fs::read_to_string(path)`.
- `persist_seeded` receives from the host the configuration save path or the
  prior configuration as a migration seed. The UI modifies `[ui]` values,
  it does not supply a filepath. No access is granted to model-provided paths.

## rook-usage-db

- File: `apps/rook/src/di.rs`; function `RookContainer::build`.
- Expression: `SqliteUsageRepository::new(Path::new(&config.database.db_path))`.
- `db_path` is operator SQLite configuration in the server composition,
  not an HTTP/model argument. It may reside outside a Huginn workspace.
- `Path::new` only converts the representation; it does not validate, sanitize, or perform I/O.
  The constructor opens the SQLite database authorized by that configuration.

## runtime-prepare

- File: `crates/huginn/runtime/src/tools.rs`; method `NativeTool::prepare`.
- Expression: `Some(tokio::fs::read_to_string(&resolved).await?)`.
- `resolve_path` canonicalizes the root and existing destination, or parent for
  new writes; requires root membership and rejects dangling symlinks.
  Original preview read also checks type and size.
- Guarantee limited to a stable tree; concurrent substitution unresolved.

## runtime-mutation

- File: `crates/huginn/runtime/src/tools.rs`; private function `execute_mutation`.
- Expression: `let current = match tokio::fs::read_to_string(path).await`.
- The caller `NativeTool::execute` re-resolves and rechecks the path
  against preparation before calling this helper. The helper verifies type,
  size, and original content to reject stale approvals.
- Guarantee limited to a stable tree; no atomic confinement or resolution
  for concurrent substitution between check and read/write.

## runtime-read

- File: `crates/huginn/runtime/src/tools.rs`; private function `execute_read`.
- Expression: `let text = tokio::fs::read_to_string(path).await?`.
- The same private execution caller revalidates the workspace path before
  dispatching the read. The helper verifies regular file and size limit.
- Guarantee limited to a stable tree; concurrent substitution unresolved.

## runtime-search

- File: `crates/huginn/runtime/src/tools.rs`; function `execute_search`.
- Expression: `let Ok(text) = tokio::fs::read_to_string(&file).await`.
- The root originates from the validated caller. `workspace_files` enumerates only
  regular entries/directories, omitting symlinks and dependency directories,
  without following symlinks. Before reading, `execute_search` verifies the size
  limit. Direct root file also originates from canonical resolution.
- Guarantee limited to a stable tree; enumerating without symlinks does not prevent
  subsequent concurrent substitution. No atomic confinement.

## Separate Historical Inventory

Seven previous annotations of the same rule remain intact. They are neither
approved nor reinterpreted in this resolution. Lines prior to change:

- `apps/rook/src/di.rs:495`: `is_in_memory_sqlite_target` (function annotation).
- `apps/rook/src/di.rs:529`: `resolve_api_key_secret`, conversion with `replacen`.
- `apps/rook/src/di.rs:531`: `resolve_api_key_secret`, direct conversion.
- `apps/rook/src/di.rs:582`: `resolve_api_key_secret`, reading the secret.
- `apps/rook/src/config.rs:453`: `RookConfig::load`, configuration read.
- `apps/rook/src/main.rs:328`: `load_config`, configuration load.
- `apps/rook/src/main.rs:343`: `load_config_with_path`, configuration load.

The checker compares identities by file/function/expression, not line numbers
or a global total of seven. It requires **7 new and 7 historical** separately.

## Reproducible Controls

Run with the Semgrep installed by CI:

```sh
python3 .github/scripts/check-semgrep-path-exceptions.py
```

To reproduce the CI engine without modifying the global installation, install Semgrep
1.126.0 with Python 3.12 and setuptools 80.9.0 in an isolated environment. Then run:

```sh
python3 .github/scripts/check-semgrep-path-exceptions.py \
  --output-dir "$EVIDENCE_DIR"
```

The script invokes the fixed `semgrep` executable from `PATH`; it does not accept an alternative
command or arguments. The evidence directory must reside outside source trees.
Without `--output-dir`, it preserves evidence in a temporary directory.

The script generates deliberately unsafe Rust only in an external temporary location.
With `p/rust` and severity ERROR it validates JSON, scanned files, and return codes:

1. Unvalidated read from an argument: one finding, return code 1.
2. Equivalent read with the exact annotation: zero active findings, return code 0.
3. The same read with `--disable-nosem`: finding reappears in JSON.
   In 1.126.0 it retains `is_ignored=true` and return code 0 even with `--error`;
   checking the return code alone is insufficient.
4. `Path::new` passed to a validator without read: zero findings in 1.126.0.
   This scanner observation does not make `Path::new` a sanitizer.

Preserves full JSON with rule version/metadata and stderr per case.
Remote rulesets are mutable: positive controls must fail if the rule ceases
to load. Local Semgrep 1.182.0 adds three warnings about `Path::new` conversions
not reproduced in CI 1.126.0; they have no suppression authorization.
Scanner regression testing does not replace native runtime boundary tests.

## Implementation Evidence

- RED, before annotation: full scan with the three CI rulesets,
  `--severity ERROR --error --json`: return code 1, seven original findings.
- Inventory RED, after passing all four scanner controls:
  return code 1, `authorized inventory mismatch: expected seven NEW exceptions`,
  new inventory empty. Seven historical annotations already matched.
- GREEN: the four controls yielded 1, 0, 1, and 0 results respectively;
  return codes were 1, 0, 0, and 0. Inventory: 7 new, 7 historical unchanged.
- GREEN B1: Semgrep 1.126.0, three CI rulesets and
  `--severity ERROR --error --json`: return code 0, zero findings, 226 targets
  (177 Rust, 47 YAML, 2 Dockerfiles). RED: return code 1, seven findings across
  the same targets. None belong to `target` or `node_modules`.
- Four `PartialParsing` warnings persist in `release.yml:159,181`: two rules
  attempt to parse PowerShell snippets as Bash. They match RED exactly.
  They are neither hidden nor presented as complete parsing. CI file selection
  is preserved, including pre-existing exclusions.
- Positive rule metadata: `r_id=66114`, `rule_id=zdUJ10`,
  `rv_id=1415500`, `version_id=44TbL2Y`, severity ERROR.
- Nine inventory Python regressions pass: exact identity, disappearance,
  expression/function/reason/link alteration, deleted historical annotation,
  additional annotation, and annotation without explicit rule.
- The tenth Python test checks the unittest invocation in CI. The same
  step runs unittest and real controls before the scan and count gate.
- B1 does not alter Rust behavior: only seven comment blocks.
  It does not incorporate or attribute F2 guard tests. Local validation includes fmt,
  actionlint, markdownlint, and workspace cargo check via the required hook.

B1 JSON and logs reside outside source trees, under `huginn-b1-evidence/` in the approved
temporary location: `baseline-red.json`, `controls/`, `complete-green.json`, and runner logs.
To repeat inventory tests:

```sh
python3 .github/scripts/test-semgrep-path-exceptions.py
```
