# Apply evidence: acceptance and gates

## V-01 fix: SET-05 (2026-10-20)

This block replaces the historical final result below for the current
candidate. Verify found a real contractual failure that those suites did not cover.
`verify-report.md` is preserved with the FAIL and an independent reverify is required;
this fix does not certify QA nor user acceptance.

**Apply V-01 closure:** RED→GREEN regression, final complete focused suite PASS
and final `just ci-local` exit 0, 9/9, 477 s. Independent `sdd-verify` pending.

New logs in `F/`:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-fix-v01/`.
Commands from `/Users/acosta/Dev/dallay/cortex`:

- `red.log` / `green.log`:
  `cargo test -p huginn-presentation --test state settings_horizontal_arrows_cycle_toasts_without_leaving_the_row -- --exact`.
  RED exit 101, behavioral assertion `SET-05: Right`, observed level 0 against 1;
  the compilation is correct. GREEN exit 0, 1 PASS without lowering the expectations. The test
  navigates `/settings` and the real `overlay_input`, without directly mutating the copy.
- `settings.log` / `settings-refactor.log`:
  `cargo test -p huginn-presentation --test state settings_ -- --test-threads=1`.
  Exit 0, 10 PASS in both runs. It includes three new tests and the existing
  ones of late receipts, serialization, errors/retry, Approval, shutdown and
  stale input. The second log corresponds to releasing the guards explicitly.
- `focused-tests.log`:
  `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features -- --test-threads=1`.
  Exit 0, 391 PASS reported in 57 blocks (they include subprocesses), 2 ignored,
  0 failed; presentation 158 PASS, state 35 and PTY 39. Executed before the
  guard lifetime adjustment in the tests; the final CI tests the final code again.
- `ci-local.log`: `just ci-local`, exit 1 in 3/9, Clippy:
  `significant_drop_tightening` in three guards of the new tests. It was not a
  product failure; this failed gate is preserved and not hidden with an allow nor with runner
  changes. `drop(state)` is added at the end of each group of assertions.
- `clippy.log`:
  `cargo clippy -p huginn-presentation --all-features --all-targets -- -D warnings`,
  exit 0 after the adjustment; without lowering assertions nor lint.
- `focused-final.log`: additional repetition of the seven package command after
  the guard adjustment, simultaneous with CI; exit 101 in PTY, 38 PASS/1 FAIL.
  `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` fails in
  `approval_code`, `approval challenge contains eight characters`. The helper
  waits for the "Type " prefix before extracting eight characters from the transcript;
  the output of that failure does not certify the complete challenge nor an unsafe
  product decision. No definitive cause is attributed to concurrency/environment
  and the Approval test is not modified outside the V-01 scope.
- `approval-recheck.log`:
  `cargo test -p huginn-presentation --all-features --test pty stale_and_paste_confirmation_cannot_approve_then_fresh_code_can -- --exact`,
  exit 0, 1 PASS, without code nor assertion changes. The additional failure is
  preserved as a PTY observation risk, it is not presented as a green run.
- `focused-recheck.log`: full repetition of the same seven package command
  over the final code, without changes after the PTY failure; exit 0. It includes state 35/35,
  PTY 39/39 and all the remaining targets; the failed case is not filtered nor ignored.
- `audit.log`: `cargo audit --no-fetch`, exit 0 without a mask, 1314 advisories and
  538 dependencies; no findings in the output.
- `static.log`: `cargo fmt --all -- --check`, `git diff --check`, lint of both
  artifacts and integrity hashes. The first later repetition of the lint failed
  MD038 for representing the final space of the challenge prefix as a code
  span; quotes are used without changing the literal. It changes neither code nor tests.
- `ci-local-final.log` / `ci-local-final.exit` / `ci-summary.txt`: second
  `just ci-local`, exit 0, 9/9, 477 s over the final candidate. Rust workspace
  all-features PASS, including state 35, PTY 39 and the challenge case that failed
  in the additional run; Vitest 24 files/210 tests and E2E 90/90 in
  Chromium/Firefox/WebKit (2.7 min). `dashboard-dev.log` preserves the server output;
  the SIGTERM corresponds to the cleanup after the E2E success.
  The Docker/browsers/image requirements were already certified by the mission; no data
  reset is executed nor replaced by `just ci-ci`. The parallelism,
  runner, assertions nor timeouts are changed. `cargo audit --no-fetch` direct also passes
  without the `|| true` mask of the audit gate. No code changes after this gate;
  only the Apply documentation is completed.
- `settings-cycle-build.log` / `settings-cycle.log`: repetition of the same external
  Verify source, without modifying it. Compilation exit 0 and execution exit 0,
  1 PASS: `active_tab=3, level=1`. It is an additional Apply check, not a new
  independent verdict. Exact commands:

```sh
rustc --edition=2021 --test /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/settings-cycle.rs -L dependency=target/debug/deps -o /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-fix-v01/settings-cycle --extern huginn_presentation=target/debug/deps/libhuginn_presentation-f028ce01cd85534f.rlib --extern crossterm=target/debug/deps/libcrossterm-19462bd5fb984d15.rlib --extern huginn_core=target/debug/deps/libhuginn_core-b0fd28370cd72cd2.rlib
/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-fix-v01/settings-cycle --nocapture
```

### Reviewed contract and impacts

- **SET-05, own GREEN, reverify pending**: both arrows are consumed in the
  toast pickers, level info→warn→error→info and duration 3→5→10→off→3, exact
  labels and stable tab/selection. Left also advances, an option explicitly
  allowed by REQ-05. The fix is limited to the Settings match; no toast layer.
- **SET-03/04/06, characterization PASS**:
  `settings_section_keys_preserve_noncycle_values_and_escape_toasts` preserves the
  section arrows in theme/indicators/sound/integrations, the wrap and Tab/BackTab
  from the toasts; it does not edit booleans nor themes with the horizontal arrows.
- **SET-07/08/13, PASS**:
  `settings_cycle_arrows_only_persist_displayed_values_on_enter` applies warn/5s
  with Enter once and recovers them when reopening; before that it does not write, Esc discards and
  does not cancel the active turn nor contaminate conversation/editor. The existing suite
  preserves receipts/Approval, errors, pending Apply and serialization.
- **OVL-03/EVT-01, focused PASS inherited from the current code**: the full suite
  reruns Approval, stale input, PTY streaming and providers_ui; the security
  tests nor the gates are modified. It does not equal a real provider nor a real Keychain.

The delivery keeps the approved `size-exception` over `main`; only seven new
product lines and three host tests, plus these two artifacts. No lowered spec,
no edit of verify-report/state, no QA/Archive, no publication operations.
The historical evidence and limits below are preserved, not deleted.

## History before Verify V-01

Date: 2026-10-20. Authorized scope: 6.2–6.3, `size-exception` delivery.
This document is neither `verify-report.md` nor `qa-report.md`. It does not certify user
acceptance. The complete tasks are preserved and `state.yaml` is not modified.

**Final result:** 6.2–6.3 complete, Apply 21/21. Final `just ci-local` PASS,
9/9 stages, exit 0, 431 s. No gate failure remains without being superseded by later
evidence; the historical failures and infrastructure risks are preserved below.

## Logs and commands

All own logs are in the approved directory:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-apply6/`.
The following names are relative to that directory.

- `prerequisites.log`: `just --list`, `just doctor`, `docker info`, image
  inspection and `pnpm --filter @dallay/rook-dashboard exec playwright install --list`.
  Docker operational; Chromium/Firefox/WebKit installed and dependencies available.
- `e2e-build.log`: `just test-e2e-build`, exit 0. Image rebuilt; no reset.
- `scrollback-red.log` / `scrollback-green.log`:
  `cargo test -p huginn-presentation --features test-driver --test pty support::screen_line_feed_at_bottom_scrolls_instead_of_overwriting -- --exact`.
  RED exit 101, assertion `FIRST != SECOND`; GREEN exit 0, 1 PASS.
- `stream-scrollback-red.log` / `stream-scrollback-green.log`:
  `cargo test -p huginn-presentation --features test-driver --test pty overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback -- --exact`.
  RED exit 101, `missing "┌Help" on screen`; GREEN exit 0, 1 PASS.
- `repeated-ui.log`:
  `cargo test -p huginn-presentation --test providers_ui repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation -- --exact`.
  Exit 0, 1 PASS. Fake clock, advance of 2001 ms; without a real time wait.
- `presentation-final.log`:
  `cargo test -p huginn-presentation --features test-driver -- --test-threads=1`.
  Exit 0, 155 PASS: lib 38, completion 9, extensions 2, files 2, parser 10,
  plugins 2, providers_ui 8, PTY 39, state 32, workflows 13. Fixture and doctests
  with zero cases are not counted as PTY nor as additional evidence.
- `app-acceptance.log`:
  `cargo test -p huginn --test tui --test providers_flow -- --test-threads=1`.
  Exit 0: tui 8 (4 app scenarios and 4 of the observer), providers_flow 6.
- `static-final.log`: `cargo fmt --all -- --check`, `git diff --check`,
  `cargo check -p huginn-application --examples` and
  `python3 .github/scripts/check-huginn-architecture.py`; output without failures,
  architecture with negative control PASS. The final review repeats fmt/diff.
- `huginn-all.log`: first consolidation of the seven packages, exit 0;
  previous to the tests and fix of this continuation. It is not the final gate.
- `huginn-all-final.log`: consolidation of the seven packages, exit 0, with
  `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features -- --test-threads=1`.
- `ci-local.log`: first `just ci-local`, exit 0, nine stages, 479 s;
  Vitest 24 files/210 tests and E2E 90 tests in three browsers. Previous to the fix;
  it is not used to assert the final gate of the modified candidate.
- `ci-local-final.log`: second `just ci-local`, exit 1 in 5/9. The test
  `actual_terminal_resize_preserves_bar_and_restores_overlay_surface` checked
  the new selection when the title appeared, before the asynchronous consumption of the watch.
  Failure screen: new form and the bar still at `fixture/alpha`.
  `resize-watch-green.log`: exact focused repetition exit 0, 1 PASS after awaiting
  `Idle|fixture/Bar-Provider` with the same timeout, keeping the exact assertion.
  It is not attributed to a product publication failure; the test barrier is corrected.
- `ci-local-verified.log`: third `just ci-local`, exit 0 with that synchronization;
  nine stages PASS, 431 s. Rust workspace/all-features tests PASS, including
  39 PTY without filter nor added serialization, providers_ui 8, state 32 and tui 8.
  Vitest 24 files/210 tests and E2E 90/90 in Chromium/Firefox/WebKit (3.2 min).
  `ci-summary.txt` preserves the summary read; `dashboard-dev.log` copies the output
  of the E2E development server to the approved temp. The SIGTERM closing of that
  server corresponds to the runner cleanup after passing E2E, not to a failure.
  `just ci-ci` was not executed: it omits E2E and does not replace this gate.
- `audit-unmasked.log`: `cargo audit --no-fetch`, exit 0 without `|| true`; 1314
  advisories loaded and 538 dependencies checked, with no findings in the output.
- `providers-headless.log`: `cargo run -p huginn-application --example providers_headless`,
  exit 0. It runs the offline example: pinned turn, next selection Unmanaged,
  safe snapshot and revocation without fallback nor remote health promise.
- `huginn-clippy-allfeatures.log`:
  `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features --all-targets -- -D warnings`,
  exit 0. It includes the fixture and the PTY `test-driver`, not only the default features
  used by the Clippy step of the CI runner.
- `artifact-lint.log` / `artifact-lint-green.log`: additional lint of the three
  edited artifacts; first exit 1 for a terminal space in a historical code span,
  corrected by representing the literal with quotes without altering the data; repetition
  `pnpm exec markdownlint-cli2 .agents/sdd/changes/huginn-overlay-system/{apply-validation,apply-progress,tasks}.md`
  exit 0, three files without incidents.
- `doctor.log`: `just doctor`, exit 1: the recipe does not exist in the current justfile.
  Divergence with the manual, not missing tooling: the tools and dependencies are
  checked through real commands and executed gates. The recipe was not invented.
- `pty-concurrent-green.log`: additional PTY run with the default parallelism,
  38 PASS/1 FAIL, exit 101 when creating a PTY (`failed to openpty`, OS -6).
  It coincided with the full CI and did not reach the execution of the failed scenario;
  it is not GREEN nor evidence of a secret leak. The assertions are not lowered.
  `pty-resource-diagnostic.log` records the PTY limit 511 and a process snapshot;
  it does not allow attributing by itself a definitive cause to the transient error.

## REQ → tests and own evidence

The state `COMPLIANT` means technical coverage observed in Apply, not a Verify/QA
verdict. The names refer to the targets of the previous logs; the
final consolidation will include in addition the core/application/runtime/plugin targets.

- **EVT-01 / OVL-01, COMPLIANT**: the new PTY
  `overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback` opens and
  closes Help, Settings, Provider, Login, ModelPicker and Confirm with a stream
  retained until explicit cancellation. It compares the full scrollback before and
  after, with a positive control of 45 conversation lines. It checks the mask,
  the screen and the transcript without the sentinel. The integration
  `repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation`
  goes through the real commands, including Logout, twice with a fake clock at
  0/2001 ms; it preserves the conversation content/count, the tail and the turn.
  `explicit_ui_categories_do_not_depend_on_message_text` covers the categories.
- **EVT-02, COMPLIANT**: state
  `session_reprojection_keeps_all_messages_beyond_ui_buffer_capacity`,
  `resume_and_synchronize_commit_messages_once_not_mutable_tail` and
  `tool_error_uses_contributed_label_and_marks_status`; runtime
  `sessions_persistence`/`stream_persistence` keep the persistence and the recovery.
- **EVT-03/04, COMPLIANT**: lib `event::tests`, state
  `error_event_producer_uses_injected_clock_and_deduplicates_by_operation`,
  `explicit_event_operation_scope_survives_interleaved_producers` and
  `oversized_errors_are_visible_without_retaining_or_truncating_them`.
  The 1999/2000 ms boundaries, the collision, the limits and the finish are not tested with sleeps.
- **OVL-02, COMPLIANT**: plugin-api `contract`/`workflows`, reference-extension
  `smoke`; presentation `result_rejection_preserves_current_payload_focus_and_conversation`
  and `declarative_help_opens_real_host_overlay_without_notification`.
- **OVL-03, COMPLIANT**: state serializes the requests, invalidates leases/channels,
  cancels the waiters and the shutdown; workflows
  `approval_preempts_secret_workflow_and_rejects_all_ordinary_requests`.
  PTY `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` and
  `approval_resize_below_minimum_disarms_until_a_fresh_draw` verify the fresh input
  and the rejection of a valid code below the minimum before the viable redraw.
- **OVL-04, COMPLIANT**: PTY of fragmented paste, Ctrl+C, unload/refresh and revocation
  of the selection; workflows `revoked_origin_and_reregistered_name_cannot_mutate_replacement`,
  `late_result_after_cancel_does_not_replace_a_new_form` and
  `submission_moves_bounded_utf8_secrets_and_freezes_captured_input_once`.
  Providers_flow `assert_surfaces` inspects the JSON session, the metadata and the temporary
  files, including WAL/SHM; the PTY inspects the screen, the transcript and the scrollback.
- **OVL-05, COMPLIANT**: providers_ui and app providers_flow exercise the login,
  the selection, the model change, Confirm/Logout, the errors, the failed Secure
  storage without fallback, the explicit Ephemeral and the unsupported OAuth. Turns and
  revocation are also covered in application/runtime. They are controlled
  services and local HTTP, not production credentials nor external providers.
- **BAR-01, COMPLIANT**: PTY
  `overlays_keep_status_visible_at_minimum_inline_size` (100x30/20x8),
  `actual_terminal_resize_preserves_bar_and_restores_overlay_surface`, narrow
  Approval and lib zero-area/Unicode clipping. The new PTY tests the separated scrollback.
- **BAR-02/03, COMPLIANT**: state
  `connection_attachment_rejects_stale_sources_revisions_and_retired_close` and
  `watch_attachment_initial_read_coalesces_concurrent_updates_without_gap`;
  application `connections`/`providers_contracts`/`providers_turns` and runtime
  `providers_integration` cover stage/confirm, faults, teardown and reconnection.
  Lib `status_bar::tests` distinguishes the local selection from Connected, absence,
  activity, feedback and diagnostic. No remote availability is asserted.
- **SET-07/08/12, COMPLIANT**: state
  `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`,
  `settings_failed_apply_preserves_copy_and_error_and_can_retry`,
  `late_settings_receipt_preserves_new_working_copy_and_focus`,
  `replaced_settings_writes_remain_serialized_in_submission_order`,
  `settings_started_write_finishes_after_shutdown_without_reopening_or_notifying`
  and `stale_settings_input_cannot_mutate_replacement_or_cancel_turn`.
  They use real barriers/receipts, without promising a rollback of the started writes.
- **SET-01 to 06/13/14/16, COMPLIANT**: lib settings, completion/plugins, state
  `settings_catalog_and_shell_navigation_never_persist_and_add_provider_only_notifies`,
  replacement workflows and streaming PTY. No live theme nor onboarding is added.
- **SET-09/10/11/15, COMPLIANT**: huginn bin, config_compat, cli and tui;
  they preserve the external/unknown bytes, the defaults and the line mode without the Settings TUI.

## Root cause and scope of the changes

The PTY observer did not scroll the rows when receiving LF at the bottom edge. The
harness regression failed first; now it retains the evicted rows and has a positive
control. It is a model of the `append_lines` protocol of the current inline backend,
not a complete terminal emulator. The resize preserves the existing scrollback.

The driver blocked Enter and the completion execution when `busy` was true,
including UI commands. The new PTY failed before the fix. Now the UI commands
are executed during the streaming; sending a conversational prompt still requires
that there is no active turn. Authority, payloads nor Approval are changed.

Consumption wildcards were replaced by explicit variants in Output (Event),
the two OverlayKind validations and the PublicationFault wrapper (ProviderAction).
The UiState projection and the router already had exhaustive matches. Examples and
fixtures keep their semantics; neither Rook nor the runners are touched.

## Limits, load and handoff

- `UNTESTED`: real external provider and OS secure store; the tests use
  doubles, local HTTP and temporary metadata/SQLite. This is not presented as an environment
  failure and no successful remote calls are invented.
- `UNTESTED`: manual user test and independent Verify/QA; they are delegated
  afterwards with their own reports. Archive is not executed.
- Two opt-in tests remain ignored by their runners: interoperability with the
  pre-rename binary (`CORTEX_AGENT_LEGACY_BIN`) and the OS secret store (Keychain/Secret Service). They are not counted as approved nor are external credential entries written
  to activate them.
- The audit recipe incorporates `|| true`; its exit does not certify security by itself.
  This limitation of the real gate is preserved; the direct execution without a mask exited 0.
- Approved `size-exception` strategy, base/HEAD `main` without new branches. The real
  diff exceeds the 2500 to 5500 line estimate: first measurement of 8769 tracked
  lines when entering this continuation, plus the untracked `status_bar.rs`: the excess
  was already prior to 6.2 to 6.3. A final measurement separating
  artifacts and lockfile will be reported, without hiding them nor creating commits/PRs.
- Preserved index: SHA-256
  `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
  `state.yaml` stays under the exclusive control of the orchestrator.

Final measurement against HEAD `442e34b`, including untracked: 55 files, 8894 additions + 667 deletions = 9561 lines; artifacts 2343, Cargo.lock 1, code/tests/docs 7217. Reproducible detail: `review-size-final.txt`. Size exception approved; no PR nor commit created.

## Gate and next owner

- `COMPLIANT`: the unit runners and the real `just ci-local` gate, complete.
- Historical `FAILING`, resolved: two behavioral REDs and the watch test barrier;
  the same expectations pass after the documented adjustments.
- Additional `openpty` OS -6 failure: the scenario was not executed in that run; it passed in the
  final CI. The definitive allocation cause is not established; the environment risk is
  preserved, it is not presented as a product regression and the runner is not modified.
- `UNTESTED`: the external/opt-in cases and the independent acceptance described above.

No pending Apply tasks remain. The orchestrator updates the state and delegates
`sdd-verify`; then `sdd-qa` produces its independent report. Without publication
nor Archive. The final code is covered by CI; the later changes are only
the documentary closure of these three artifacts.
