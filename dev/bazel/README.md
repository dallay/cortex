# Bazel workspace coverage

This directory contains coverage tooling for the Bazel adoption effort. The coverage contract
compares the declared Cargo workspace inventory with explicit Bazel target mappings or reviewed
exclusions. A matching package/target count is not evidence of coverage.

Cargo metadata is the source of truth for package and declared-target inventory. The current
workspace has 43 packages and 261 declared targets.

```sh
cargo metadata --locked --no-deps --format-version 1
```

The checker keys packages by repository-relative manifest path plus the exact package `id` from
Cargo metadata, and compares target name and sorted target kind. The explicit mapping is
`cargo_inventory.json`. Each inventory entry has exactly one Bazel label or a non-empty reviewed
exclusion reason. Labels are checked against `bazel query //... --output=label --lockfile_mode=error`.

Run the checker from any working directory with `python3 dev/bazel/check_inventory.py`. Use
`--format json` for deterministic JSON output. Tests use fixture query output and do not need a
successful Bazel query:

```sh
python3 -m unittest dev/bazel/test_check_inventory.py
```

## BUILD file generator

`generate_build_files.py` derives Bazel targets from the Cargo workspace inventory and writes the
`BUILD.bazel` files of the packages it owns.

```sh
python3 dev/bazel/generate_build_files.py                  # write the generated BUILD files
python3 dev/bazel/generate_build_files.py --check          # fail on drift, write nothing
python3 dev/bazel/generate_build_files.py --inventory dev/bazel/cargo_inventory.json
```

The generator reads `cargo metadata --locked --no-deps --format-version 1`, so the Cargo
workspace stays the single source of truth for packages, targets, dependencies, and features. The
output is deterministic: a second run produces byte-identical files. `--check` compares the
rendered content with the files on disk and reports drift without writing.

Every generated file starts with an `@generated` marker. The generator only rewrites files that
carry that marker. Hand-tuned `BUILD.bazel` files stay untouched and are reported as such:

- `crates/domain/rook-core`
- `crates/domain/shared-kernel`
- `crates/huginn/application`
- `crates/huginn/conformance`
- `crates/huginn/core`
- `crates/huginn/guards`
- `crates/huginn/outline-bindings`
- `crates/huginn/plugin-api`
- `crates/huginn/presentation`
- `crates/huginn/reference-extension`
- `crates/huginn/runtime`
- `crates/huginn/rust-outline-component`
- `crates/infrastructure/cache-memory`
- `crates/infrastructure/encryption-inmemory`
- `crates/infrastructure/models-catalog`
- `crates/infrastructure/observability`
- `crates/test-support`

The Huginn packages are hand-tuned because the generator does not model their `test-hooks` and
`test-driver` feature split, the Seatbelt sandbox tags of the shell tests, or the runfiles helpers
that resolve auxiliary binaries. The generator still derives their inventory entries, and
`--check` reports them as hand-tuned instead of writing them.

The generator unit tests pin the mapping rules:

```sh
python3 -m unittest dev/bazel/test_generate_build_files.py
```

## Bazel targets for this tooling

The tooling that maintains the Bazel graph is covered by the same suite as the code it generates:

| Target | Covers |
| --- | --- |
| `//dev/bazel:test_generate_build_files` | The unit tests of `generate_build_files.py` |
| `//dev/bazel:test_check_inventory` | The unit tests of `check_inventory.py` |

```sh
bazel test //dev/bazel:all --lockfile_mode=error
```

The targets delegate execution to the Python standard library test runner, the same way the
dashboard targets delegate to pnpm. They are not hermetic: the interpreter comes from the system,
not from Bazel, so they run with `local = True` and the `no-sandbox` tag, and they require a
Python 3.10 or newer interpreter on the machine. The helper resolves the interpreter explicitly
because Bazel runs tests with a minimal `PATH` and the system interpreter bundled with macOS is
older than the version the tooling supports.

### Mapping rules

- A Cargo library becomes a `rust_library` with `crate_root` at `src/lib.rs` and `srcs` from
  `glob(["src/**/*.rs"])`.
- A Cargo binary or example becomes a `rust_binary`. When the binary name collides with the
  library name of the same package, the Bazel target gets a `_bin` suffix and the Cargo crate name
  is preserved in `crate_name`. Examples link the package dev-dependencies, because Cargo compiles
  examples with them; binaries do not.
- A test that reads `CARGO_BIN_EXE_<name>` from any of its sources receives the `env` and
  `rustc_env` contract of the matching binary target, and depends on it. The variable is
  compile-time under Bazel, so a helper module must resolve the runfiles path before it spawns the
  binary.
- A Cargo library with `#[cfg(test)]` modules gets a `<crate>_unit_test` target that reuses the
  library through `crate`. A Cargo binary with `#[cfg(test)]` modules gets a `<bin>_test` target,
  because Cargo compiles a binary with the test harness as well.
- A Cargo integration test becomes a `rust_test` with the test file and its `mod` helper modules
  in `srcs`. A helper module declared with a relative `#[path]` that lives in another Bazel package
  is referenced by label, so its owning package must export the file. For example,
  `crates/huginn/presentation` exports the PTY harness that `apps/huginn` reuses.
- A Cargo build script becomes a `cargo_build_script` target.
- Registry dependencies resolve through `all_crate_deps()` and `aliases()` from `@crates`. Path
  dependencies inside the workspace are listed as explicit labels, for example
  `//crates/domain/rook-core:rook_core`.
- Optional dependencies are only linked when the effective feature set enables them. A target with
  `required-features` becomes a feature variant library named `<crate>_<feature>`.
- Files embedded with `include_str!`, `include_bytes!`, `rust-embed`, or a migration macro become
  `compile_data` inputs. Cross-package references require the owning package to export the file.
- The root package owns every file without a nearer `BUILD.bazel`. A file that the root `BUILD.bazel`
  exports through `exports_files` becomes a `//:<path>` compile data label. A file the root package
  does not export stays excluded and is reported.
- A test target listed in `NO_SANDBOX_TARGETS` receives the `no-sandbox` tag when the sandbox would
  distort the behavior under test.

## Coverage state

`python3 dev/bazel/check_inventory.py` reports the current state:

```text
Inventory coverage: 256/261 mapped; 5 reviewed exclusions.
```

- 154 non-Huginn Cargo targets: 151 map to verified Bazel labels and 3 carry reviewed exclusions.
- 107 Huginn Cargo targets, including `apps/huginn`: 105 map to Bazel labels and 2 carry reviewed
  exclusions.

The excluded targets are:

| Cargo target | Reason |
| --- | --- |
| `apps/rook` `build-script-build` | The build script shells out to the pnpm dashboard build. Bazel does not model the Node build step; the dashboard assets are supplied as compile data instead. |
| `apps/rook` `dotenv_startup` | The test clears the child environment and changes the child working directory before it launches `CARGO_BIN_EXE_rook`. Cargo supplies an absolute executable path; Bazel can only supply a runfiles-relative path, which the test cannot resolve from another working directory. |
| `crates/huginn/runtime` `wasm_outline` | The test requires the non-default `experimental-wasm` feature, which activates the optional `wasmtime` dependency. The generated `@crates` repository omits that crate because no default Cargo target activates the feature, so modeling the target requires feature-aware `crate_universe` generation in `MODULE.bazel`. |
| `crates/huginn/runtime` `wasm_output_validation` | Same reason as `wasm_outline`: the test requires `experimental-wasm` and `wasmtime` is not vendored. |
| `crates/muninn/c1-exporter` `export_minilm_c1` | The binary requires the non-default `cpu-f32` feature, whose optional `candle-*`, `safetensors`, and `tokenizers` dependencies are not vendored by `crate_universe`. |

The root `BUILD.bazel` now exports `dev/muninn/fixtures/**`. The generator detects those exported
root-package files and adds their labels as compile data. The `c1-exporter` unit-test target has the
`no-sandbox` tag because Bazel's macOS sandbox drops setuid/setgid mode bits set by the tests; the
tag lets the filesystem mode checks exercise native macOS behavior without weakening or skipping
the tests.

## Test parity

Parity was measured by running both build systems over the non-Huginn packages:

- Bazel: `bazel test //crates/application/... //crates/infrastructure/... //crates/muninn/...
  //apps/rook/... //apps/muninn/... //crates/domain/... //crates/test-support/...` reports 146 test
  targets passing, which is 2885 test cases, 0 failures, and 28 ignored tests.
- Cargo: `cargo test --workspace` with the 11 Huginn packages excluded reports 2886 passing library
  and integration test cases, 0 failures, and 28 ignored tests, plus 6 passing doc-test cases.

The case totals are the recorded baseline plus the 7 cases of the `transport-axum`
`csrf_session_binding` target added by the security remediation, and both build systems were run
again on that target. The target count is confirmed by the current full run: `just bazel-test`
reports 249 test targets, 0 failures, including the two `//dev/bazel` tooling targets.

The case comparison above still covers only the non-Huginn packages. Huginn is now modeled and
green under Bazel (`bazel test //crates/huginn/... //apps/huginn/...`), but it is not part of the
measured case-count comparison, so no case parity is claimed for it.

The remaining differences are known and reviewed:

- The `apps/rook/dashboard` Bazel targets `lint`, `typecheck`, and `vitest` carry the `manual` tag,
  so `//...` does not expand to them and the default suite leaves the dashboard to pnpm. Invoked
  explicitly, all three pass: `bazel test //apps/rook/dashboard:lint
  //apps/rook/dashboard:typecheck //apps/rook/dashboard:vitest`.
- Doc-tests are not modeled. `rules_rust` has no doc-test support, so the 3 doc-tests in
  `crates/muninn/observability` and the 3 doc-tests in `crates/infrastructure/providers-core` run
  under Cargo only.
- The excluded targets listed above account for the remaining difference of 1 test case: the
  `apps/rook` `dotenv_startup` integration test. The `crates/muninn/c1-exporter` `export_minilm_c1`
  binary carries no test case of its own.

This measures Cargo package/target declaration inventory only. It does not establish feature,
build-script, platform, artifact, or runtime test parity. `cargo metadata --no-deps` does not
establish equivalent dependency resolution or execution behavior.
