# Apply progress, huginn-overlay-system

## V-01 fix after failed Verify (2026-10-20)

Accumulated state: the 21/21 tasks and all their historical evidence are preserved.
This block replaces the previous closure for the modified candidate: Verify
detected a real V-01 in SET-05; conformance is not inferred from the previous suites.
Authorized scope: fix V-01 and add a host regression, without widening the design.
Previously approved `size-exception` delivery, base/HEAD `main`, without chain nor PR.
The dirty worktree is the candidate explicitly received for this fix;
no prior work was restored nor replaced. Only `overlay.rs`,
`tests/state.rs`, this artifact and `apply-validation.md` are edited.

- `verify-report.md`, the ui-settings base spec, the ui-settings delta,
  the design, the tasks and the previous progress were read in full. REQ-05 is the specific exception to REQ-03:
  the horizontal arrows edit the two toast pickers; on the other tabs
  they navigate sections. Tab/BackTab always navigate. The contractual option
  of advancing also with Left is chosen; the footer, the spec nor the cycle values are changed.
- Real RED in the repo: `settings_horizontal_arrows_cycle_toasts_without_leaving_the_row`
  via `/settings` → real host input with ticket/focus → Tab×3 → Right, exit 101,
  observed level 0 against 1. GREEN from the same command exit 0. It checks both
  full cycles, both arrows, labels, selection and staying on tab 3.
- Minimal fix: seven lines in the Settings match of `overlay.rs`, before the
  generic navigation. It does not change leases, receipts, Approval, streaming nor I/O.
  The test refactor explicitly releases guards after the assertions;
  it does not change expectations nor add lint suppressions.
- Additional host characterization: `settings_section_keys_preserve_noncycle_values_and_escape_toasts`
  preserves arrows, wrap and Tab/BackTab without changing values; the async
  `settings_cycle_arrows_only_persist_displayed_values_on_enter` checks zero
  writes during editing/discarding, a single Enter with warn/5s, receipt and reopening,
  without conversation/editor nor cancellation of the active turn.
- Ten Settings tests PASS before and after the refactor; the seven Huginn packages
  with all-features: 391 PASS reported in 57 blocks (they include subprocesses),
  2 ignored, 0 failed. Presentation 158 PASS, including state 35 and PTY 39.
- The first new `just ci-local` fails in 3/9, Clippy: three guards of the new
  tests required explicit release (`significant_drop_tightening`). `drop(state)`
  is added after the same assertions; Settings 10/10 and focused Clippy
  all-features/all-targets PASS. Second full CI completed: `just ci-local`
  exit 0, 9/9, 477 s (`ci-local-final.log` and `ci-summary.txt`); Rust workspace
  all-features PASS, including state 35 and PTY 39 with the runner parallelism,
  Vitest 24 files/210 tests and E2E 90/90 in three browsers (2.7 min).
  The historical CI is not used to certify this candidate and the runner is not changed.
- Additional run of the seven packages after the refactor (`focused-final.log`),
  simultaneous with CI, exit 101 in PTY (38 PASS/1 FAIL): the `approval_code`
  helper observed "Type " without eight available characters. Approval is not modified nor
  is a definitive cause attributed to environment/concurrency. Exact repetition of the
  case without changes exit 0, 1 PASS (`approval-recheck.log`); the risk is preserved.
  Later full repetition without changes (`focused-recheck.log`) exit 0,
  including state 35/35 and PTY 39/39; the case is not omitted and its timeout is not changed.
  The same case also passes in the final full CI. `cargo audit --no-fetch`
  direct and fmt/diff-check/artifact lint exit 0. The final code is covered by
  these gates; later changes are limited to the closure of these two artifacts.
- Exact logs and commands in `apply-validation.md`, approved directory
  `opencode/huginn-fix-v01/`. The initial staging matches the received SHA-256
  `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
- Pending: new independent verification `sdd-verify`; its previous FAIL is not modified,
  nor `state.yaml`, QA nor Archive. No staging/unstaging/reset/checkout,
  commit/push/merge/PR nor data reset. Skill resolution: `none` (without a project
  registry); common procedures and SDD compact rules consulted.

## Apply 6.2–6.3 resumption (2026-10-20)

**Verified closure:** 6.2 and 6.3 complete; accumulated state 21/21 tasks.
Final `just ci-local` exit 0, 9/9, 431 s; final code without later changes.
The in-progress command paragraphs of this section are historical and are
replaced by this closure, without deleting their failures nor results.

- Accumulated state: 1.1–1.3, 2.1–2.6, 3.1–3.3, 4.1–4.4, 5.1–5.2 and 6.1 are preserved as complete; their tests and decisions remain in `tasks.md` and in the history below. Their implementation was not repeated.
- 6.2 complete as directed technical acceptance of Apply; it does not equal independent Verify nor QA. Own PTY scrollback coverage and repeated commands with a fake clock were added, preserving the previous matrix.
- Harness RED/GREEN: `cargo test -p huginn-presentation --features test-driver --test pty support::screen_line_feed_at_bottom_scrolls_instead_of_overwriting -- --exact`: RED exit 101 (`FIRST` instead of `SECOND`); GREEN exit 0, 1 PASS. The observer keeps the rows scrolled by the bottom LF and keeps the scrollback when resizing.
- Product RED/GREEN: `cargo test -p huginn-presentation --features test-driver --test pty overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback -- --exact`: RED exit 101, `┌Help` missing during retained streaming. Root cause: Enter and completion required `!busy` even for UI commands. GREEN exit 0, 1 PASS after allowing UI dispatch and keeping the busy barrier for conversational prompts. Without lowering assertions nor cancelling the streaming to open overlays.
- `cargo test -p huginn-presentation --test providers_ui repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation -- --exact`: 1 PASS, fake advance of 2001 ms; Help/Providers/Login/Model/Settings/Logout preserve conversation, stream and turn. Characterization without a fake RED.
- `cargo test -p huginn-presentation --features test-driver -- --test-threads=1`: 155 PASS, including 39 PTY, 32 state, 13 workflows, 10 parser and 8 providers_ui. `cargo test -p huginn --test tui --test providers_flow -- --test-threads=1`: 8 + 6 PASS.
- Exhaustive review: `Output` enumerates the ignored events, command/workflow validate the six OverlayKind variants explicitly and the PublicationFault wrapper distinguishes Select/StageSelection from the other four actions without a wildcard. Refactor without behavior change, supported by the existing suites and the new consolidation.
- Requirements read: `docs/development.md`, `dev/README.md`, manifests and the real recipes. Docker available, dependencies/browsers present, `just test-e2e-build` exit 0. The first `just ci-local` ended exit 0 (479 s, 90 E2E + 210 Vitest); it corresponds to the state before the fix and does not certify the final candidate by itself. A second full execution is in progress over the final code.
- Full logs directory: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-apply6/`. Detail per command and REQ in `apply-validation.md`.
- The approved `size-exception` is preserved. No staging, commit, push, branch, merge, PR nor archive operation. The index hash is still `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`; `state.yaml` was not modified and no data was reset.
- Consolidation of seven packages (`huginn-all-final.log`) finished exit 0: core, application, runtime, presentation, plugin-api, reference-extension and app, with `--all-features -- --test-threads=1`; two opt-in tests ignored (legacy binary and Keychain/Secret Service), not certified.
- Second CI (`ci-local-final.log`) ended exit 1 in 5/9: the PTY resize saw the new title before the watch updated the bar. Harness cause: two async tasks not synchronized by that title. The exact expectation is preserved and `Idle|fixture/Bar-Provider` is awaited before asserting it; focused exact exit 0 in `resize-watch-green.log`. A third full CI is in progress, `ci-local-verified.log`; the first run is not used as a substitute.
- `cargo audit --no-fetch` without a mask exit 0 (`audit-unmasked.log`), with no reported advisories. `just doctor` exit 1 because the documented recipe does not exist (`doctor.log`); the tooling was verified through real commands, without touching runners nor Rook. Own fmt/diff-check exit 0 after the changes.
- Additional PTY run with the default parallelism (`pty-concurrent-green.log`, the name does not imply success): 38 PASS/1 FAIL, exit 101 due to `failed to openpty`, OS -6, before running the refresh case. It coincided with the full CI; possible resource interference, no definitive cause demonstrated. The failure and the diagnostic are preserved (`pty-resource-diagnostic.log`, PTY limit 511); the runner is not changed and the CI parallelism is not reduced.
- Additional Clippy of the seven Huginn packages with `--all-features --all-targets -- -D warnings`: exit 0 (`huginn-clippy-allfeatures.log`), including test-driver/fixture/PTY. Offline example `providers_headless`: exit 0. Lint of tasks/progress/report: exit 0 after correcting a historical code span without changing its literal.
- The final gate was read in full and summarized in `ci-summary.txt`: `just ci-local` exit 0, nine stages PASS, 431 s; cargo test workspace/all-features PASS, including 39 PTY (default parallelism), providers_ui 8, state 32, workflows 13 and app tui 8. Vitest 24 files/210 tests; E2E 90/90 in Chromium/Firefox/WebKit, 3.2 min. Logs `ci-local-verified.log`, copy `dashboard-dev.log`. The extra run with the openpty OS -6 failure is preserved; the refresh case passed in the final gate, without changing the runner nor asserting a definitive cause of the allocation failure.
- `apply-validation.md` contains REQ→test/evidence, RED/GREEN, real gates, previous failures/UNTESTED and the exhaustive consumer review. There are no pending Apply tasks. Handoff to `sdd-verify` and then `sdd-qa`; no reports of those phases are created and Archive is not done. The orchestrator keeps the state transition.

## Current state (2026-10-20, phase 5 continuation)

- I confirmed the scope with tasks 5.1–5.2, the four relevant delta specs and the design; I did not modify `state.yaml`, git staging nor the branch.
- RED: I added a PTY fixture Approval test in a 20x8 terminal; it failed because the long footer did not fit nor appeared as complete text in the screen model. An additional RED detected that the `wait_for("Permission #")` helper could observe the panel border before the footer was painted; the test inspects the screen model and awaits the challenge/field before the close.
- GREEN: narrow Approval uses the compact instructions `Approve: <challenge>` and `Code: <typed>` in the two available lines. The test asserts the challenge (which includes the fresh eight-character code) and the field.
- `cargo test -p huginn-presentation --features test-driver --test pty minimum_terminal_approval_shows_challenge_and_code_field -- --exact` — 1 passed.
- `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1` — 35 passed, 0 failed; full log: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-phase5-verified.log`. Existing PTY evidence includes `dropping_connection_restores_synchronously` and normal exit/cancel with restored termios, bracketed-paste disable, no alt screen.
- `cargo test -p huginn-presentation status_bar::tests --lib` — 5 passed (absence/no false Connected, activity, diagnostic, unicode clipping).
- `cargo test -p huginn-presentation --test state` — 32 passed (Approval arm/disarm/lease and associated states).
- `cargo fmt --all -- --check` and `git diff --check` pass; at first fmt detected the format of the new test, rustfmt was applied and both checks repeated green.
- Still incomplete: real PTY matrix of all the overlays and no-overlay at the usual size/20x8, real PTY resize (including resize below the minimum→recovery and re-arm only after redraw with current identity), zero-size draw with a real backend and a test of a long non-conversational panel/summary and connected/unselected bar. I do not close 5.1/5.2 nor start phase 6 until I provide focused evidence for these criteria.

## Verified current state (2026-10-20, PTY continuation)

- I reproduced the serial suite and the five cases of the previous record. Result before the test adjustment: 29/34, with the five historical failures.
- Cause per case: `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` used `wait_for("END_OF_DIFF")`, although the render limits the preview to the visible viewport; the marker text existed in the logical buffer, not on the screen. The criterion was corrected to `PREVIEW_FIRST`, an element that is indeed visible, preserving the fresh code/denial assertions.
- The three revoked selection cases did process the input flow and showed Help, but the helper sent Ctrl+C immediately after finding Help; that closed the render/UI turn, without guaranteeing the overlay close and the exit before the timeout. Now Esc is sent and Help is awaited to disappear before Ctrl+C. There is no evidence of a product regression in those three cases; the assertions against `NEW_COMMAND_EXECUTED` are preserved.
- `resizing_during_incomplete_secret_paste_does_not_start_another_reader`: the test depended on looking for `Form discarded.` in the raw ANSI transcript, whose text is segmented by cursor movements. It was changed to observe the reconstructed screen buffer. `Pty::resize` is used to also update the observation grid to the new size. The assertion of a single cursor query and the discard of both fragments are preserved.
- Fresh result: `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1` — **34 passed, 0 failed**. Full log reproduced a second time: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-final-verified.log` (34/34); the previous run was also green: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-rerun.log`. Base reproduction log (29/34): `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-current.log`; previous reference log: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-final.log`.
- No product code was modified: the evidence points to criteria/observation/race in the harness, not to defective behavior. I did not stage/unstage nor perform other Git operations; `state.yaml` was not touched. Tasks 5.1–5.2 remain open and unmarked; phase 6 was not started.

## PTY continuation history (2026-10-20; hypotheses and failures superseded by the verified state above)

- I resumed on the worktree with the preexisting staging intact; I did not perform stage/unstage operations, I did not touch `state.yaml` nor external Rook/Settings code.
- I investigated the real matrix. The render identifies Help/Settings by their title, but Provider/Login/ModelPicker/Confirm workflows are presented as a generic form (for example real title `Bar Provider`, credential), and the notifications on close help the lower cases. The PTY has an important limitation: its screen model is a grid of the initial dimensions; its `resize()` resets the model and that test driver setup does not necessarily match the inline bottom row of the application. I observed contradictory real outputs when interpreting the position with `status_line()`, so I did not consolidate a misleading test. The temporary test reported an empty status bar under Help; the PTY transcript did show `Idle|fixture/alpha` at row 13, which confirmed an observer limitation. I removed those temporary tests.
- I validated the full serial PTY suite after the attempts: 29/34 pass. Five fail: `approval_does_not_erase_a_revoked_selection` and `revoked_palette_selection_cannot_fall_back_to_replacement_dispatch` complete the Approval close, open Help and then the fixture does not finish; `ctrl_enter_cannot_execute_a_revoked_selection` stays with `REPLACEMENT_READY` after Help and does not finish; `resizing_during_incomplete_secret_paste_does_not_start_another_reader` fails waiting for the cancellation to happen during the resize; `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` does not see `END_OF_DIFF` in the visible buffer (cropped preview, it may be a bad visible PTY criterion). Log: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-final.log`. I reproduced the five individually in series; I cannot attribute them to the change nor assert flakiness: the suite does not go green and they require triage.
- `cargo fmt --check` passes. `cargo test -p huginn-presentation --lib status_bar::tests` and `cargo test -p huginn-presentation --test state` pass (consulted focused results; the output was truncated by the wrapper).
- I did not manage to implement/validate a final matrix, a `TestBackend(0,0)` with a real renderer and recovery/disarm, nor a dedicated PTY lifecycle with bracketed-paste/termios after shutdown. Neither production nor the permanent test code was modified in this resumption.
- Due to insufficient evidence, 5.1 and 5.2 remain pending in `tasks.md`. This continuation is not PR-ready. Next technical step: isolate the five PTY timeouts and adjust/create a harness that can check the bottom row status in both dimensions without depending on stale cells nor on a wrong grid dimension; then cover the real zero-size renderer and shutdown before reconsidering the checklist.

## Actionable/status continuation (2026-10-20)

- Cause confirmed in code: `cancel_workflow_at` publishes `Notification` (`Form discarded.`), which previously was projected to the single `status.notice`; `start_file_index` then writes `Partial file index...` with `set_notice`, replacing it. The status bar render reads that same slot, so the notification was visible in one frame and then disappeared when refreshing the completion/index.
- TDD: I added `status_bar::tests::actionable_notification_survives_informational_completion_refresh` first; RED confirmed (it received `Partial file index...` instead of `Form discarded.`). GREEN with a separate actionable feedback slot (`feedback`, associated diagnostic) and an informative notice. Notification/diagnostic update the feedback and the new `begin()` clears the previous feedback; informative refreshes do not erase it. It keeps the existing types/events/APIs.
- Focused unit tests `cargo test -p huginn-presentation status_bar::tests --lib`: 5 passed. `cargo test -p huginn-presentation --test state`: 31 passed. `cargo test -p huginn-presentation --test workflows`: 13 passed.
- Later independent serial PTY: `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1`, 40 passed, log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-overlay-pty-1792479397.log` (13.51s). A previous serial run, `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-overlay-pty-serial-20261020.log`, failed 6/40 in 72.77s, including `long_plugin_dialog_window_keeps_selection_visible` and five cancellation/secret tests. The full repeated run passes, a sign of flakiness/transient timeout compatibility; no definitive cause is attributed and no failure is hidden.
- I did not add a PTY of notice persistence plus refresh: the current fixture does not control/invoke the partial scanner/index

## Apply 5.1–5.2 resumption (2026-10-20)

- The staging was kept intact; no git operations were executed.
- I added `actionable_feedback_and_diagnostic_reset_on_new_operation` to `tests/state.rs`: it verifies that the actionable error diagnostic is projected and that `begin()` clears both the content and the diagnostic flag.
- I expanded `overlays_keep_status_visible_at_minimum_inline_size` for the open/close cycle and the bar inspection with Help, Settings, Provider, Login, ModelPicker and Confirm at 20x8; in addition, Approval is verified at normal size with a real Challenge/code. I did not add extensive coverage of each overlay at both sizes: it remains pending for 5.2.
- `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1`: **40 passed**, full log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-overlay-pty-followup-final3.log` (13.49s). It includes `approval_preview_scrolls_to_both_ends_at_minimum_inline_size`, `approval_resize_below_minimum_disarms_until_a_fresh_draw`, `inline_stream_composer_cancel_and_normal_exit_restore_termios`, `dropping_connection_restores_synchronously`, and panic/restoration tests.
- `cargo test -p huginn-presentation status_bar::tests --lib` (5 passed), `cargo test -p huginn-presentation --test state` (32 passed), and `cargo fmt --all -- --check` (passes) were also executed.
- **Gaps that prevent completing 5.1/5.2**: the complete matrix of *each* `Overlay` variant and no-overlay at both sizes is not demonstrated; the new check covers the main public variants at 20x8 and the normal Approval, complemented by the existing PTYs, but not a complete matrix. The 20x8 view does not prove that the Approval challenge is visible (the body/footer are cropped); the preview PgUp/PgDn is covered independently. There is no viable test of `Rect::default()`/zero-size draw; recovery and explicit re-arm after zero-size were not validated. Unicode cell clipping is already in the existing unit tests. Bracketed-paste/termios is verified in exit/cancellation/drop and in the suite panic, but not in an independent explicit shutdown case.
- Consequently, 5.1 and 5.2 remain `[ ]`; phase 6 is not started. No acceptance, verify/QA nor task closure is claimed.

## Apply phase 5 continuation (2026-10-20; closure by evidence)

- The test source and the historical logs were audited. The previous checkpoint was outdated: a matrix at both sizes, the zero-area TestBackend, the status bar matrix, Approval 20x8 and the cancel/drop terminal tests already existed.
- RED/GREEN: I extended `approval_resize_below_minimum_disarms_until_a_fresh_draw` to capture the current code at 8x20, switch to 1x1, send the valid code without obtaining approval and recover 8x20; the same code is accepted after the redraw. The initial attempt failed while looking for `"Type "` which does not fit in the small viewport and later while assuming that the disarmed submit should resolve the request; I adjusted the test to its observed contract: no acceptance below the minimum and authorization post-redraw.
- `dropping_connection_restores_synchronously` opens the `drop` fixture; the fixture discards the connection and asserts that `is_raw_mode_enabled()` is false before exiting. It does not inspect ICANON/ECHO. `inline_stream_composer_cancel_and_normal_exit_restore_termios` does verify ICANON/ECHO after cancellation plus normal exit, bracketed-paste disable and the absence of the alternate screen.
- I ran the exact filters for resize, drop and cancel/exit (1/1 each). Serial PTY `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1`: 37/37, full log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-apply-phase5-final.log`.
- `cargo test -p huginn-presentation status_bar::tests --lib`: 5/5, log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-statusbar-phase5-final.log`; `cargo test -p huginn-presentation --test state`: 32/32, log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-state-phase5-final.log`.
- `cargo fmt --all -- --check` and `git diff --check` pass after rustfmt; logs `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-fmt-phase5-final.log` and `huginn-diffcheck-phase5-final.log`.
- The combined evidence closes 5.1/5.2 and corrects their checkpoints, without attributing explicit termios coverage to the drop case.
