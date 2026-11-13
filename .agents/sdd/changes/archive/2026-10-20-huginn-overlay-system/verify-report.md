# Verification Report

## Result

- Change: `huginn-overlay-system`.
- Date: 2026-10-20. Persistence: openspec, explicit path of this mission.
- **Current verdict: PASS WITH WARNINGS** after the V-01 reverify on 2026-10-20.
  V-01 and V-02 resolved with independent evidence; no blocking technical
  defects identified. The external and tooling limits V-03 are preserved.
- Initial verdict: **FAIL** due to REQ-05; history and RED evidence preserved below.
- Independent technical verification; it does not certify user acceptance nor QA.
- Execution mode: **fallback**. No `sdd-quality-runner.mjs` was found in
  the project nor in the consulted local SDD resources. There was no deterministic
  enforcement nor versioned envelopes. `testing.strict_tdd: false`; the
  Strict TDD module was not loaded.
- Skill resolution: `none`; there is no `.agents/skill-registry.md`.

| Completeness                             | Result                                                                            |
|-----------------------------------------|--------------------------------------------------------------------------------------|
| Main tasks declared complete | 21/21                                                                                |
| Subchecklist 2.2                        | 3/3                                                                                  |
| Artifacts read                        | proposal, four delta specs, ui-settings base spec, design, tasks, apply-validation |
| Technical conformance                     | Certified in the executed scenarios; V-01 resolved                               |
| QA / user acceptance             | Pending; not inferred from Apply nor from Verify                                         |

The final code was inspected, not only the Apply COMPLIANT tags.
Production, repository tests, staging nor `state.yaml` were not modified.

## Reverify V-01: current evidence and decision

The fix in `overlay.rs:140–151`, the three new host tests in
`state.rs:742–864` and the new block of `apply-validation.md` were read. No spec
was lowered nor the contractual scenario replaced by Space. The new branch is
limited to Settings in toasts: Left/Right invoke `toggle_selected`, while
Tab/BackTab keep changing tab and the other tabs keep the horizontal
arrows. REQ-05 allows Left to also advance the cycle. This concrete rule
of pickers takes precedence over the general navigation of REQ-03; Tab/BackTab
allow leaving toasts. No live application nor toast layer was added.

Independent reverify directory (from now on `R/`):
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-reverify-v01/`.
All commands were executed from `/Users/acosta/Dev/dallay/cortex`.

| Command / current evidence                                                        | Exit | Result                                                                                                                                      |
|-----------------------------------------------------------------------------------|------|------------------------------------------------------------------------------------------------------------------------------------------------|
| `cargo test -p huginn-presentation --all-features -- --test-threads=1`            | 0    | `R/presentation.log`: 158 PASS, 0 failed, 0 ignored; lib 38, state 35, PTY 39 and the remaining targets                                              |
| Recompilation of the independent original source `V/settings-cycle.rs` with rustc   | 0    | `R/settings-cycle-build.log`; rlibs obtained from the fingerprint of the state target just executed; exact command in `R/independent-command.txt` |
| `R/settings-cycle --nocapture`                                                    | 0    | `R/settings-cycle.log`: 1 PASS; `active_tab=3, level=1`; the same source and assertions that failed before                                          |
| `cargo fmt --all -- --check` and `git diff --check`                                 | 0    | `R/static.log`                                                                                                                                 |
| `cargo clippy -p huginn-presentation --all-features --all-targets -- -D warnings` | 0    | `R/static.log`; includes tests/fixture                                                                                                          |

The current suite repeats all the presentation cases of the initial matrix,
including single authority, Approval fail-closed/resize/0x0, secrets, scrollback,
late receipts and watch. It also runs these regressions through the real host router:

| REQ → scenario of the fix                                                                                                                | Executed test                                                     | Result                                                                |
|----------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------|--------------------------------------------------------------------------|
| SET-05: level and duration, both arrows, full cycles and labels                                                                     | `settings_horizontal_arrows_cycle_toasts_without_leaving_the_row`  | PASS: it does not change tab/selection nor the other value                           |
| SET-03/04/06: ordinary tabs, wrap, leaving toasts and intact values                                                               | `settings_section_keys_preserve_noncycle_values_and_escape_toasts` | PASS: Tab/BackTab remain navigation; it does not edit non-cyclic values |
| SET-06/07/08/13 and EVT-01: no write before Enter, visible values saved together, Esc discards without cancelling the turn nor contaminating the UI | `settings_cycle_arrows_only_persist_displayed_values_on_enter`     | PASS: warn/5s, one write and reopening with the saved snapshot          |

The final log and the Apply summary in `huginn-fix-v01/` were reviewed:
`ci-local-final.log`, `ci-local-final.exit` and `ci-summary.txt` certify exit 0,
9/9, 477 s, 210 Vitest and 90 E2E. `focused-recheck.log` documents the green
repetition of the seven packages. It is inherited evidence from Apply, not CI executed
by reverify. The independent repetition was limited to the modified package and its
tests; it was not necessary to repeat broad CI nor the packages without product changes.

The intermediate Apply failure in PTY
`stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` is preserved: the helper did not
obtain eight challenge characters. It does not certify undue authorization and no
definitive cause is established. It passed afterwards, without modifying assertions nor that
code, in the repetition of the seven packages, the final CI and the 39 PTY of this reverify.
It is a PTY observation risk, not an active failure nor a reason to omit the case.

**Decision:** V-01 corrected and V-02 covered. The external/opt-in limits of V-03
do not block the pass to QA because OVL-05 requires controlled services, not real
external credentials. They do not equal an approved operational integration. The
handoff to **sdd-qa** is allowed, not Archive nor user acceptance. Fallback mode and the absence
of instrumental coverage remain declared; UNAVAILABLE does not become a PASS.

## Initial execution and evidence (historical)

Independent evidence directory (from now on `V/`):
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/`.
All commands were executed from `/Users/acosta/Dev/dallay/cortex`.

| Command / evidence                                                                                                                                                                        | Exit | Result                                                                                                                   |
|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------|-----------------------------------------------------------------------------------------------------------------------------|
| `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features -- --test-threads=1` | 0    | `V/focused-tests.log`; 388 PASS reported in 57 blocks, including subprocess results; 2 ignored, 0 failed          |
| Presentation inside the previous command                                                                                                                                                   | 0    | 155 PASS: lib 38, completion 9, extensions 2, files 2, parser 10, plugins 2, providers_ui 8, PTY 39, state 32, workflows 13 |
| `cargo fmt --all -- --check`                                                                                                                                                               | 0    | `V/static.log`                                                                                                              |
| `git diff --check`                                                                                                                                                                         | 0    | `V/static.log`                                                                                                              |
| `cargo check -p huginn-application --examples`                                                                                                                                             | 0    | `V/static.log`; additional check to the tests                                                                          |
| Independent test `V/settings-cycle --nocapture`                                                                                                                                        | 101  | 0 PASS, 1 FAIL; `V/settings-cycle.log`, V-01                                                                                |
| Instrumental coverage                                                                                                                                                                     | n/a  | UNAVAILABLE: llvm-cov was not executed; no percentage is declared                                                               |

The independent test was written outside the repository in
`V/settings-cycle.rs`. It compiled correctly with the following command,
using the same artifacts as `tests/state.rs` of this execution:

```sh
rustc --edition=2021 --test /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/settings-cycle.rs -L dependency=target/debug/deps -o /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/settings-cycle --extern huginn_presentation=target/debug/deps/libhuginn_presentation-f028ce01cd85534f.rlib --extern crossterm=target/debug/deps/libcrossterm-19462bd5fb984d15.rlib --extern huginn_core=target/debug/deps/libhuginn_core-b0fd28370cd72cd2.rlib
```

Build exit 0 in `V/settings-cycle-build.log`. The first link attempt had
selected rlibs of different features and failed with incompatible types; it was not
a product regression. It was resolved by reading the hashes of the fingerprint
`huginn-presentation-537756dcf8e42867/test-integration-test-state.json`.
The cited FAIL corresponds to an executed behavioral assertion, not to compilation.

The Apply summary and final log in `huginn-apply6/` were also read:
`ci-local-verified.log` / `ci-summary.txt`, 9/9 exit 0, 431 s, 210 Vitest,
90 E2E. It is preserved as inherited evidence, not as CI executed by Verify.
Broad CI was not repeated because the modifications are in Huginn and the focused
repetition covers its seven packages. The historical openpty OS -6 failure did not reappear
in the 39 PTY of Verify; this does not establish its definitive cause.

## Behavioral conformance matrix

The following names correspond to tests that passed in `V/focused-tests.log`.
The presentation cases passed again in `R/presentation.log`; SET-05 is
updated with the reverify evidence. `lib` refers to huginn-presentation;
the targets are identified with their package when they do not belong to presentation.

| REQ → scenario                                                              | Executed test / evidence                                                                                                                                                                                                 | Technical state                                                       |
|------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------|
| OVL-01-A                                                                     | workflows `declarative_help_opens_real_host_overlay_without_notification`; lib `render_snapshot_and_input_share_authority_and_redact_secrets`; PTY streaming/scrollback                                                    | COMPLIANT                                                            |
| OVL-02-A                                                                     | workflows `declarative_help_opens_real_host_overlay_without_notification`; plugin-api contract/workflows                                                                                                                   | COMPLIANT                                                            |
| OVL-02-B                                                                     | workflows `result_rejection_preserves_current_payload_focus_and_conversation`, `approval_preempts_secret_workflow_and_rejects_all_ordinary_requests`; plugin-api validators                                               | COMPLIANT                                                            |
| OVL-03-A                                                                     | state `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`, `approval_retains_entire_diff_and_requires_fresh_armed_code`; PTY stale/fresh                                                              | COMPLIANT                                                            |
| OVL-03-B                                                                     | state `approval_sender_loss_is_denial_and_late_input_has_no_authority`, `dropping_approval_waiter_retires_authority_and_sender`, `approval_cancellation_wins_ready_answer_and_shutdown_wakes_serial_waiters`               | COMPLIANT                                                            |
| OVL-03-C                                                                     | state `approval_requests_serialize_and_stale_lease_cannot_resolve_successor`; workflows preemption/rejection                                                                                                                 | COMPLIANT                                                            |
| OVL-04-A                                                                     | PTY `ctrl_c_cancels_incomplete_secret_paste_and_discards_its_remaining_bytes`, `closing_secret_form_discards_fragmented_paste_and_accepts_fresh_input`; app providers_flow                                                 | COMPLIANT                                                            |
| OVL-04-B                                                                     | workflows `revoked_origin_and_reregistered_name_cannot_mutate_replacement`, `late_result_after_cancel_does_not_replace_a_new_form`, `reentrant_callbacks_cannot_commit_stale_tickets_and_pending_drop_is_outside_lock`     | COMPLIANT                                                            |
| OVL-05-A                                                                     | providers_ui 8; huginn/providers_flow 6; application/providers_turns; runtime/providers_integration                                                                                                                        | COMPLIANT with controlled services                                  |
| OVL-05-B                                                                     | providers_ui and providers_flow: Secure failed, explicit Ephemeral, validation, logout/cancellation; runtime secret_store with double                                                                                         | COMPLIANT with controlled services                                  |
| EVT-01-A                                                                     | providers_ui `repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation`; PTY `overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback`                                                          | COMPLIANT                                                            |
| EVT-01-B                                                                     | state `explicit_ui_categories_do_not_depend_on_message_text`                                                                                                                                                               | COMPLIANT                                                            |
| EVT-02-A                                                                     | state `resume_and_synchronize_commit_messages_once_not_mutable_tail`, `tool_error_uses_contributed_label_and_marks_status`; runtime stream_persistence                                                                     | COMPLIANT                                                            |
| EVT-02-B                                                                     | state `session_reprojection_keeps_all_messages_beyond_ui_buffer_capacity`; lib `ui_event_buffer_keeps_only_the_latest_capacity_in_fifo_order`; runtime sessions_persistence                                                | COMPLIANT                                                            |
| EVT-03-A                                                                     | lib `dedup_uses_half_open_two_second_window_and_reanchors_visible_emission`; state error producer                                                                                                                          | COMPLIANT                                                            |
| EVT-03-B                                                                     | lib `dedup_interleaved_operations_do_not_suppress_each_other`; state `explicit_event_operation_scope_survives_interleaved_producers`                                                                                       | COMPLIANT                                                            |
| EVT-03-C                                                                     | lib `dedup_scopes_operations_and_compares_full_text_on_hash_collision`                                                                                                                                                     | COMPLIANT                                                            |
| EVT-04-A                                                                     | lib `dedup_enforces_entry_byte_and_oversize_bounds_without_truncation`, `dedup_purges_expired_entries_and_releases_retained_bytes`, `dedup_nonce_exhaustion_emits_without_panicking_or_caching`                            | COMPLIANT                                                            |
| BAR-01-A                                                                     | PTY `overlays_keep_status_visible_at_minimum_inline_size`, `actual_terminal_resize_preserves_bar_and_restores_overlay_surface`, `approval_resize_below_minimum_disarms_until_a_fresh_draw`; lib zero-area and status_bar     | COMPLIANT                                                            |
| BAR-02-A                                                                     | state `watch_attachment_initial_read_coalesces_concurrent_updates_without_gap`; application/providers_contracts `staged_selection_requires_exact_commit_and_snapshot_is_safe`                                              | COMPLIANT                                                            |
| BAR-02-B                                                                     | state `connection_attachment_rejects_stale_sources_revisions_and_retired_close`                                                                                                                                            | COMPLIANT                                                            |
| BAR-02-C                                                                     | application/connections fault injection/publication/reopen; providers_contracts `selection_watch_tracks_exact_teardown_and_activation_source`, `adapter_self_cancellation_publishes_absence_before_unload`; core lifecycle | COMPLIANT                                                            |
| BAR-03-A                                                                     | lib status_bar tests; application/providers_contracts staged snapshot; PTY bar                                                                                                                                           | COMPLIANT                                                            |
| BAR-03-B                                                                     | providers_contracts failure/revocation; application/connections publication faults; lib status_bar feedback/diagnostic                                                                                                     | COMPLIANT: no remote availability is emitted, not even after success |
| SET-07-A/B/C                                                                 | state `settings_failed_apply_preserves_copy_and_error_and_can_retry`, `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`; lib projected config                                                       | COMPLIANT                                                            |
| SET-07-D                                                                     | state `late_settings_receipt_preserves_new_working_copy_and_focus` (success/failure)                                                                                                                                           | COMPLIANT                                                            |
| SET-08-A/B                                                                   | state receipt/preemption/streaming; PTY streaming; `settings_started_write_finishes_after_shutdown_without_reopening_or_notifying`                                                                                         | COMPLIANT                                                            |
| SET-12-A/B/C/D                                                               | state stale input and receipt/Approval; workflows authorized replacement, result rejection and preemption                                                                                                                      | COMPLIANT                                                            |
| SET-01, SET-16: registration, whitespace, catalog and no prompt                   | state `settings_catalog_and_shell_navigation_never_persist_and_add_provider_only_notifies`; completion/plugins; PTY streaming                                                                                              | COMPLIANT                                                            |
| SET-02/03/04: structure, tabs and rows/markers                               | lib settings snapshot/tab/item; state shell; PTY size matrix                                                                                                                                                         | COMPLIANT                                                            |
| SET-05: themes/defaults/booleans                                             | lib settings `snapshot_items_match_locked_d6`; huginn config_compat/bin defaults                                                                                                                                           | COMPLIANT                                                            |
| SET-05: toasts, `→` over level: info → level: warn                          | Recompiled original independent test: active_tab=3, level=1; three new host tests in `R/presentation.log`                                                                                                           | **COMPLIANT, V-01 resolved**                                         |
| SET-06: navigation without write                                             | state shell; lib settings; counted closures                                                                                                                                                                               | COMPLIANT                                                            |
| SET-09/10/11: external bytes, absent/partial/complete defaults, unknown | huginn bin tests 15 and config_compat 5                                                                                                                                                                                      | COMPLIANT                                                            |
| SET-13/14: Esc does not cancel the turn, Add provider without I/O                        | state shell/stale input; PTY streaming                                                                                                                                                                                     | COMPLIANT                                                            |
| SET-15: line-mode exclusion                                               | huginn tui and cli                                                                                                                                                                                                           | COMPLIANT                                                            |

## Fix and design inspection

| Area                      | Independent inspection                                                                                                                                                                                                                | Result                                                                         |
|---------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------|
| Single authority D1/D8     | `state.rs:158–192`: only `overlays` owns the active payload; the modal/workflow/settings accessors are projections; the `terminal.rs` snapshot and input query the same machine                                                                 | Compliant                                                                          |
| Host-only D2/D9           | The plugin API does not include Approval; the `overlay.rs:401–427` machine validates host/ticket/origin; Approval keeps a sender separate from PluginDialog                                                                                               | Compliant                                                                          |
| Fail-closed resize        | `terminal.rs:523–549,909–918` disarms an unviable size and only arms after a viable paint; the PTY sends a valid code at 1x1 and then at 20x8; lib covers 0x0                                                                                      | Compliant in the executed scenarios                                                 |
| UI and persistence D3/D7   | `state.rs:284–300` separates the categories; `terminal.rs:564–575` only inserts conversation; the FIFO in event.rs is not authoritative; the runtime recovers a session >256                                                                                | Compliant                                                                          |
| Selection/watch D4        | providers publishes with mutex/`send_replace`, requires the exact candidate; subscribe uses the same mutex; ModelPublication and MetadataMutation close fail-closed; the terminal uses borrow_and_update before the draw and attachment on update/close | Compliant                                                                          |
| Secrets D5/D6            | The overlay snapshot holds identity only; the workflow renderer masks; dedup keeps encrypted comparison, the 256/256KiB/4096 limits and the operation; the sentinel/JSON/PTY tests pass                                                           | Compliant to the test scope; it does not certify all future error sources |
| Settings receipts         | `state.rs:1051–1143` captures once, serializes the workers and consumes in order; a late success only updates the saved config; a late error does not modify the current overlay                                                                           | Compliant                                                                          |
| Size, clipping and scroll | The bar is reserved before the branches; clipping is per cell; the Approval preview is complete with PgUp/PgDn; the PTY 20x8/100x30, resize and restoration pass                                                                                                | Compliant                                                                          |
| Settings navigation       | `overlay.rs:140–151` prioritizes the toast cycles; Tab/BackTab keep the navigation and the other tabs keep the section arrows; the host regressions and the independent test pass                                                  | Compliant after reverify; the historical SET-05 breach resolved                  |

The design allows kind + typed payload instead of imposing literally a
payload variant for each surface; this choice is not a dual authority.
The reentrancy tests exercise callbacks and Drop outside the lock. No
independent legacy active slots nor durable contract changes were observed.

## Prioritized findings

| Finding                                                                | Judge A      | Judge B      | Severity           | Status                                                              |
|------------------------------------------------------------------------|--------------|--------------|--------------------|---------------------------------------------------------------------|
| V-01: SET-05 did not cycle toasts with →, overlay.rs:140–144 original        | Not executed | Not executed | Historical CRITICAL | RESOLVED: the original independent test PASSES in reverify              |
| V-02: the original coverage does not certify the keyboard of the SET-05 scenario      | Not executed | Not executed | Historical WARNING  | RESOLVED: three host regressions PASS                                |
| V-03: external/opt-in systems not executed                           | Not executed | Not executed | WARNING            | UNTESTED; it is not an external PASS                                        |
| V-04: the challenge PTY helper failed in an intermediate Apply run | Not executed | Not executed | WARNING            | No definitive cause; the complete case PASSES in reverify and the final gates |

There were no two independent judges; the required columns do not simulate votes.

### Historical CRITICAL: V-01 resolved

The preserved base specification requires in REQ-05, lines 132–139, that `→` over
`level: info` shows `level: warn`. Reproduction: open `/settings`, press Tab
three times to reach the toasts and press Right. The real router switches to integrations (`active_tab=4`) and keeps info (`level=0`). The independent assertion fails
with exit 101. This prevents accepting the full preservation of Settings even though
the behavior could predate this migration: the delta preserves
REQ-05 expressly. It was not corrected nor the spec changed in Verify.
The later Apply correction makes exactly this external source pass:
exit 0, tab=3 and level=1, without changing the assertions. V-01 remains open no more.

### Historical WARNING: V-02 resolved

`toggle_advances_toast_level_cycle` / `toggle_advances_toast_duration_cycle`
invoke `toggle_selected`; the host test navigates the tabs with Tab and does not test
Right inside the toasts. Thus, the 155 presentation tests can pass while the
contractual scenario fails. A regression through host input is required, in addition to
deciding how to make compatible the horizontal navigation of tabs and the cycle of pickers.
Apply added three host tests that cover the keyboard, the cycles, the navigation and the persistence;
the three pass in this reverify. V-02 remains open no more.

### WARNING: V-03 and acceptance limits

- Neither an external provider nor the real OS secure store were used. OVL-05-A requires
  explicitly a controlled service; OVL-05-B requires a controlled storage failure.
  Therefore these limits do not invalidate those technical scenarios,
  which were indeed executed, but they also do not certify external operational integration.
- Two ignored tests remain UNTESTED: the lock with the pre-rename binary and the OS
  secrets backend. The first does not correspond to a new overlay requirement;
  the second does not replace the Secure failure test without fallback, which did pass.
  No spec of this change requires using real external credentials to accept
  those scenarios. QA must declare its operational scope, not assume real success.
- The base spec enumerates a `--test settings` target that does not exist. The behavior
  is distributed among lib/state/PTY; those real targets were verified. That
  distribution did not excuse the missing keyboard scenario V-01, now covered.
- Without instrumental coverage nor a deterministic runner, exhaustive branch coverage
  is not declared. The conformance is limited to the executed scenarios.
- The cases called QA in the specs have technical automated evidence; the
  operator acceptance and `qa-report.md` remain pending. Do not archive.

### SUGGESTION

Keep a host regression test for each contractual Settings key;
do not replace the Right scenario with a Space test without an approved spec change.
Keep the existing security and PTY assertions.

## Integrity and next phase

The final staging hash matches the received one:
`180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
No add/reset/unstage/checkout/commit/push/merge/PR, data reset,
Archive nor state write was executed.

**Current handoff:** the orchestrator can delegate to **sdd-qa**, responsible for the
acceptance scenarios and for `qa-report.md`. Reverify concludes **PASS WITH WARNINGS**;
the initial FAIL is superseded by new evidence, not deleted. It does not authorize
Archive nor product acceptance. Staging verified without changes also at the
end of the focused reverify execution; the state remains under the orchestrator control.
