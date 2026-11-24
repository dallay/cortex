# Bazel bootstrap

Bazel is being introduced as a candidate build and test orchestrator for Cortex. This is an
experimental bootstrap, not full adoption. Cargo and pnpm remain operational and authoritative
for their dependency graphs. No existing CI gate is removed, weakened, or delegated to Bazel: CI
gains only an informational affected-crate classifier and a separate advisory workflow.

The Bazel workflow is advisory: it reports status, and a failure in it does not block a merge.

## Prerequisites

Install Bazelisk and run `bazel` from the repository root. The checked-in `.bazelversion` selects
Bazel 9.3.3. Bazel downloads the pinned Rust toolchain and resolves Rust crates from
`Cargo.Bazel.lock` through `rules_rust` and Crate Universe. While `Cargo.lock` remains the
authoritative lockfile for Cargo workflows, `Cargo.Bazel.lock` is authoritative for Bazel and
enables non-default optional features (such as `wasmtime`) without altering the standard Cargo
dependency graph. The generated `MODULE.bazel.lock` pins Bazel modules and extension resolution.

The dashboard is built hermetically by Bazel using Vite 7 (Rollup-based). Dependencies are
resolved from `pnpm-lock.yaml` through `npm_translate_lock`, and the build runs in the Bazel
sandbox. Dashboard tests (vitest, typecheck) are part of `//...` expansion with no `manual` tag.

## Commands

```sh
just bazel-build-core   # build huginn-core only
just bazel-test-core    # unit and integration tests of huginn-core
just bazel-build        # build every modeled target, dashboard assets included
just bazel-test         # run every modeled test outside the manual tag, without cached results
```

For direct inspection, use the same commands the recipes run:

```sh
bazel query //... --output=label --lockfile_mode=error
bazel test //... --lockfile_mode=error --test_output=summary
```

`--lockfile_mode=error` makes these commands fail if module resolution would change the tracked
lockfile. To intentionally update Bazel dependencies, review changes to `MODULE.bazel.lock` and run
`bazel mod deps --lockfile_mode=update`; do not update Cargo or pnpm lockfiles as a side effect of
this bootstrap.

For Cargo manifest changes, refresh the Crate Universe graph while keeping existing crate versions:

```sh
CARGO_BAZEL_REPIN=workspace bazel mod deps --lockfile_mode=update
env -u CARGO_BAZEL_REPIN bazel mod deps --lockfile_mode=update
```

Review `Cargo.Bazel.lock` and `MODULE.bazel.lock` after the refresh. The second command runs without
the temporary repin setting so its value is not recorded in `MODULE.bazel.lock`.

## What the graph models

The generator in `dev/bazel/` derives one `BUILD.bazel` per Cargo package from
`cargo metadata --locked --no-deps --format-version 1`, so the Cargo workspace stays the single
source of truth for packages, targets, dependencies, and features. Hand-tuned `BUILD.bazel` files
are never overwritten.

The declared inventory is tracked in `dev/bazel/cargo_inventory.json`: every Cargo target has
either an explicit Bazel label or a reviewed exclusion reason. All 43 workspace packages now have
a generated or hand-tuned `BUILD.bazel`, including the Huginn CLI and its tests. The inventory
maps 264 of 265 Cargo targets; the remaining 1 has a reviewed exclusion. `dev/bazel/README.md`
documents the mapping rules, current coverage, and the limits of the measured comparison against
Cargo.

## Boundaries

- Seatbelt tests on Huginn runtime are `local` and `exclusive`. They drop
  Bazel's darwin-sandbox so `/usr/bin/sandbox-exec` can run, and they do not
  overlap other tests. The rest of the suite may run in parallel; do not restore
  `--local_test_jobs=1` (that flag serializes every local test).
- No remote cache or remote execution is configured.
- CI and release workflows still use their existing Cargo, pnpm, and Docker commands. The
  `Bazel (non-blocking)` workflow is advisory and never participates in the required checks.
- Do not claim performance improvement from this bootstrap. It has no comparative benchmark.
- The measured comparison against Cargo is a snapshot recorded in `dev/bazel/README.md`. It covers
  declaration inventory and the recorded test run; it does not establish feature, build-script,
  artifact, or platform parity.
- Doc-tests are not modeled, because `rules_rust` has no doc-test support.
- The dashboard is hermetic: Vite 7 (Rollup-based) builds in the Bazel sandbox with
  dependencies resolved from `pnpm-lock.yaml`. Local dev still uses pnpm.
- Expand targets only after comparing features, build scripts, generated inputs, test inventory,
  runtime resources and platform support against the existing build.
