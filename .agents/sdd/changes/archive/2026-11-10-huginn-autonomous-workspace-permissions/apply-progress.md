# Apply progress — launcher/lifecycle (tasks 2.1–2.4)

Date: 2026-11-09. Pre-existing dirty worktree with the same implementation; its changes were neither reverted nor replaced.

## Executed scope

macOS tests were added in `crates/huginn/runtime/src/tools.rs` for cwd, selected PATH, stdin EOF, stdout/stderr separation, high size output that is bounded and drained, and a workspace preparation failure that does not start the command. The separate arguments of the existing launcher and the timeout/cancellation/ProcessGroup cycle were preserved.

## TDD evidence

No valid RED of the behavior was observed: the first attempt failed because of incorrect newline literals in the assertions; the second detected an overly strict expected format and a fixture command that depended on `yes`/`head`, not allowed by the profile; the next attempt timed out by emitting 2 MB using builtins in 10 s. The assertions and the fixture were fixed, but these failures do not count as a product RED. No launcher override, profile nor filesystem failure was added to production only to force a RED.

The limit is verified with a 100 KB output per stream: `run_shell_bounds_captured_output_while_draining_both_streams` passed and the final output did not exceed `MAX_OUTPUT_BYTES`, with content from both streams. The existing code bounds each vector during the drain and keeps reading until EOF.

## Commands and results

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: first run with a faulty fixture/assertions, it failed; the corrected run passed (7 unit tests in lib and 7 in `approval_resource.rs`; the remaining integrations were filtered without executed cases).
- `cargo test -p huginn-runtime`: passed without a filter. 38 unit tests, 3 adapter_security, 22 approval_resource, 34 coding_workflow, 4 connections (1 ignored), 8 guardrails, 1 host_secrets, 31 inference_adapters, 2 mcp_lifecycle, 9 providers_integration, 6 session_locks, 5 sessions_persistence, 10 skill_loading, 9 skills_catalog, 4 stream_persistence, 4 tool_authoring, 12 tool_lifecycle, 6 transports and doc-tests (0); no failures.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Limits/blockers of this delivery

- The absent/not executable launcher, the profile rejection or the creation/write/close/cleanup failures with a marker were not validated adversarially. The production executable is fixed and there is no existing safe seam to inject those failures. No production override was added only for tests; the coverage of 2.1 and part of 2.2 remain blocked.
- `SeatbeltProfileFile::drop` ignores the `remove_file`/`remove_dir` errors; therefore, a successful cleanup is not an observable condition of the operation and the fail-closed requirement on a cleanup error is not demonstrated.
- The invalid workspace test demonstrates a failure before the spawn coming from `prepare_workspace`, but it does not simulate an OS spawn error nor a profile application failure.
- The output respects a bounded memory limit during capture. The stdout/stderr capture failures were not tested simultaneously nor all the limit variants.
- The additional tests characterize behavior that was already present; since there is no valid RED and no product change is made, 2.1–2.4 are not marked as complete in `tasks.md`.

## Status

Partial. Tasks 2.1–2.4 remain unmarked. Next step: define a test seam that does not change the production surface or agree on how to validate the real launcher/profile errors; do not advance to tasks 3.x nor 4.x from this slice.

## Scoped review 2.1–2.2 (2026-11-09)

- `sandbox_exec_rejects_invalid_profile_before_shell_can_write_marker` was added in the macOS test module. It directly invokes the real `/usr/bin/sandbox-exec` with an invalid SBPL file and a shell command that would write a marker. The process returned code 65, reported `unbound variable` next to the path `invalid.sb`, and the marker did not appear. The parser rejection was unambiguous. The test does not use a configurable launcher nor alter the production profile/environment.
- First focused run: it failed only because the assertion expected the word `profile`/`syntax`, while macOS 27.3 reports `unbound variable`; it does not count as a product RED. After correcting the assertion to the observed diagnostic, the focused test passed. No production logic change was made; therefore, no TDD RED/GREEN cycle is claimed for product behavior.
- This test attests the rejection of the invalid file by the real launcher and the absence of shell execution in that case. It does not prove that `run_shell` delivers the invalid file, that it translates such a rejection to `Err`, nor spawn or filesystem failures. No helper is factored out nor a launcher override added: the rejection is tested against the real fixed binary, and tasks 2.1–2.2 remain incomplete.
- Cleanup: `SeatbeltProfileFile::drop` continues to be best-effort, since `Drop` cannot propagate errors. The test does not force unlink failures. No absolute removal guarantee is claimed if the operating system rejects the unlink. The explicit cleanup with propagation for controlled exits is still unimplemented/unverified; drop/cancellation keeps only best-effort.
- SBPL, grants, permissions nor tasks 2.3–2.4 or later phases were modified. `tasks.md` is left without checks for 2.1–2.2: the evidence covers only a part of 2.1.

### Validation of this review

- `cargo test -p huginn-runtime sandbox_exec_rejects_invalid_profile_before_shell_can_write_marker -- --nocapture`: passed (1 lib test; the filter also found the test replicated by integration).
- `cargo test -p huginn-runtime`: passed without a filter (39 unit tests; integration and doc-tests passed; 1 test ignored for an external requirement).
- `cargo fmt --all -- --check`: passed after applying `cargo fmt --all`.
- `git diff --check`: passed.

## Scoped diagnostic of UDP/TCP/Unix sockets (2026-11-09)

### Harness change

- Only the UDP test was changed in `crates/huginn/runtime/src/tools.rs`: the host listener and the sandbox listener now do `recv_from` in a loop with a deterministic timeout of 3 s, accumulate each datagram and finish on receiving exactly `udp-probe` or on the deadline expiring. Both results are logged in `--nocapture`.
- The host control received `[[88], [88], [88], [88], [117, 100, 112, 45, 112, 114, 111, 98, 101]]`, that is, four `X` datagrams and then `udp-probe`. The control counts: real payload received. No `nc`/stdin alternative was tried, because the control did transmit the expected payload.

### Results

- **TCP:** `cargo test -p huginn-runtime run_shell_seatbelt_denies_tcp_connect_after_host_control_succeeds -- --nocapture` passed in lib and `approval_resource`. Host: `Connection to 127.0.0.1 port <port> [tcp/*] succeeded!`; the sandbox uses `nc -v -w 1`, returns normal status 1 and `nc: connectx to 127.0.0.1 port <port> (tcp) failed: Operation not permitted`; the sandbox listener did not accept. The independent read error `/private/var/select/sh` also appears.
- **UDP:** `cargo test -p huginn-runtime run_shell_seatbelt_denies_udp_datagram_after_host_control_succeeds -- --nocapture` passed in lib and `approval_resource`. The host control received the four `X` and the exact payload. The sandbox uses `nc -v -u -w 1`, returns normal status 1 and `nc: connectx to 127.0.0.1 port <port> (udp) failed: Operation not permitted`; the listener received no datagrams. The independent read error `/private/var/select/sh` also appears.
- **Unix/local:** `cargo test -p huginn-runtime run_shell_seatbelt_denies_unix_socket_connect_after_host_control_succeeds -- --nocapture` failed deliberately in lib. The host control connected and the listener accepted. The sandbox uses `nc -v -U ... -w 1`; normal status 1, the listener did not accept, but stderr only contains `Error opening /private/var/select/sh: Operation not permitted` and not a `nc`/socket diagnostic. The test does not count this absence as an OS denial and remains failed/ambiguous. The listener observation is now printed before the diagnostic assertion.

### Validation

- The three focused tests were executed. TCP and UDP passed; Unix failed for lack of a diagnostic attributable to the operation, after confirming a positive host control and a sandbox listener without acceptance.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- The full package suite was not executed. Production, SBPL, profile, policy, permissions, phase 2 nor task 3.1 were touched. Task 3.2 remains unchecked; a conclusive Unix result and descendant scenarios are missing.

## TCP 3.2 ambiguity diagnostic (2026-11-09)

- A diagnostic-only test was added that runs `/usr/bin/nc -h` via the real `run_shell` and the host control, with markers before and after, and keeps the full outputs in the `--nocapture` log. Under the sandbox the result was shell status 0; stdout `before-nc` and `after-nc-status=1`; stderr included `Error opening /private/var/select/sh: Operation not permitted` and then the full help text of `nc`. The host control gave shell status 0 and the full help text, without the read error. The usage appears in stderr; `nc -h` returns status 1 inside the shell.
- Limited conclusion: `/usr/bin/nc` does start and process `-h` under the profile. This refutes the general form of hypothesis (1), but not that the connection operation has a specific initialization failure.
- The controlled TCP probe and the sandboxed one use the same form `/usr/bin/nc -v -w 1 127.0.0.1 <port>`. The host produced `Connection to 127.0.0.1 port <port> [tcp/*] succeeded!`, status 0, and the listener accepted. In the sandbox the shell reached `shell-started`, ended normally with status 1 and stderr only had `Error opening /private/var/select/sh: Operation not permitted`; the listener did not accept during the 3 s of observation. The assertion still fails because there is no `nc` diagnostic of its own that attributes the failure to a socket denial.
- Hypothesis outcome: (1) does not explain the general inability to start `nc`; a connection-specific init failure remains possible. (2) remains possible but there is no alternative `nc` message that proves it. (3) the host/sandbox invocation form is now identical and the host control validates the listener; no sandboxed acceptance was observed, but this does not determine whether there was another failure prior to the socket.
- No production/SBPL/policy/allow rules change. The TCP, UDP and Unix tests keep failing with an ambiguous outcome, and the network/socket capabilities remain unchecked.

### Validation of the diagnostic

- `cargo test -p huginn-runtime run_shell_diagnoses_nc_startup_under_seatbelt -- --nocapture`: passed, both in lib and in `approval_resource`; the full help output was observed in both contexts.
- `cargo test -p huginn-runtime run_shell_seatbelt_denies_tcp_connect_after_host_control_succeeds -- --nocapture`: failed intentionally at the `nc` diagnostic assertion; the host control accepted, the sandbox listener did not accept, the sandbox output without an `nc` diagnostic.
- `cargo test -p huginn-runtime run_shell_seatbelt_denies -- --nocapture`: 0/3 passed; TCP, UDP and Unix failed for lack of a denial diagnostic. None counts as an OS denial.
- `cargo fmt --all -- --check`: it initially found only the format of the diagnostic test; `cargo fmt --all` was applied. The final check still has to be re-run.
- `git diff --check`: passed.

## Limited continuation: 3.2 socket tests (2026-11-09)

### RED and controls

- Three macOS tests were added directly to the `run_shell` integration module. Each test creates a host listener with a bounded timeout, first verifies a positive control without sandbox using `/usr/bin/nc` and requires the listener to observe the TCP connection, the `udp-probe` UDP datagram or the local Unix connection. The fixture uses ephemeral ports and `tempfile::TempDir`; the binary was not copied.
- The same `/usr/bin/nc` probe is then sent through `run_shell`, which prepares the product profile and uses the real `/usr/bin/sandbox-exec` launcher. The sentinel `shell-started`, a normal non-zero shell status and an `nc` diagnostic of its own containing `Operation not permitted` are required. Without that, the test fails as an ambiguous result; the absence of a listener alone would not approve a denial.
- Observed RED: the positive controls passed for TCP, UDP and Unix/local. In every sandboxed run `shell-started` appeared, `Exit: exit status: 1` and the stderr `Error opening /private/var/select/sh: Operation not permitted`. Neither `nc:` nor a protocol diagnostic appeared. Therefore, the test does not distinguish whether `nc` managed to start; no TCP/UDP/Unix enforcement is claimed and it is not accepted as GREEN. The denied path suggests a dependency on a startup read, but the evidence does not identify which process attempted it nor demonstrates on its own that it was `nc`.
- The profile was not modified; no network/socket permission, `dyld-support.sb` nor an allowance for `/tmp` was added. `tasks.md` was not modified to mark 3.2 as complete. The test did not cover descendants, which remains pending in 3.2.

### Results per protocol

| Protocol  | Host control                                                    | Result of `run_shell`                                                                                                               | Status                                   |
|------------|-----------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------|
| TCP        | `/usr/bin/nc` connected and the listener accepted                         | Sentinel; normal exit 1; read error `/private/var/select/sh`; no `nc` message of its own; sandboxed acceptance could not be measured | Blocked/ambiguous; does not count as a denial |
| UDP        | `/usr/bin/nc` sent `udp-probe`; the listener received the exact bytes | Sentinel; normal exit 1; same read error; no `nc` message of its own; sandboxed reception could not be measured                     | Blocked/ambiguous; does not count as a denial |
| Unix/local | `/usr/bin/nc -U` connected and the listener accepted                      | Sentinel; normal exit 1; same read error; no `nc` message of its own; sandboxed acceptance could not be measured                    | Blocked/ambiguous; does not count as a denial |

### Commands and validation

- `cargo test -p huginn-runtime run_shell_seatbelt_denies -- --nocapture`: intentional RED, 3 tests failed for not observing an `nc` diagnostic of its own; the three host controls reached the listener. The failures are probe incompatibility/ambiguity under the profile, not accepted denials.
- `cargo test -p huginn-runtime`: 48 unit tests passed, the 3 new scenarios failed for the same ambiguous condition; no other failures recorded before finalizing the target.
- `cargo fmt --all -- --check`: pending at the time this record was updated.
- `git diff --check`: pending at the time this record was updated.

### Status and next step

Partial/blocked. Task 3.2 remains unchecked; TCP, UDP and Unix/local have a valid positive control, but none has conclusive evidence of a protocol denial under the profile. Do not advance to CI/docs/3.1 nor widen the permissions. Resolve in an authorized slice how to run a known probe under this profile without widening the permissions; if a file read of the probe prevents it, record the blocker without adding an allowance.

## Follow-up of the private helper 2.1–2.2 (2026-11-09)

- Before the refactor, characterization baseline: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` passed. On macOS it ran 8 tests of the module in lib and the same 8 in `approval_resource`; the direct test confirmed status 65, the `unbound variable` diagnostic with `invalid.sb` and the absence of the marker. The real `run_shell` flow still did not receive the invalid profile.
- `run_shell_with_launcher` was extracted to share the assembly and the process lifecycle. `run_shell_with_seatbelt_profile` is private and macOS-only: it prepares the temporary file and calls the same runner with the fixed `/usr/bin/sandbox-exec`. The product callsite keeps `prepare_workspace` + `build_profile`; there is no configuration, env, public arguments nor fallback.
- A test was added that delivers invalid SBPL through the private helper and requires `Err` and an absent marker. The first execution failed naturally: sandbox-exec returned the rejection status 65, but the runner formatted it as normal output. The conversion of status 65 to `AgentError::Tool` was added; on the repeat, the test passed. This is RED/GREEN evidence of the observable case, although the rest of the change is a behavior-preserving extraction. No artificial RED was added.
- Cleanup, the spawn error seam nor tasks 2.3–2.4 were touched. The 2.1–2.2 matrix remains incomplete: the absent/not executable launcher, the creation/write/close/cleanup errors nor all the rejection forms were tested; therefore both tasks remain unchecked. `SeatbeltProfileFile::drop` remains best-effort.

### Validation after the refactor

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: passed; 9 tests in lib and 9 in `approval_resource`, including the rejection through the helper.
- `cargo test -p huginn-runtime` (without a filter): passed. Counts: lib 40; adapter_security 3; approval_resource 24; coding_workflow 34; connections 4 passed/1 ignored; guardrails 8; host_secrets 1; inference_adapters 31; mcp_lifecycle 2; providers_integration 9; session_locks 6; sessions_persistence 5; skill_loading 10; skills_catalog 9; stream_persistence 4; tool_authoring 4; tool_lifecycle 12; transports 6; wasm_outline 0; wasm_output_validation 0; doc-tests 0. No failures.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

Status of this unit: partial. Tasks 2.1 and 2.2 remain unmarked; do not advance to 2.3–2.4 nor phase 3 in this slice.

## Review of status 65 (2026-11-09)

- Root cause verified in `run_shell_with_launcher`: any status 65 on macOS returns `AgentError::Tool` before formatting stdout/stderr. The status does not by itself identify the SBPL parser: the shell command can end with the same code.
- `run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile` was added, which generates a profile with `crate::seatbelt` and runs `exit 65` through the private helper. The RED was reproduced and saved: the test fails at `.expect(...)` with `Tool("sandbox-exec rejected the shell profile or command")`.
- macOS 27.3 evidence with `/usr/bin/sandbox-exec`: an invalid SBPL returns 65 with a parser stderr of `sandbox-exec: unbound variable ... at <invalid.sb>, line 1, column 1` and the token location; the valid SBPL with shell `exit 65` also returns 65, but the stderr contains execution messages (`Error opening .../var/select/sh` and a `getcwd` error in that execution), not the parser diagnostic. The helper test also confirms the valid case as an ambiguous status 65 for the runner.
- Discrimination using stderr patterns is not implemented: there is no evidence of a stable/documented Seatbelt contract for that format, and stderr is shared with the child, which can emit arbitrary text. The exit code does not discriminate either. The fail-closed behavior is not relaxed nor guessed.
- Decision: stop the fix and report the dilemma. The regression is added and is currently RED; the existing invalid rejection test through the helper keeps passing. The 2.1/2.2 task matrix is incomplete, without checks. A stable discriminator must be agreed/tested before continuing; the allow rules, the cleanup nor other tasks were modified.

### Validation

- `cargo test -p huginn-runtime run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile -- --nocapture`: fails as the expected RED due to the blanket conversion of status 65.
- `cargo test -p huginn-runtime run_shell_returns_error_when_seatbelt_rejects_profile_without_running_shell -- --nocapture`: passes (lib test and duplicate in `approval_resource`).
- `cargo fmt --all -- --check`: passes after formatting; `git diff --check`: passes.
- The full package suite was not executed because no product change was implemented and the regression test is deliberately RED.

## Resolution of status 65 (2026-11-09)

- Minimal change: `run_shell_with_launcher` no longer interprets a status 65 as `AgentError`. There is no reliable discriminator between the sandbox-exec parser status and the shell exit status. The process status and stdout/stderr remain in the standard observable result (`Exit: exit status: 65`, `stdout`, `stderr`); the spawn failures keep propagating as `Err`.
- The rejection test through the helper now requires exit status 65, the observed diagnostic `unbound variable`/`profile.sb` and the absence of the marker. This output is the observable result of the rejection, not an accepted shell execution: the launcher rejected the profile before the shell. The test with a valid profile and `exit 65` also requires the same status as a normal exit.
- RED/GREEN: the RED of `exit 65` under the previous code is documented above; the user also reported the baseline of the rejection test. After the change both tests pass on macOS 27.3, both in lib and in `approval_resource`.
- stderr is not inspected in production nor are SBPL, permissions, cleanup or fallback modified. The rejection remains fail-closed because `sandbox-exec` does not start the shell with an invalid profile; the API represents the status as an observable output, not as `Err`.
- `tasks.md` keeps 2.1 and 2.2 incomplete: the incomplete matrix of the absent/not executable launcher and the creation, write, close and cleanup failures, among other scenarios, persists. No other tasks were advanced.

### Validation of the resolution

- `cargo test -p huginn-runtime run_shell_reports_seatbelt_profile_rejection_without_running_shell -- --nocapture`: passed; 1 test in lib and its duplicate in `approval_resource`.
- `cargo test -p huginn-runtime run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile -- --nocapture`: passed; 1 test in lib and its duplicate in `approval_resource`.
- `cargo test -p huginn-runtime`: passed without a filter; 41 lib, 25 approval_resource, the rest of the integrations passed, 1 test ignored for an external requirement and the doc-tests passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

## Limited continuation: spawn, limit and cleanup (2026-11-09)

- A macOS test was added that invokes the private helper with a nonexistent absolute launcher and a marker command. Result: spawn `Err` and absent marker. No launcher configuration nor environment override was added. The test characterizes an already existing error propagation.
- An upper limit test was added to `run_shell_with_seatbelt_profile`: a profile of 64 KiB + 1 byte is rejected during preparation and the shell marker stays absent. The limit already existed; the test characterizes the rejection before the spawn.
- `SeatbeltProfileFile` now has `cleanup(self) -> io::Result<()>`: it removes the profile and then the directory, propagating the errors. `run_shell_with_seatbelt_profile` calls cleanup after every controlled result (success, rejection/error status, timeout or cancellation). If cleanup fails after a possible execution, the message explicitly states that the command may have run; it does not claim that there was no execution nor starts a fallback. `Drop` keeps doing best-effort as a backstop for abrupt drop/cancellation.
- Tests of the `cleanup` method: normal removal checks that the profile/directory no longer exist; an extra entry in the directory makes the directory removal fail and the error propagates. The first run with those tests failed to compile because `cleanup` did not exist. This demonstrates the absence of the API before GREEN, but it is not a behavioral assertion RED; no valid TDD RED cycle is claimed for this product change.
- A cleanup error could not be forced deterministically inside the flow that runs the launcher. The test verifies the propagator and the wiring was reviewed in `run_shell_with_seatbelt_profile`; a failed unlink during execution, a creation/write/close failure nor a non-executable launcher were not simulated. The non-empty directory error validates a `remove_dir` failure, not an OS denial.
- SBPL, permissions, `status 65`, phase 3 tests nor tasks 2.3–2.4 were changed. 2.1 and 2.2 remain unchecked because their full matrix is not covered.

### Validation of this continuation

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: passed, 14 tests in lib and 14 duplicates in `approval_resource` (including spawn, limit and cleanup).
- `cargo test -p huginn-runtime` without a filter: passed; 45 unit tests, 4 connections passed/1 ignored, and all the integration suites passed; the doc-tests passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed after `cargo fmt --all`.
- Final re-run of `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: passed 14/14 in lib and 14/14 in `approval_resource`.

## Limited continuation: testable profile and launcher failures (2026-11-09)

- A macOS test was added with an existing launcher created in a temporary directory and mode `0600` (without the executable bit). The private launcher helper returns a spawn `Err` and the marker remains absent. The fixture stays under `tempfile::TempDir`, which cleans up on success and when the test unwinds.
- `create_seatbelt_profile_under(profile, temp_root)` was extracted, private and used by production through `create_seatbelt_profile(profile)`, which always passes it `std::env::temp_dir()`. The test supplies a regular file as the root and gets `io::ErrorKind::NotADirectory` when creating the profile directory. There is no environment variable nor public/configurable seam.
- `write_seatbelt_profile(&mut impl Write, bytes)` was extracted, used by the product path. `FailingWriter` returns a deterministic error and the test verifies that `write_all` propagates it. The product flow only calls the launcher after the profile creation/writing returns `Ok`; the independent spawn test also checks that a launch `Err` does not run the command. No writer is injected nor a test option added to production.
- `0700` for the directory, `0600` for the file, `create_new(true)` and the 64 KiB limit are preserved.
- `sync_all` was not added: the required guarantee is that the launcher reads the file after `write_all` and a normal close, in the same flow; syncing to durable storage is not necessary for that read and adds I/O/cost. It also does not produce a `File::drop` error check.
- A `close` failure cannot be forced deterministically with the safe API of `std::fs::File`: `File` offers no `close()` that returns an `io::Result`; `drop` does not report errors. `sync_all` would report sync failures, it does not turn the close into a testable operation nor demonstrate a close failure. `unsafe` was not used nor that guarantee claimed.
- The cleanup tests were kept outside the real execution flow: success and non-empty directory test the explicit method, not the arbitrary denial of `remove_file`/`remove_dir` in a launcher exit. `Drop` remains best-effort for abrupt exits; tasks 2.1 and 2.2 remain unmarked because close and arbitrary cleanup/full flow are not covered.

### Validation

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: passed; 17 tests in lib and 17 duplicates in `approval_resource`.
- `cargo test -p huginn-runtime`: passed without a filter; 48 unit tests, 4 connections passed/1 ignored, all the remaining integration suites and the doc-tests, without failures.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- 2.3–2.4 nor phase 3 were advanced; SBPL, grants, policy or fallback were not changed.

## Reviewed apply close: tasks 2.1–2.2 (2026-11-09)

### TDD RED → GREEN → REFACTOR

- Private behavioral tests were added for the pre-spawn error composition (primary failure plus cleanup) and the post-spawn error (primary timeout plus cleanup), plus the case of a successful execution result with a failed cleanup. They check that the pre-spawn message says the launcher did not start and that the post-spawn errors indicate that the shell may have run.
- Observed RED: `cargo test -p huginn-runtime pre_spawn_error_composition_preserves_primary_and_cleanup_failures -- --nocapture` failed with only the primary failure in the message, without the secondary cleanup error. The failure was a behavioral assertion, not a compilation error.
- GREEN: the composition was centralized in private helpers shared by production and tests. A failed open and write explicitly clean up the already created resources and include any cleanup error alongside the primary one. A directory creation error indicates a failure before the spawn; there are no created resources to clean up. `run_shell_with_seatbelt_profile` composes every controlled launcher output with the result of `SeatbeltProfileFile::cleanup`, preserving spawn errors, timeout/cancellation and the execution result/error.
- The tests of not-a-directory mkdir, failing writer, exceeded size, rejection and absent/not executable launcher keep the expected markers/errors. The cleanup test with a non-empty directory tests `SeatbeltProfileFile::cleanup` directly; it demonstrates `remove_dir` error propagation with leftover content, not an OS denial inside the full launcher flow. The close errors of `std::fs::File` were not simulated.
- `Drop` still does a best-effort unlink and does not promise removal on an OS failure. No `unsafe`, public/environment overrides, new dependencies, shell/profile override, additional permissions, fallback, retry nor status 65 changes were introduced. The fixed profile, the limit, `create_new`, the `0700`/`0600` modes and `/usr/bin/sandbox-exec -f PROFILE /bin/sh -c COMMAND` are preserved.

### Validations

- RED: `cargo test -p huginn-runtime pre_spawn_error_composition_preserves_primary_and_cleanup_failures -- --nocapture` — failed as expected (0 passed, 1 failed; skips 59 filtered tests).
- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` — passed on macOS: 29/29 in `lib` and 29/29 in `approval_resource`; the first attempt before the minor fixes failed 2 text asserts (case/assertion `may have run`), it does not count as the final result.
- `cargo test -p huginn-runtime` without a filter — passed: 60 `lib` tests, 3 adapter security, 44 approval resource, 34 coding workflow, 4 connections passed/1 ignored, 8 guardrails, 1 host secrets, 31 inference adapters, 2 MCP lifecycle, 9 providers integration, 6 session locks, 5 session persistence, 10 skill loading, 9 skills catalog, 4 stream persistence, 4 tool authoring, 12 tool lifecycle, 6 transports and doc-tests; zero failures.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings` — passed.
- `git diff --check` — passed.

### Status and limits

- Tasks 2.1–2.2 remain unmarked: even though the asserts pass, the behavior change was implemented before adding the new tests. The RED was observed by temporarily reintroducing the loss of the secondary error, but this does not fix the non-compliance with the requirement to write the test first.
- The recorded completed SDD phases are still `init, explore, propose, spec, design, tasks`; current phase `apply`, `next: apply`, `apply_status: partial`. Acceptance is not declared nor `sdd-verify` recommended until the TDD order is resolved.
- The local, ignored RPI task reflects the partial status. Verify, QA nor archive were not executed.

## Launcher characterization: tasks 2.3–2.4 (2026-11-09)

- I reviewed the lifecycle spec and the flow described in the design, as well as `run_shell`, `run_shell_with_seatbelt_profile`, `run_shell_with_launcher` and the macOS tests in `crates/huginn/runtime/src/tools.rs`.
- Baseline before editing the tests: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` passed; 17 tests in lib and 17 duplicates in `approval_resource`. The cwd, PATH, stdin EOF, separated streams, two 100 KB streams with limit and draining, and the spawn errors for an absent/not executable launcher without a marker were already characterized.
- The execution parameters test was extended to check `$0=/bin/sh` and that `HOME` is not inherited when it exists in the host environment. After the change, the focused suite passed with the same counts. This test characterizes the previous behavior; no product RED/GREEN is claimed.
- Evidence mapping: the canonical cwd, the selected PATH, null stdin/EOF, the separated stdout and stderr, the limit/draining and the spawn errors are covered by macOS tests. The implementation forms the launcher argv in separate elements and sets `/bin/sh -c` with the original command as a single argument; `env_clear()` restores only `PATH`. The `$0` test confirms the observed shell, it does not by itself prove the absence of every possible intermediate layer; the argv/no extra shell claim is limited to direct inspection of the callsite and the helper.
- Tasks 2.3–2.4 were marked complete only within that evidence. 2.1–2.2 remain unchecked and partial: the safe close of `File` is not observable and the arbitrary cleanup is not forced from the launcher flow. No product behavior nor later phases were touched.

### Validation of tasks 2.3–2.4

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: baseline and post-change passed; 17 tests in lib and 17 in `approval_resource`.
- `cargo test -p huginn-runtime`: passed without a filter; 48 unit tests, 4 connections passed/1 ignored, all the remaining integration suites and the doc-tests, without failures.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Scoped close of task 3.2 (2026-11-09)

- I reviewed the three socket tests in `crates/huginn/runtime/src/tools.rs`. I removed the auxiliary `nc -h` diagnostic test that dumps the whole help, along with the high-volume `eprintln!` of the TCP/UDP/Unix cases. The verifications now report the output and the listener result in the assertion message on failure. UDP requires that the sandboxed listener receives no datagram.
- I adjusted the shared listener result wait to 5 s, above the bounded timeouts of the listeners; the threads have finite deadlines and are joined in the normal flow. The Unix socket is under `TempDir` inside the canonical workspace; the path is removed between the host and sandbox controls and the temporary directory covers the cleanup on failure.
- Fresh evidence reported by the orchestrator on macOS 27.3: the TCP host connects and is accepted; the sandboxed descendant `nc` probe ends with status 1 with `connectx ... (tcp) failed: Operation not permitted` and the listener without acceptance. The UDP host receives four `[X]` datagrams and the exact payload `[udp-probe]`; the sandbox `nc` ends with status 1 with `connectx ... (udp) failed: Operation not permitted` and no datagram arrives. Unix: the host accepts; the sandbox shell sees the socket (`socket-visible`), `nc -v -U ... -w1` exits with normal status 1 and the listener does not accept. Apple's `nc` shows no Unix connect diagnostic; only the non-fatal error `/private/var/select/sh`, which also appears in `nc -h` with the full help and the later marker. No diagnostic is invented: the Unix evidence is the combination of the exact host control, the socket visibility, the normal non-zero return and the non-acceptance.
- The three probes run as child processes of the confined shell. This limits the claim to those probes and their socket operations; it does not validate other descendants, external file access nor more protocols/versions. Only 3.2 was marked `[x]`; 3.1, 3.3–3.6 and Phase 2 2.1–2.2 remain pending/incomplete. Production, SBPL, CI, docs nor grants were modified.

### Commands Run

- `cargo test -p huginn-runtime run_shell_seatbelt_denies -- --nocapture`: exit 0; 3/3 in lib and 3/3 in `approval_resource`.
- `cargo test -p huginn-runtime`: exit 0, without a filter. Results: lib 51; `adapter_security` 3; `approval_resource` 35; `coding_workflow` 34; `connections` 4 passed/1 ignored; `guardrails` 8; `host_secrets` 1; `inference_adapters` 31; `mcp_lifecycle` 2; `providers_integration` 9; `session_locks` 6; `sessions_persistence` 5; `skill_loading` 10; `skills_catalog` 9; `stream_persistence` 4; `tool_authoring` 4; `tool_lifecycle` 12; `transports` 6; the two wasm targets and the doc-tests without tests. No failures.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.

## Scoped close of task 3.1 (2026-11-09)

- Scope reviewed before editing: `crates/huginn/AGENTS.md`, spec delta, design, tasks and status. The tree had previous broad changes; they were preserved and SBPL, allow rules, product code, CI, docs nor grants were altered.
- TDD: the task verified already implemented enforcement, so the added scenarios are characterization, no product change was invented. Previous baseline: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` passed 20 tests in lib and 20 in `approval_resource`; the external test and the positive workspace read/write already existed.
- Six real tests of `run_shell`/`/usr/bin/sandbox-exec` were added: another workspace; an isolated home/credentials fixture without accessing real secrets; an external path; a designated temporary directory only as a fixture and outside the workspace (without touching the global `/tmp`); an attempt to `cwd` to the outside; a symlink from the workspace to the external file. For the five external read cases, the host verified that the fixture was readable before the sandbox; the shell emitted the sentinel, `/bin/cat` produced `Operation not permitted` for the existing file, the shell returned a normal non-zero status and the secret did not appear. In the symlink case it was verified that `canonicalize(link)` was the external target. The `cwd` test requires an OS denial when changing directory or, if the change is allowed, a denial of the relative read and the absence of the secret. The existing positive test verifies the read/write inside the canonical workspace.
- No profile failure was observed in these scenarios; there is no product RED/GREEN. In the first combined run of the whole focused suite, the pre-existing UDP host control failed once for not receiving the datagram. The isolated repeat passed; the final focused adversarial run passed 7/7 in lib and 7/7 in `approval_resource`; the positive workspace passed 1/1 in both; the subsequent full suite passed.
- Observed platform: macOS 27.3, build 26C5035r, with `/usr/bin/sandbox-exec`. This evidence does not establish support for other versions nor test other paths/operations, general robustness against TOCTOU nor future Seatbelt compatibility.

### Commands Run

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` (baseline): exit 0; 20 lib + 20 `approval_resource` passed.
- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` (post-tests, first run): exit 101; the six denies and the path cases passed, but the existing UDP host control did not deliver its datagram. This result was recorded, not hidden.
- `cargo test -p huginn-runtime run_shell_seatbelt_denies_udp_datagram_after_host_control_succeeds -- --nocapture`: exit 0; 1 lib + 1 `approval_resource` passed on an isolated repeat.
- `cargo test -p huginn-runtime run_shell_cannot_read -- --nocapture`: exit 0; 7 denies in lib + 7 in `approval_resource` passed.
- `cargo test -p huginn-runtime run_shell_can_read_and_write_inside_workspace -- --nocapture`: exit 0; 1 positive in lib + 1 in `approval_resource` passed.
- `cargo test -p huginn-runtime` (without a filter): exit 0; lib 57; adapter_security 3; approval_resource 41; coding_workflow 34; connections 4 passed/1 ignored; guardrails 8; host_secrets 1; inference_adapters 31; mcp_lifecycle 2; providers_integration 9; session_locks 6; sessions_persistence 5; skill_loading 10; skills_catalog 9; stream_persistence 4; tool_authoring 4; tool_lifecycle 12; transports 6; wasm targets/doc-tests 0. No failures.
- `cargo fmt --all -- --check`: exit 0 after running `cargo fmt --all` to normalize the new format.
- `git diff --check`: exit 0.

Status: 3.1 is marked `[x]` only for these fixtures/cases/observed macOS. It does not imply completing 3.3–3.6 nor the change; the other incomplete tasks keep a partial status.

### Final review of the `cwd` case

- The review detected that the initial version hid the `cd` error and therefore did not demonstrate that the shell had changed directory. The test was adjusted to observe the result: it requires an `Operation not permitted` diagnostic if the change is denied; if the change is allowed, it requires that the relative read of the external file receives the denial and does not reveal the secret.
- `cargo test -p huginn-runtime run_shell_cannot_read_external_marker_after_changing_cwd -- --nocapture`: passed in lib and `approval_resource` (1 case per target). `cargo fmt --all -- --check` and `git diff --check`: passed. The full `cargo test -p huginn-runtime` passed before this last assertion precision; the complete adversarial suite also passed before that precision.

## Final apply continuation: CI, documentation and remaining limits (2026-11-09)

### Completed in this continuation

- 1.14 and 3.3–3.6, and 4.1–4.4 were marked, only with the scope described in `tasks.md`. The profile tests show deny-default, `dyld-support.sb` before deny-default, explicit fork/exec, runtime reading bounded to `/bin` and `/usr/lib`, workspace read/write, the absence of network/socket allows and the absence of broad roots. The real execution and enforcement tests remain macOS-only.
- The support inventory in the manifests/CI does not declare a matrix of macOS versions. Runtime separates dependencies by macOS/Linux target; the existing CI uses `macos-latest` in multi-OS jobs only for push and excludes Huginn on Windows. The new job `Test Huginn Seatbelt (macOS)` also runs on PR; `macos-latest` is not interpreted as support for additional fixed versions.
- The dedicated job was added to `.github/workflows/ci.yml`, with an explicit `RUNNER_OS` check and the executable existence of `/usr/bin/sandbox-exec`, and the execution of `cargo test -p huginn-runtime` without a filter. The job preserves the output and requires that the workspace read/write, external path and TCP/UDP/Unix tests appear as `ok`; an absent or ignored test does not satisfy those checks. It was added to the `CI / Required` job and to the results evaluator. The workflow test verifies the runner, the launcher, the unfiltered invocation, the mandatory cases and the required gate; `node --test .github/scripts/ci-*.test.cjs` passed 309/309.
- The README, architecture, example config and ADR-0018 were updated. The obsolete "not sandboxed" text was removed for the shell, without widening the claim to native tools/MCP nor to Linux/Windows. No configuration or opt-out was added.
- For 4.1 the preexisting characterization tests were used: `rejected_effect_is_recorded_denied_and_not_executed` and `allow_once_authorizes_only_one_prepared_operation_and_keeps_profile_unchanged`. No TDD RED/GREEN is claimed in this task nor that those application tests constitute Seatbelt kernel evidence. The real integration of `run_shell` separately confirms the mandatory sandbox.
- Clippy initially detected warnings on crate-public items inside the private module, a redundant borrow and a duplicated assertion between branches. The visibility was adjusted to `pub` inside the private module, the borrow was removed and the assertion consolidated; no `allow` was added nor the profile altered.

### Blocker of 2.1–2.2

- The available coverage includes the invalid profile observed by `run_shell` as status 65 without a marker, the absent/not executable launcher, the oversized profile, the creation failure via a non-directory root, the writer error propagation and the explicit cleanup in the normal/error case. The `status 65` is kept as an observable result because it can come both from the launcher and from a valid shell; its cause is not inferred from the code alone.
- The safe API of `std::fs::File` does not offer `close() -> io::Result`; `Drop` does not report close errors. No `unsafe`, no configurable override nor a public seam was added, and no observed close error is claimed. Nor was an arbitrary unlink failure forced in the full launcher flow: `Drop` remains best-effort on abrupt termination, although explicit cleanup exists for controlled results. Due to these pending criteria, 2.1 and 2.2 remain unmarked. A decision/documentation of the verifiable close requirement and of the guarantee level demanded for the cleanup on an OS error is needed, or a different approved API/platform.
  
### Validation of this continuation

- macOS 27.3 / build 26C5035r, `/usr/bin/sandbox-exec` present: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` exit 0, 26/26 in lib and 26/26 duplicates in `approval_resource`; it covers the existing macOS lifecycle, path, socket, profile rejection and spawn scenarios.
- `cargo test -p huginn-runtime` without a filter exit 0, executed on macOS: lib 57/57; `adapter_security` 3; `approval_resource` 41; `coding_workflow` 34; `connections` 4 passed/1 ignored for an external requirement; `guardrails` 8; `host_secrets` 1; `inference_adapters` 31; `mcp_lifecycle` 2; `providers_integration` 9; `session_locks` 6; `sessions_persistence` 5; `skill_loading` 10; `skills_catalog` 9; `stream_persistence` 4; `tool_authoring` 4; `tool_lifecycle` 12; `transports` 6; wasm/doc tests 0. No failures. This run was after the last Rust code adjustment.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings`: exit 0 after fixing the three described findings.
- `just huginn-architecture`: exit 0; the architecture boundaries passed, including the negative control.
- `cargo fmt --all -- --check`: exit 0. `git diff --check`: exit 0.
- `node --test .github/scripts/ci-*.test.cjs`: exit 0, 309/309. `pnpm exec markdownlint-cli2` for the README, the architecture, the ADR index and ADR-0018: exit 0, 0 issues.
- Focused approval tests: `cargo test -p huginn-application allow_once_authorizes_only_one_prepared_operation_and_keeps_profile_unchanged` and `cargo test -p huginn-application rejected_effect_is_recorded_denied_and_not_executed` passed separately (1 test each).
- `just ci-local` was not run: it was not required to validate the focused apply criteria and it adds gates unrelated to the change. Remote CI was not run; the PR-blocking guarantee is implemented/tested locally, not observed in a remote run. `sdd-verify` was not executed.

Apply status: partial and blocked only on 2.1–2.2 due to the safe close and cleanup evidence not being available/forced in the full flow. Acceptance is not claimed nor verify started.

## Reopening of 2.1–2.2 under the approved contract (2026-11-09)

- The user approved requiring explicit cleanup with error propagation on the controlled returns; on pre-spawn failures, not starting the shell and reporting the primary and cleanup errors; and leaving the abrupt `Drop` as best-effort, without a guarantee on an operating system failure. An independent `close()` error is not required because the safe API of `std::fs::File` does not expose it.
- To complete TDD, the previous composition behavior was temporarily reconstructed after the first implemented change. Behavioral RED: `pre_spawn_error_composition_preserves_primary_and_cleanup_failures` failed when it lost the secondary cleanup error; `post_spawn_error_composition_preserves_primary_and_possible_execution` failed when it did not preserve the timeout and the fact that the shell could have run. The implementation was restored and both tests passed. The additional test `post_spawn_success_with_cleanup_error_does_not_claim_shell_was_skipped` also passed. No `unsafe`, public seam, override nor dependency was added.
- The change preserves the explicit cleanup after the controlled launcher result and propagates the cleanup errors; the creation/open/write failures before the spawn attempt to clean up and compose the primary and cleanup errors. The post-spawn composition preserves the execution error and does not claim that the shell did not run. `Drop` continues best-effort on abrupt discard.
- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: passed, 29 tests in lib; the same module is also compiled/executed from `approval_resource`.
- `cargo test -p huginn-runtime`: exit 0; 60 lib tests, 44 of `approval_resource`, the integration suites passed and 1 test was ignored for an external requirement.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings`, `cargo fmt --all -- --check` and `git diff --check`: exit 0.
- The checks are local; remote CI was not run. Real enforcement remains limited to macOS 27.3/build 26C5035r. Removal on an OS failure during the abrupt `Drop` is neither tested nor promised, nor the unobservable `close()` error.

Status: 2.1 and 2.2 are marked complete according to the reviewed and approved criteria. Apply is complete; repeat `sdd-verify` before any QA/archive.
