<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:27baf03b86d9229f2172bc70922e1dde5e6b7969a223302aa1c696e19d096539 -->

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:0d6aff0f4c3f7bf83834b26a013cd4f240af74aae44bb6c48b6a91b69f0776de -->

# Design: Huginn autonomous workspace permissions

## Technical approach

On macOS, `run_shell` must always prepare a restrictive Seatbelt SBPL profile and launch the command through `/usr/bin/sandbox-exec`. The isolation is mandatory, not configurable and has no exclusion nor bypass: if the profile cannot be prepared or applied, the operation fails without running the command. The profile starts from `(deny default)`, allows read/write only in the authorized workspace and a private temporary zone per invocation if the tests demonstrate it is necessary; it denies network and sockets. The shell and the processes it creates must inherit the same sandbox.

The authorization of the operation stays in `HostPolicy` and does not change because of the presence of the sandbox. `Deny` keeps denying; `Allow once` only approves that invocation and never removes the confinement. `HostPolicy` does not authorize removing it. This design adopts the Codex Seatbelt technical mechanism, not its elevation model, exceptions or consent to run outside the sandbox. Any future possibility of elevating permissions requires a separate change and independent consent; it is not part of this change. No persistent grants are created, read or honored. The guarantee is not extended to native tools nor to MCP, and no isolation is claimed on Linux or Windows. The mechanism depends on deprecated Apple interfaces; no future availability nor uniform support between versions is promised.

## Architecture decisions

### Decision: Keep the pure generator and the launch in the runtime adapter

**Choice:** Add a private module in `crates/huginn/runtime/src/` that converts a validated execution context into SBPL text. The generator performs no I/O and runs no processes; it receives already resolved roots and returns `Result<SandboxProfile, ProfileError>`. The runtime adapter prepares the paths/context, creates and cleans up temporary resources and invokes `sandbox-exec`. The orchestration stays in `run_shell`.

**Alternatives considered:** Put SBPL generation, path resolution and launch in one long function; add a sandbox port to `huginn-core` or a dedicated crate.

**Rationale:** The concrete policy is macOS-dependent infrastructure and is not a domain contract. A small pure generator makes adversarial testing easy without simulating sandbox application. A private module avoids a per-platform abstraction that is not needed today and preserves the inward dependencies per ADR-0012.

### Decision: Mandatory confinement; no adjustment nor bypass

**Choice:** On macOS, every `run_shell` invocation uses the sandbox. No configuration, exclusion, opt-out mode nor bypass is added. No existing approval can choose execution without confinement. Any future possibility of elevating permissions requires a separate change and independent consent; it is neither defined nor implemented here.

**Alternatives considered:** Add `sandbox = false`; treat ordinary shell approval as consent to run without sandbox; import the Codex elevation model.

**Rationale:** `HostPolicy` authorizes operations, but it does not authorize removing the operating system limit. Reusing the existing approval would be a silent bypass. The current spec requires confinement always and expressly excludes execution without a sandbox; the design aligns with that contract.

### Decision: Deliver the profile through a private temporary file and `-f`

**Choice:** Use the profile file interface (`sandbox-exec -f <profile-file>`) instead of passing the full SBPL with `-p`. Create the file with a random name in a private Huginn temporary directory, restrictive permissions (directory `0700`, file `0600`, exclusive creation without following links), keep it until the launcher finishes and request explicit cleanup on every controlled exit. Limit the size before writing and fail if it is exceeded.

**Alternatives considered:** `-p <text>`; stdin as the profile channel.

**Rationale:** `-p` exposes the profile to the system argument length limits and makes it unnecessary to carry potentially large SBPL through argv; stdin is not a profile channel documented in the change contract and competes with the shell's null stdin semantics. The file has a bounded lifetime and private permissions. On macOS the test verifies that `sandbox-exec` accepts `-f` on the target runners. No version compatibility is claimed from documentation or isolated generation.

The file is created and opened with safe file APIs (without user-controlled paths), written with `write_all` and dropped before the launch. No independent `close` error is required or tested: `std::fs::File` does not expose it through the safe API. If creation or writing fails before the spawn, the launcher and the shell are not started; the runtime attempts explicit cleanup of what was created and composes the primary error with any cleanup error. If the launcher could already start, every controlled return performs explicit cleanup and propagates its error. The guard's `Drop` still tries to delete the file and directory on a best-effort basis for abrupt cancellation of the future/owner, without a removal guarantee in case of an operating system failure. The profile only lets the launcher read the file in the load phase if the tests demonstrate that this read happens under restrictions; its temporary directory is not added as a general allowed root.

The composed errors must distinguish the phase: for a failure before the spawn, the text identifies the primary failure and the cleanup failure, and may state that the launcher did not start; for a launcher result, it preserves the primary result (including timeout/cancellation) and adds the cleanup error without stating that the shell did not run. The composition does not use a cleanup error to replace or hide the primary error. The current `open` and write errors must stop discarding the results of `remove_file`/`remove_dir`: the primary error is preserved, explicit cleanup is attempted and any secondary failure is attached before returning. A failure to create the directory has no created resources to clean up.

### Decision: `sandbox-exec` leads the process group

**Choice:** Build a `tokio::process::Command` for the absolute executable `/usr/bin/sandbox-exec`, add `-f`, the profile path and then `/bin/sh -c <command>`, and apply `CommandWrap::wrap(ProcessGroup::leader()).wrap(KillOnDrop)` to the launcher process. The process born as the group leader is therefore `sandbox-exec`; the shell and its descendants inherit both the group and the sandbox. Cleanup uses the `ChildWrapper` already used by `run_shell`.

**Alternatives considered:** Keep leading the group with `/bin/sh` directly; start the launcher outside the group and rely on signals to the PID.

**Rationale:** The directly monitored process becomes the launcher, so the signal behavior and `start_kill()` change with respect to the current code. It is not considered proven that `process-wrap` kills all descendants merely by wrapping the launcher. A macOS test must confirm the real leader/group process and that timeout, cancellation and launcher exit leave no live descendants. If that is not achieved, stop the change; do not weaken the requirement.

### Decision: Resolve roots and serialize paths conservatively

**Choice:** Resolve the workspace once before generating the profile, canonicalize it and require that it exists and is a directory. Generate separate read and write rules. The submitted profile allows reads under `/bin` and `/usr/lib` and `file-read-metadata` on the immediate parent of the workspace. Paths are emitted as SBPL literals through a single tested serialization routine, which escapes quotes and backslashes. Control characters have no portable escape in SBPL, so they are rejected before generating the profile and the workspace is refused; a root without a parent directory is also rejected. Paths are not interpolated into shell text, SBPL regex nor policy fragments.

**Rationale and limits:** `canonicalize` does not close races between checking paths and using them, nor does it protect on its own against concurrent renames/links. The tests cover symbolic links inside and outside the workspace, paths with metacharacters and target changes. The design does not claim resistance to every pathname/FD race that Seatbelt cannot demonstrate. Arbitrary descriptors are not inherited; stdio is configured explicitly and `pre_exec` nor user FDs are used as indirect access. With an ambiguous, non-canonicalizable or out-of-scope root, execution is rejected.

Global write access to `/tmp`, home, preferences, `/Applications` nor other workspaces is not allowed. The full workspace is the only persistent write root authorized by this profile; `cwd` is not allowed to widen the scope. The temporary zone, if needed, is created per execution and only that exact path receives permission. The policy does not include `allow network*`, local sockets, proxy nor optional services.

### Decision: Minimum runtime list based on runner evidence

**Choice:** Keep `(deny default)` and add only the rules needed to run `/bin/sh`, load dyld/system libraries and proven runtime resources, do `fork/exec` under the same policy, use `/dev/null`, keep the already connected stdio, access the workspace `cwd` and the designated private temporary zone. System rules will be read-only and as narrow as possible. Each additional permission requires a reproducible integration case and a review that it does not widen the boundary.

**Alternatives considered:** Copy the broad base policy of Codex or grant global system/service trees in advance.

**Rationale:** Codex is a mechanism reference, not authorization to import its allowlist. SBPL and dyld/runtime requirements can vary by macOS version. The concrete set is validated on the blocking macOS runners; no cross-version compatibility statement is made without test backing. A tool failure due to an unauthorized dependency is reported; it is not resolved by opening broad permissions without evaluation.

**Evidence and implementation adjustment (macOS 27.3, 2026-11-09):** the initial profile, without dyld bootstrap rules, ended in `SIGABRT` even for `/usr/bin/true` and `/bin/sh -c 'exit 0'` when invoked directly with `sandbox-exec`. Importing `dyld-support.sb` before the local deny rules allowed those processes to start and, with the existing rules, allowed shell read/write in the canonical root while denying an external marker. The system file identifies its rules as necessary for dyld bootstrap and is declared a private Apple interface, subject to change. The implementation imports only `dyld-support.sb`; it does not import `system.sb`, adds reads under `/bin` and `/usr/lib` and metadata of the immediate parent of the workspace, without network permissions. The exact operation that causes the initial abort was not isolated. This import is an explicit compatibility risk: availability, content and behavior must be checked on every supported macOS runner; any error keeps `run_shell` closed. It is not claimed
that the import alone proves complete confinement.

## Data flow and command

```text
ToolContext + command
  → validate/canonicalize workspace and prepare bounded context
  → pure builder: validated context → deny-default SBPL
  → private temporary file (-f; write_all before the spawn)
  → CommandWrap(ProcessGroup::leader, KillOnDrop)
  → /usr/bin/sandbox-exec -f PROFILE /bin/sh -c COMMAND
  → monitored process/group, bounded and drained stdout/stderr
  → explicit profile cleanup on every Result return; best-effort Drop only if the future/owner is discarded abruptly
```

The exact invocation uses separate arguments, without an extra `sh -c` to build the launcher:

```text
argv[0] = "/usr/bin/sandbox-exec"
argv[1] = "-f"
argv[2] = <private-profile-path>
argv[3] = "/bin/sh"
argv[4] = "-c"
argv[5] = <original command>
```

The launcher's `Command` sets `current_dir` to the validated workspace, `env_clear()`, restores only the `PATH` that the current contract selects, connects stdin to `Stdio::null()` and stdout/stderr to pipes. The shell inherits those values from the launcher. No additional environment variables are copied. The tasks must confirm whether any other variable is part of the current contract before changing it.

The launcher process is built with the fixed absolute executable; `sandbox-exec` is never looked up through `PATH`. Token cancellation and timeout keep the current selection around `wait()`. On both controlled exits, group termination is requested and the reader output is awaited/closed with the existing limits; every controlled return (success, non-zero exit, rejection, spawn error, timeout or returned cancellation) then goes through explicit profile cleanup. A cleanup failure is propagated. If a primary error also exists, both are reported; on post-spawn results the message does not state that the shell did not run. Only an abrupt discard before returning `Result` relies on best-effort `Drop`.

## Interfaces / minimum contracts

Private internal types, names subject to the final organization of the module:

```rust
struct SandboxRoots {
    workspace: PathBuf,       // canonical, existing, directory
    invocation_tmp: PathBuf,  // exact, private, optional only if proven necessary
}

struct SandboxProfile(String); // already validated SBPL; not exposed to inner layers

fn build_profile(roots: &SandboxRoots) -> Result<SandboxProfile, ProfileError>;
```

The profile layer is pure: it does not canonicalize, read files, run processes or decide approvals. Runtime owns canonicalization, the temporary file, the launcher and the lifecycle. No trait/port is added to the core, no configuration field for opt-out nor new permission/grant.

## Expected files

| File                                                                    | Action                    | Description                                                                                                                                                                                                                                                                              |
|-------------------------------------------------------------------------|---------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/huginn/runtime/src/tools.rs`                                   | Modify                    | Replace the direct `/bin/sh` executable with the macOS SBPL launcher and preserve the stdio/env/cwd configuration, bounded output, timeout, cancellation and group. The approval/definition text must no longer state "not sandboxed" on macOS; distinguish explicitly the platforms and the limits. |
| `crates/huginn/runtime/src/seatbelt.rs` (or equivalent private module) | Create                    | Pure SBPL constructor, escapes, profile limits and validation of already canonicalized roots. Only compile/use on macOS when appropriate.                                                                                                                                          |
| `crates/huginn/runtime/src/lib.rs`                                     | Modify if needed          | Declare the private module per existing conventions.                                                                                                                                                                                                                                   |
| `crates/huginn/runtime/tests/`                                         | Create/modify             | Unit and macOS integration tests isolated from the rest of the tests.                                                                                                                                                                                                                    |
| `apps/huginn/src/config.rs`, `apps/huginn/config.example.toml`         | No configuration change   | Do not add a sandbox setting, exclusion nor bypass.                                                                                                                                                                                                                                       |
| `docs/huginn/README.md`, `docs/huginn/architecture.md`, relevant ADR  | Modify                    | Document the macOS default, denied network, compatibility failures, Apple deprecation, platform limits and native/MCP exclusions.                                                                                                                                                 |
| `.github/workflows/ci.yml`                                             | Modify                    | Run the sandbox integration tests on a macOS runner on PR events and make them blocking. Use `test-multi` only if its PR trigger is widened without accidentally altering coverage; otherwise add a specific job.                                      |
| `crates/huginn/runtime/Cargo.toml`                                     | No change expected        | `process-wrap` is already a dependency. No new dependency is planned; any new requirement needs an architecture review.                                                                                                                                                            |

## Test strategy and evidence criteria

The SBPL generation test only demonstrates which text the builder produces; **it does not demonstrate that macOS applies the isolation**. Acceptance requires real tests on macOS with `/usr/bin/sandbox-exec`.

| Layer                                | Cases and method                                                                                                                                                                                                                                                                                                                                                                                               | Required result                                                                                                                                     |
|-------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------|
| Pure unit tests                      | Normal root, Unicode, quotes, backslash, rejected newlines/control characters, parentheses/commas and SBPL syntax; oversized paths; read/write separation; absence of network rules or broad roots. Adversarial cases verify that the data never closes/injects a rule.                                                                                                                                      | Valid output or error; never widen permissions through serialization.                                                                                        |
| Path runtime                         | Nonexistent workspace, file instead of directory, failed canonicalization, symlink to the outside, internal links, adversarial rename/symlink during preparation and inherited FDs.                                                                                                                                                                                                                         | Rejection when a safe scope cannot be determined; without assuming that canonicalize neutralizes TOCTOU.                                                    |
| Mandatory macOS integration          | Under the real launcher: read/write inside the workspace allowed; read/write of home/credentials, another workspace, external paths and undesignated temp denied; a `cwd` change does not widen permission; TCP/UDP, Unix sockets and other sockets denied. Probes with an exit code/control verify that the resource/operation exists and distinguish denial from a harness error.                        | Operating system enforcement evidence correlated with the process and the operation. A failure or ambiguous result blocks.                                              |
| Descendants and group                | The shell creates a grandchild, launches another allowed interpreter/executable and tries to access outside/network; verify the inherited sandbox. On timeout/cancellation the child leaves a marker if it is still alive; confirm that the launcher leads and the whole group terminates, even if they hold pipes.                                                                                                                                         | No descendant survives nor obtains more permissions.                                                                                                  |
| Fail-closed and cleanup              | Missing/not executable launcher, invalid profile, pre-spawn creation or write failure and launcher rejection. On pre-spawn failure, verify that the shell does not run and that the result preserves the primary and any cleanup error. Test failed cleanup directly on `SeatbeltProfileFile` in private tests; test the composition with a pure private helper, without a public seam nor `unsafe`. | Observable error; marker absent for pre-spawn failures; any cleanup error is preserved. No `File` close error is induced or required.   |
| Lifecycle and controlled exits       | cwd, selected PATH, null stdin, separated stdout/stderr and output limit; success, non-zero exit, spawn error, timeout and returned cancellation. Verify explicit cleanup on each return and propagation when cleanup fails, with an internal composition helper tested separately; do not add a public interface or dependency injection only to force failures.                         | Same `run_shell` contract; the combined errors preserve the primary result and cleanup, without stating that the shell did not run if it could have started. |
| Abrupt Drop                          | Discard the future/owner during the wait and confirm only the best-effort attempt of the resources' `Drop`.                                                                                                                                                                                                                                                                                                     | Successful removal or error propagation from `Drop` is not required; it is not presented as equivalent to a controlled exit.                          |
| CI/versions                          | Blocking macOS job on PR; record the observed OS version. Run on each major version that the project declares supported, according to the available runners.                                                                                                                                                                                                                                             | Do not mark the integration as approved if it did not run. Do not turn a failure/absence into a green skip.                                                            |

If the operating system or `sandbox-exec` is not available in a local environment, the integration test may produce an explicit `unsupported`/skip state with a visible reason; that result does not count as approval and must remain distinguishable from a pass. In required macOS CI, the absence, error or any dependency-related skip of the integration must fail the job. Unit tests may run on other platforms, but they do not replace macOS integration. The runtime tests must not reset persistent data.

The matrix tests run separately per major macOS version chosen by the project. The results only attest the observed version and configuration. Future support nor Apple endorsement is claimed.

## Migration and deployment

There is no data migration. The behavior change on macOS is intentional: the shell is always confined, so commands with external dependencies, network or paths outside the workspace can fail. Compatibility is not recovered through an exclusion option or sandbox-free execution. Linux/Windows keep their existing behavior without that implying isolation. There are no persistent grants to migrate. The implementation must not be launched without approved blocking macOS integration tests; an absence or skip of those tests blocks the launch.

A non-buildable policy, a missing executable or a policy application failure produce an error without running the shell. If the macOS matrix or the adversarial tests fail, the launch is stopped and the operation remains unexecuted (fail-closed), without automatic fallback. Later retiring the backend requires an explicit and reviewed change; the default cannot be silently reverted to sandbox-free execution. A code rollback must also not be presented as equivalent to isolation: until another validated implementation exists, `run_shell` will not be enabled without confinement.

## Risks, limits and pending decisions

- `sandbox-exec` and the Seatbelt interfaces are deprecated and can change or disappear. Confirm with real tests on each supported macOS version; do not make cross-version compatibility claims.
- The runtime allowlist is platform sensitive. Any added permission requires evidence and review; do not copy the broad list of Codex.
- SBPL policies based on paths do not eliminate races, aliases or descriptor access on their own. The tests cover representative attacks; no broader guarantee than the evidenced one is claimed.
- The semantics of `ProcessGroup::leader()` over the launcher and `ChildWrapper::start_kill()` must be confirmed experimentally. A failed integration blocks the implementation.
- The `Drop` cleanup cannot propagate failures and is best-effort; only the paths that return `Result` can perform explicit cleanup and propagate the observed error. The close error of `std::fs::File` is not observable through the safe API and is not a separate verification.
- The current `HostPolicy` does not authorize removing the confinement. The change copies the Seatbelt mechanism, not the Codex elevation model; any possible future elevation requires a separate change and independent consent. There is no bypass nor setting in this design.
- This sandbox covers only `run_shell` and its descendants. It does not cover native tools, MCP nor the other Huginn processes.
- There is no isolation claim for Linux/Windows, nor parity between platforms.
- Tests only on the SBPL generation do not constitute an enforcement test.

## Implementation sources

- `crates/huginn/runtime/src/tools.rs::run_shell`: current direct `/bin/sh -c` launch, workspace cwd, environment with `PATH`, null stdin, pipes, `ProcessGroup::leader`, `KillOnDrop`, cancellation/timeout and bounded draining.
- `crates/huginn/runtime/Cargo.toml`: `process-wrap` already available.
- `docs/huginn/architecture.md`; ADR-0006, ADR-0007 and ADR-0012.
- `exploration.md` of the change: comparison of the Codex implementation and Apple warnings. Codex is a pattern reference, not a permission template.

## Open questions

- [ ] The supported macOS matrix must be defined from product support and the available runners; this design does not set compatibility that has not been tested yet.
- [ ] The final minimum profile and any runtime/temporary permission must be closed with evidence on macOS runners; the design does not presume that the initial list is sufficient.

<!-- /coderabbit-ai-generated:v1 --><!-- /coderabbit-ai-generated:v1 -->
