# Verification report: huginn-autonomous-workspace-permissions

**Version:** N/A (OpenSpec delta)  
**Date:** 2026-11-09  
**Current result:** PASS WITH WARNINGS  
**Scope:** technical verification of the implementation and of the reviewed contract; it is not QA acceptance.

## Executive summary

The current source satisfies the reviewed criteria of tasks 2.1–2.2. Every controlled exit of `run_shell_with_seatbelt_profile` obtains an execution result and calls `SeatbeltProfileFile::cleanup`; the composition preserves the primary error and the cleanup one when both exist. For open/write failures prior to the spawn, the deletion of what was created is attempted and the primary error is preserved along with the observable cleanup errors. The flow does not claim that the shell did not run when it could have run. `Drop` remains best-effort, and no unobservable close error is required through the safe API of `std::fs::File`.

The current package suite passed on macOS 27.3/build 26C5035r, and clippy, formatting, diff-check and the CI contract tests also passed. The result carries warnings because real enforcement was only observed in that configuration, remote CI and `just ci-local` were not run, and the isolated composition/cleanup tests are not equivalent to injecting an arbitrary operating system failure through the full launcher. That limit does not contradict the approved design, which contemplates testing the composition through a helper and the explicit cleanup directly.

The previous FAIL is kept as historical. The early historical note of 3.2 is already followed by the most recent dated close that documents the later evidence and marks the task; the history was not altered.

## Completeness

| Metric            | Value |
|--------------------|------:|
| Checklist entries |    23 |
| Complete          |    23 |
| Incomplete        |     0 |

The grouped entries (for example, 1.1–1.13) count as one checklist entry, as stated in `tasks.md`. Tasks 2.1 and 2.2 are marked `[x]` and match the approved contract. No pending tasks were detected.

## Build, tests and validations

### Run in this verification

- `cargo test -p huginn-runtime` — **exit 0** on macOS 27.3/build 26C5035r; `/usr/bin/sandbox-exec` exists and is executable. Results: lib 60/60; `adapter_security` 3/3; `approval_resource` 44/44; `coding_workflow` 34/34; `connections` 4 passed and 1 ignored for requiring external Keychain/D-Bus; `guardrails` 8/8; `host_secrets` 1/1; `inference_adapters` 31/31; `mcp_lifecycle` 2/2; `providers_integration` 9/9; `session_locks` 6/6; `sessions_persistence` 5/5; `skill_loading` 10/10; `skills_catalog` 9/9; `stream_persistence` 4/4; `tool_authoring` 4/4; `tool_lifecycle` 12/12; `transports` 6/6; WASM targets and doc-tests without cases. Zero failures.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings` — **exit 0**.
- `cargo fmt --all -- --check` — **exit 0**.
- `git diff --check` — **exit 0**.
- `node --test .github/scripts/ci-*.test.cjs` — **exit 0**, 309/309.
- Inspection of the current source: `run_shell`, `run_shell_with_seatbelt_profile`, `combine_shell_cleanup`, `cleanup_before_spawn`, `SeatbeltProfileFile::{cleanup,drop}`, profile creation/writing, the Seatbelt builder, the CI workflow, the spec, the design, the proposal and the tasks.

### Reviewed apply evidence

`apply-progress.md`, section "Reopen of 2.1–2.2 under the approved contract", records a real behavioral RED by temporarily reconstructing the previous behavior and restoring the implementation: the pre-spawn composition lost the secondary cleanup error, and the post-spawn one lost the timeout and the possibility that the shell had run. The composition tests and the successful result with failed cleanup passed afterwards. The record also reports 29 focused Seatbelt tests in lib and the same module in `approval_resource`, `cargo test -p huginn-runtime` exit 0 (60 lib, 44 `approval_resource`, the rest of the suites passed, one test ignored for an external requirement), clippy `-D warnings`, fmt and a successful diff-check. This apply evidence is distinguished from the commands re-run here.

### Omissions and adapters

- No separate build was run: `.agents/sdd/config.yaml` does not configure `build_command`; it is not presented as a pass.
- Coverage is not configured (`coverage_threshold: 0`); not applicable.
- `.agents/sdd/quality-runner.json` does not exist: `fallback`; there is no deterministic enforcement nor a runner envelope.
- Neither `just ci-local` nor a remote GitHub Actions run was executed. The local workflow tests passed and the macOS job `test-huginn-seatbelt` appears connected to the `CI / Required` evaluator; it is not claimed that the remote job passed.
- Neither QA nor archive was run.

## Compliance matrix

| Requirement / scenario                                                      | Evidence and result                                                                                                                                                                                                                                                                                                                                                                                                                                                |
|----------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 2.1: RED for the pre-spawn and post-spawn composition                        | **COMPLIANT.** Behavioral RED evidence recorded in apply: when temporarily restoring the previous composition, each test failed on the information it had to preserve; the corrected implementation was restored and the tests passed. It is not an inference based only on the existence of tests.                                                                                                                                                                       |
| 2.2: explicit cleanup for every controlled return                            | **COMPLIANT.** The source obtains the `Result` of `run_shell_with_launcher`, invokes `profile_file.cleanup()` and composes both. It covers success, a non-zero status/rejection represented as an observable output, a spawn error, timeout, cancellation and the read/execution errors returned by the helper. The focused test runs inside `cargo test -p huginn-runtime` and the helper composition tests passed.                                                       |
| Post-spawn: propagate the cleanup error without claiming that the shell did not run | **COMPLIANT.** `combine_shell_cleanup` keeps the primary error plus the cleanup failure; on a successful result with a cleanup error and in the combined case it emits "may have run". The tests `post_spawn_error_composition_preserves_primary_and_possible_execution` and `post_spawn_success_with_cleanup_error_does_not_claim_shell_was_skipped` passed.                                                                                                                          |
| Pre-spawn: do not start the shell; cleanup and preserve the primary/secondary error | **COMPLIANT.** The directory creation, the open and the write precede the launcher. A failed `open` and `write_all` invoke the explicit cleanup of the created resources and `cleanup_before_spawn` preserves both errors; an mkdir failure leaves no created resources. The tests cover a creation failure due to a non-directory parent, a failing writer, the size, the error composition and a failed spawn without a marker. It is not claimed that an arbitrary OS failure was injected end-to-end. |
| Abrupt Drop                                                               | **COMPLIANT.** `Drop` attempts `remove_file`/`remove_dir` and discards their errors, consistent with best-effort and without a guarantee on OS failure. This path is not turned into a controlled exit nor is a successful unlink required.                                                                                                                                                                                                                                                 |
| File write and close                                                      | **COMPLIANT.** The production write uses `write_all`; the failing writer test passes. No independent `close` error of `std::fs::File` is required, since it does not expose that error through the safe API.                                                                                                                                                                                                                                                                        |
| macOS isolation, deny-default policy and mandatory launcher                  | **PARTIAL / observed scope.** The real tests executed by `run_shell` use `/usr/bin/sandbox-exec`; the inspected profile imports `dyld-support.sb`, then applies `(deny default)`, limits the paths and does not allow network/socket. The real enforcement suite passed on this host. It is not generalized to other/future macOS versions.                                                                                                                                     |
| Launcher/profile failures and no pre-spawn execution                         | **COMPLIANT for the tested cases.** The invalid profile, the missing/not executable launcher, the creation/writing and the size cases have tests that pass; the markers are absent in the corresponding cases. It is not extrapolated to all possible OS failures.                                                                                                                                                                                                                 |
| Policy/approval, no grants/bypass                                          | **COMPLIANT according to inspection and apply evidence.** There is no opt-out, fallback nor bypass on macOS; no grants are created/read. The application tests `Deny` and `Allow once` are recorded in apply-progress. Kernel enforcement is not attributed to those tests.                                                                                                                                                                                                                      |
| Native tools/MCP outside the guarantee                                     | **COMPLIANT according to the inspected scope and documentation.** The guarantee is limited to `run_shell` and its descendants.                                                                                                                                                                                                                                                                                                                                                    |
| No Linux/Windows parity                                                    | **COMPLIANT according to inspection.** The Seatbelt launcher is conditioned on macOS and equivalent isolation is not declared on other platforms.                                                                                                                                                                                                                                                                                                                                 |
| Lifecycle and CI                                                          | **COMPLIANT in the tested cases, limited by the environment.** The current suite includes cwd/PATH/stdin/streams, limits, timeout/cancellation, descendants, paths and protocols; local CI tests 309/309. Enforcement observed only on macOS 27.3/build 26C5035r, without remote CI.                                                                                                                                                                                                |

**Summary:** scenarios 2.1–2.2 comply with the reviewed contract with runtime/helper tests and a review of the production wiring. The use of a helper to test the composition, and the direct cleanup test, are explicitly foreseen by the design; an arbitrary injected unlink failure end-to-end is not required. The evidence does not claim that such an OS failure was tested through the full launcher.

## Structural correction and consistency with the design

| Decision                                                         | Status                           | Evidence                                                                                                                                 |
|------------------------------------------------------------------|----------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| Pure and private builder; runtime preparation of the root           | Compliant                         | `seatbelt.rs` separates generation/policy from the runtime flow; `run_shell` uses the canonical root for the profile and `current_dir`.                 |
| Fixed `/usr/bin/sandbox-exec`, `-f`, separate argv; fail-closed   | Compliant                         | The macOS callsite fixes the launcher and the arguments. There is no sandbox-free fallback for macOS.                                                     |
| Private temporary profile, restrictive permissions, limit and cleanup | Compliant                         | Directory `0700`, file `0600`, `create_new`, 64 KiB limit, `write_all`, explicit cleanup on a controlled exit and best-effort `Drop`. |
| Profile security                                                  | Compliant in the proven evidence | `(deny default)`, without network/socket rules nor broad roots added. Real enforcement only on the indicated probes and host.              |
| Group/descendant processes                                     | Compliant in the observed cases | `ProcessGroup::leader`, `KillOnDrop`; timeout/cancellation tests pass. The claim is not widened beyond those cases.                      |
| Expected files and CI                                          | Compliant                         | Runtime, docs, example config, ADR and workflow correspond to the design. The local workflow tests passed; there was no remote execution. |

## TDD audit

| Metric                         | Status                                                                                                                                                                                                |
|---------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Applicability                   | Runtime behavior: TDD applies.                                                                                                                                                                |
| RED→GREEN                       | Confirmed in apply for the pre/post-spawn error composition through a temporary reconstruction of the previous behavior; the asserts failed because of the incorrect behavior and passed once the solution was restored. |
| Refactor / later validation     | The focused Seatbelt suite and the full package suite passed; the code centralizes the composition in private shared helpers.                                                                     |
| Persisted evidence              | Yes, in `apply-progress.md`; the report distinguishes the recorded run from the new verify runs.                                                                                         |

## Findings

### CRITICAL

None for the reviewed contract of 2.1–2.2.

### WARNINGS

1. Real enforcement verified only on macOS 27.3/build 26C5035r. The Seatbelt interface is deprecated and the `dyld-support.sb` import depends on Apple; no future compatibility nor compatibility with other versions is claimed.
2. The composition tests synthesize errors to verify the error preservation; the cleanup test checks propagation directly. An arbitrary OS unlink failure within a full launcher run has not been provoked. The reviewed design allows this test separation, and the inspected implementation wires cleanup to every controlled return.
3. There was no remote CI, no `just ci-local`, no separate build nor deterministic runner. The result is not equivalent to a full CI.
4. One test is ignored for an external Keychain or D-Bus requirement. It does not correspond to the modified area.

### SUGGESTIONS

None.

## Verdict

**PASS WITH WARNINGS** — 2.1–2.2 satisfy the reviewed contract and the 23 checklist entries are complete. The warnings limit the platform scope and the operational evidence; they do not invalidate the compliance of those tasks. This report does not replace independent QA acceptance nor authorize archive by itself.
