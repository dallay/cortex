# Point-in-time Semgrep path exceptions

## Scope and authorization

The original reviewed baseline authorized exactly seven new exceptions for rule
`rust.actix.path-traversal.tainted-path.tainted-path`, by expression and not by
file. Lens and Mirror accepted the triage and the residual-risk formulation:
`ses_e828d89f0ffeUAnQyVDAzESwf7` and `ses_e828d899bffegiooi6Zl3B671x`.
Lens and Mirror gave `VERDICT CLEAN` to the complete tree backed by `ee44ab8`.
This B1 unit rebuilds only the scanner over `origin/main` (`5945002`), without
F1/F2 hooks or guards. Its evidence is verified separately; it is not inherited.

PR #663 separately authorizes exactly seven additional sink-local exceptions
for the same rule in Candle's explicit-path I/O APIs. The checker retains the
original seven entries and historical inventory unchanged, and checks the seven
Candle entries in a separate closed inventory. This follow-up adds eight exact
Codex profile-store sinks to the application inventory: the original three
profile-store expressions plus five for validated profile path construction and
removal. Each entry fixes the file, function, exact sink, immediately adjacent
annotation, exact rule ID, and all nearby reason comments. Missing, additional,
moved, or modified annotations fail validation; other rule IDs in the three
covered Candle files also fail validation.
The seven APIs and their caller-boundary assessment are documented in
`third_party/candle-core/CORTEX-PATCH.md`. These are contextual API-contract
exceptions, not a directory or vendor exemption. Reassess before a new caller
passes lower-trust paths.

These annotations do not change the Semgrep rule, severity, exclusions, or
baseline. `save_account` and `remove_account` independently validate profile IDs
before constructing paths; valid UUID saves and disconnects retain their intended
behavior. An annotation affects only the immediately following expression.
Changing the expression, function, reason, link, or inventory requires a new
review. These exceptions do not authorize arbitrary model-supplied paths.

In the four runtime cases, canonical validation blocks traversal via
arguments in a stable tree. Concurrent substitution of files or links
between validation and I/O remains an **unresolved** residual risk. There is no
atomic confinement. Metadata and stale-content checks do not eliminate
that race. The exceptions do not claim otherwise.

## huginn-config

- File: `apps/huginn/src/config.rs`; function `Config::load`.
- Expression: `match std::fs::read_to_string(&resolved)`.
- Origin: operator's `--config` option or fixed host configuration paths.
  Reading outside the workspace is intentional. It is not a tool/model-supplied path.
- `PathBuf` represents a path; it does not sanitize it or grant authorization by itself.

## huginn-ui-settings

- File: `apps/huginn/src/ui_settings.rs`; private function `load_or_init`.
- Expression: `fs::read_to_string(path)`.
- `persist_seeded` receives from the host the configuration save path or the
  previous configuration as a migration seed. The UI changes `[ui]` values;
  it does not supply a filepath. No access to model-supplied paths is granted.

## rook-usage-db

- File: `apps/rook/src/di.rs`; function `RookContainer::build`.
- Expression: `SqliteUsageRepository::new(Path::new(&config.database.db_path))`.
- `db_path` is operator-supplied SQLite configuration in the server composition,
  not an HTTP/model argument. It may be outside a Huginn workspace.
- `Path::new` only converts the representation; it does not validate, sanitize, or perform I/O.
  The constructor opens the SQLite database authorized by that configuration.

## runtime-prepare

- File: `crates/huginn/runtime/src/tools.rs`; method `NativeTool::prepare`.
- Expression: `Some(tokio::fs::read_to_string(&resolved).await?)`.
- `resolve_path` canonicalizes the root and the existing target, or the parent for
  new writes; it requires membership in the root and rejects dangling links.
  The original read for the preview also checks type and size.
- Guarantee limited to the stable tree; concurrent substitution unresolved.

## runtime-mutation

- File: `crates/huginn/runtime/src/tools.rs`; private function `execute_mutation`.
- Expression: `let current = match tokio::fs::read_to_string(path).await`.
- The caller `NativeTool::execute` re-resolves and checks the path
  against the preparation before calling this helper. The helper checks type,
  size, and original content to reject a stale approval.
- Guarantee limited to the stable tree; there is no atomic confinement or
  resolution of concurrent substitution between checking and reading/writing.

## runtime-read

- File: `crates/huginn/runtime/src/tools.rs`; private function `execute_read`.
- Expression: `let text = tokio::fs::read_to_string(path).await?`.
- The same private execution caller revalidates the workspace path before
  dispatching the read. The helper checks for a regular file and size limit.
- Guarantee limited to the stable tree; concurrent substitution unresolved.

## runtime-search

- File: `crates/huginn/runtime/src/tools.rs`; function `execute_search`.
- Expression: `let Ok(text) = tokio::fs::read_to_string(&file).await`.
- The root comes from the validated caller. `workspace_files` enumerates only
  regular entries/directories, skips links and dependency directories,
  without following links. Before reading, `execute_search` checks the
  size limit. The direct root file also comes from canonical resolution.
- Guarantee limited to the stable tree; enumerating without links does not prevent a
  subsequent concurrent substitution. There is no atomic confinement.

## codex-profile-store

- File: `crates/infrastructure/providers-openai/src/codex/mod.rs`.
- `load_profiles`, `save_account`, and `remove_account` receive the store root created during startup
  from the operator's local `server.codex_accounts.state_dir` configuration.
  Startup requires an absolute, non-symlink directory with no group/other
  permissions, canonicalizes it, and creates/checks the application subdirectory
  with owner-only permissions. The Rook configuration also requires loopback host
  binding when Codex accounts are enabled.
- `load_profiles` enumerates only that private root. It validates each entry name
  as a UUID and checks that profile entries are directories, not symlinks, with
  private permissions before joining the fixed credentials filename.
- `save_account` receives an ID generated by `RequestId::new()` (UUID v4) from
  `start`; it now independently rejects anything that fails `valid_id` before
  joining it to the root. HTTP cancellation IDs are compared with the active
  attempt and are never used as filesystem paths. Encrypted data remains size
  limited, and profile/token files are created with owner-only permissions.
- `remove_account` independently validates the ID, uses `symlink_metadata` to
  reject symlinks and non-directories, and rejects profile directories with
  group/other permissions before removing only the matching profile. It syncs the
  canonical private root after the directory entry is removed. If a login is
  cancelled after persistence begins, the task completes the write, rechecks the
  active attempt under the state mutex, then removes the uncommitted profile.
- The directory sync calls open only the derived profile directory and private
  root. The exception is limited to these eight expressions; it does not exempt
  the file or grant access to request/model-supplied paths. A process running as
  the same OS user can modify that user's local state; this is outside the
  untrusted HTTP/model path boundary considered here.
- Regression tests verify that `save_account` and `remove_account` reject
  traversal/symlink paths, non-private profile directories are not removed, a
  disconnect removes only the selected account, and the deletion persists after
  restart. The inventory fixes all eight sinks, reasons, and this evidence anchor.

## Separate historical inventory

Seven previous annotations of the same rule remain unchanged. They are not
approved or reinterpreted in this resolution. Lines prior to the change:

- `apps/rook/src/di.rs:495`: `is_in_memory_sqlite_target` (function annotation).
- `apps/rook/src/di.rs:529`: `resolve_api_key_secret`, conversion with `replacen`.
- `apps/rook/src/di.rs:531`: `resolve_api_key_secret`, direct conversion.
- `apps/rook/src/di.rs:582`: `resolve_api_key_secret`, secret reading.
- `apps/rook/src/config.rs:453`: `RookConfig::load`, configuration reading.
- `apps/rook/src/main.rs:328`: `load_config`, configuration loading.
- `apps/rook/src/main.rs:343`: `load_config_with_path`, configuration loading.

The verifier compares identities by file/function/expression, not line
numbers. It requires **15 new** entries (the original seven plus eight Codex
profile-store sinks), **7 historical** entries, and the separate seven-entry
Candle inventory.

## Reproducible controls

Run with the CI-installed Semgrep:

```sh
python3 .github/scripts/check-semgrep-path-exceptions.py
```

To reproduce the CI engine without changing the global installation, install Semgrep
1.126.0 with Python 3.11 and setuptools 80.9.0 in an isolated environment. Then run:

```sh
python3 .github/scripts/check-semgrep-path-exceptions.py \
  --output-dir "$EVIDENCE_DIR"
```

CI installs the same toolchain with `actions/setup-python` pinned to CPython 3.11
and `python -m pip install semgrep==1.126.0 'setuptools<81'`. The interpreter is
pinned on purpose: runner images ship their own Python, and the images with Python
3.14 abort the pinned Semgrep CLI while it imports protobuf through opentelemetry.
The gate must not depend on which Python the runner image provides.
Renovate therefore ignores the `python-version` datasource, and the CI contract
test requires the workflow, the `just ci-semgrep` recipe, and the Renovate
configuration to agree on that interpreter. Changing the interpreter means
updating the Semgrep pin in the same review.

The script invokes the fixed `semgrep` executable from `PATH`; it does not accept an
alternative command or arguments. The evidence directory must be outside of source.
Without `--output-dir`, it keeps evidence in a temporary directory.

The script deliberately generates unsafe Rust only in an external temporary location.
Using `p/rust` and severity ERROR, it checks JSON, scanned files, and exit codes:

1. Unvalidated read from an argument: one finding, exit code 1.
2. Equivalent read with the exact annotation: zero active findings, exit code 0.
3. The same read with `--disable-nosem`: a result reappears in JSON.
   In 1.126.0 it retains `is_ignored=true` and exit code 0 even with `--error`;
   that is why checking the exit code alone is not sufficient.
4. `Path::new` passed to a validator with no read: zero findings in 1.126.0.
   This scanner observation does not turn `Path::new` into a sanitizer.

It keeps the complete JSON with rule version/metadata and stderr per case.
Remote rulesets are mutable: positive controls must fail if the rule stops
loading. Local Semgrep 1.182.0 adds three warnings about `Path::new`
conversions that CI 1.126.0 does not reproduce; they do not have suppression authorization.
The scanner regression does not replace the runtime's native boundary tests.

## Application evidence

- RED, before annotating: full scan with the three CI rulesets,
  `--severity ERROR --error --json`: exit code 1, seven original findings.
- Inventory RED, after passing the four scanner controls:
  exit code 1, `authorized inventory mismatch: expected seven NEW exceptions`,
  empty new inventory. The seven historical annotations already matched.
- GREEN: the four controls gave 1, 0, 1, and 0 results respectively;
  their exit codes were 1, 0, 0, and 0. Inventory: 7 new, 7 historical unchanged.
- GREEN B1: Semgrep 1.126.0, three CI rulesets, and
  `--severity ERROR --error --json`: exit code 0, zero findings, 226 targets (177 Rust, 47 YAML, 2 Dockerfiles). RED: exit code 1, seven findings over the
  same targets. None belongs to `target` or `node_modules`.
- Four `PartialParsing` warnings persist in `release.yml:159,181`: two rules
  try to analyze PowerShell fragments as Bash. They match exactly
  RED. They are neither hidden nor presented as complete parsing. The
  CI file selection is preserved, including its pre-existing exclusions.
- Positive rule metadata: `r_id=66114`, `rule_id=zdUJ10`,
  `rv_id=1415500`, `version_id=44TbL2Y`, severity ERROR.
- Nine Python inventory regressions pass: exact identity, disappearance,
  expression/function/reason/link change, removed historical annotation,
  additional annotation, and annotation without an explicit rule.
- The tenth Python test checks the unittest invocation in CI. The same
  step runs unittest and the actual controls before the scan and its count gate.
- B1 does not change Rust behavior: only seven comment blocks.
  It does not incorporate or attribute F2 guards tests. Local validation includes fmt,
  actionlint, markdownlint, and cargo check of the workspace via the mandatory hook.

The B1 JSON and logs are outside source, in `huginn-b1-evidence/` of the approved
temporary directory: `baseline-red.json`, `controls/`, `complete-green.json`, and runner logs.
To repeat the inventory tests:

```sh
python3 .github/scripts/test-semgrep-path-exceptions.py
```
