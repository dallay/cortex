# QA Report: huginn-overlay-system

## 1. Identity and result

- Change: `huginn-overlay-system`. Mode: **openspec**. Phase: **qa**.
- Date: **2026-10-20**. Executor: dedicated QA subagent.
- Verdict: **PASS WITH WARNINGS**, limited to the executed scenarios.
- QA gate for Archive: **AUTHORIZED WITH WARNINGS**; Archive is not executed.
- Execution: **fallback**. No QA runner/FSM configured in the
  repository was found; there is no deterministic enforcement nor an envelope of that runner.
  The `agent-tty` envelopes are preserved without reinterpreting their errors.

## 2. Sources and technical handoff

`proposal.md`, `design.md`, `tasks.md`, `apply-validation.md`,
`verify-report.md`, `state.yaml`, the four delta specs and the base
`.agents/sdd/specs/ui-settings/spec.md` were read. `.agents/AGENTS.md`,
`.agents/sdd/config.yaml` and the `agent-tty` and `dogfood-tui` skills were consulted.

Verify delivers **PASS WITH WARNINGS**, with V-01/V-02 resolved. Its reverify of
158 cases and the Apply CI 9/9, 477 s, Vitest 210/E2E 90 are antecedents, **not tests executed by QA**. This report does not count them as acceptance.

The local configuration does not add a more restrictive QA/Archive policy;
`testing.strict_tdd: false` does not exempt acceptance. The QA contract applies:
failures, open CRITICAL/P0/P1 and relevant BLOCKED/NOT TESTED items for acceptance
block Archive. No exception is needed nor invoked for documentation/config:
this change contains product and its real target was executed.

## 3. Target, environment and limits

- Repository: `/Users/acosta/Dev/dallay/cortex`, macOS/Unix.
- Target: Huginn inline TUI, real host/presentation with `tui-fixture` compiled
  by `test-driver`, Huginn binary and controlled connection/runtime services.
- Real capabilities: Cargo, PTY, termios, HTTP loopback, temporary SQLite,
  controlled clock/barriers, Ghostty renderer and `agent-tty` Chromium.
- No connection to an external provider nor to the real OS secure store.
  Only synthetic credentials; no data reset nor access to real secrets.
- The fixture tests the real UI with a local engine, not production inference.
  The controlled services run operations; they are not visual placeholders.
- The repository PTY observer models the inline protocol; it is not a complete
  emulator. The exploratory session adds an independent renderer as a contrast.
- No production/test code, `state.yaml` nor staging is modified.

All own evidence is under **Q/**:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-qa/`.
`reproduce.md` contains the commands and interactions; `create.json` identifies the
session `01M5CCGBF89CVHTSHK6S4MG08T`. The instrument working directory and home are
isolated in Q. `destroy.json` certifies the closing of the session.

## 4. Capability inventory

| Capability                                | Availability / selection                        | Reason                                                                    |
|------------------------------------------|---------------------------------------------------|---------------------------------------------------------------------------|
| PTY and inline TUI                         | available / selected                              | Input, focus, scrollback, resize and restoration are observable              |
| Host, barriers and injected clock         | available / selected                              | Receipt/watch races and boundaries without sleeps                         |
| Local HTTP/API services and SQLite        | available / selected                              | Real controlled login/model/logout, errors, revocation and recovery |
| Snapshots and PNG                          | available / selected                              | `doctor.json` certifies the renderer; own captures                         |
| Output asciicast                           | available / selected                              | Valid and reviewable recording of the exploration                            |
| WebM                                     | available announced / selected, failed execution | `recording.json`: `ok:false`, `EXPORT_ERROR`; video is not certified        |
| Web browser as a product                   | available / rejected                              | TUI target; Chromium only renders terminal evidence                    |
| Keyboard/basic accessibility             | available / selected                              | Tabs, arrows, Escape, mask and focus                                     |
| Screen reader/assistive technologies | unavailable                                       | No AT session; no accessibility conformance is declared                 |
| Narrow terminal/Unicode                | available / selected                              | 20x8, 1x1, 0x0 and per-cell clipping                                      |
| Full locale                              | rejected                                          | No new translation contract; Unicode does not certify all locales |
| Remote provider/OS secure store         | unavailable / rejected for execution             | No credentials nor permission to write external secrets               |
| Static inspection                      | available / selected as context                | Never used on its own to assign a PASS                                 |

## 5. Own execution

Commands from the root; the logs include the exact commands and the exit codes:

```sh
cargo test -p huginn-presentation --all-features -- --test-threads=1
cargo test -p huginn-application --test providers_contracts --test connections --test providers_turns -- --test-threads=1
cargo test -p huginn --test tui --test providers_flow --test config_compat -- --test-threads=1
cargo test -p huginn-runtime --test sessions_persistence --test stream_persistence --test providers_integration -- --test-threads=1
cargo test -p huginn --bin huginn --test cli -- --test-threads=1
```

- `Q/presentation.log`: exit 0, **158 PASS**, including 39 PTY, 35 state,
  13 workflows, 10 parser and 8 providers_ui.
- `Q/services.log`: exits 0; application **50**, app **19**, runtime **14** PASS.
- `Q/settings-persistence.log`: exit 0; bin **15**, CLI **7** PASS.
- Total of cases reported by those runners: **263 PASS, 0 FAIL, 0 ignored**.
  Empty targets are not counted as evidence. The two opt-ins were not selected.
- `Q/independent-observations.log`: **10 PASS** of new checks over the
  snapshots/recording of the exploratory session, script `check-evidence.py`.
  They are not added to the 263 Cargo cases nor do they represent exhaustive coverage.

### Acceptance scenario matrix

`P` = `Q/presentation.log`, `S` = `Q/services.log`, `C` =
`Q/settings-persistence.log`. The test names identify evidence executed
again by QA, not results copied from Verify.

| Scenario / requirement                                                           | Result  | Own evidence / observation                                                                                                                                                                                    |
|---------------------------------------------------------------------------------|------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Happy path Help and focus return, OVL-01/02-A                                  | PASS       | P: `declarative_help_opens_real_host_overlay_without_notification`; session `stream-help.json` opens Help while Busy                                                                                             |
| Each surface preserves payload/callback/continuation                          | PASS       | P workflows: `every_typed_surface_keeps_payload_secret_callback_and_continuation_operation`                                                                                                                       |
| Plugin request rejection, Confirm without authority and host-only Approval         | PASS       | P: `result_rejection_preserves_current_payload_focus_and_conversation`, `approval_preempts_secret_workflow_and_rejects_all_ordinary_requests`                                                                     |
| Approval preempts Settings and does not restore it, SET-12/OVL-03                      | PASS       | P state: `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`; stale lease/serialization also executed                                                                                   |
| Close, cancellation, lost sender, waiter drop and shutdown deny            | PASS       | P state: `approval_sender_loss_is_denial_and_late_input_has_no_authority`, `dropping_approval_waiter_retires_authority_and_sender`, `approval_cancellation_wins_ready_answer_and_shutdown_wakes_serial_waiters`   |
| Paste/stale challenge does not approve; fresh input does                               | PASS       | P PTY `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can`; exploration `approval-after-paste.json`: DENIED; `approval-approved-screen.json`: APPROVED only with a newly typed challenge              |
| Fragmented paste, Ctrl+C and lease change, OVL-04-A                           | PASS       | P PTY `ctrl_c_cancels_incomplete_secret_paste_and_discards_its_remaining_bytes`, `closing_secret_form_discards_fragmented_paste_and_accepts_fresh_input`; parser 10/10                                            |
| Sentinels outside the terminal, conversation, scrollback, JSON and files         | PASS       | P PTY streaming/secret; S `ui_two_connections_select_switch_pin_revoke_and_persist_without_secret_leaks`; own snapshots, PNG and asciicast without the exploration sentinel                                         |
| Revoke/re-register the same name and late result                            | PASS       | P workflows `revoked_origin_and_reregistered_name_cannot_mutate_replacement`, `late_result_after_cancel_does_not_replace_a_new_form`                                                                              |
| UI during streaming does not enter conversation/scrollback                        | PASS       | P PTY `overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback`, conversational positive control and reconstructed scrollback; own session Help/Login keeps Busy                                     |
| Repeating commands after 2001 ms creates no prompts nor conversation                   | PASS       | P providers_ui `repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation`, fake clock                                                                                                             |
| Same text in different categories, EVT-01-B                                   | PASS       | P state `explicit_ui_categories_do_not_depend_on_message_text`                                                                                                                                                    |
| Legitimate messages, tail and reprojection preserved, EVT-02                     | PASS       | P state `resume_and_synchronize_commit_messages_once_not_mutable_tail`, tool error; S stream_persistence/sessions_persistence                                                                                     |
| FIFO 256/257, session recovered >256                                            | PASS       | P lib FIFO and state reprojection; S `reopened_store_recovers_all_events_beyond_the_ui_retention_limit`                                                                                                             |
| Dedup 1999/2000 ms, interleaving/collision, limits and expiration                | PASS       | P event tests of the half-open window, full equality, bytes/oversize, purge and nonce exhaustion; new errors visible                                                                                        |
| Settings arrows in toasts and exit with Tab/BackTab, SET-03/05                  | PASS       | P three host regressions of the fix; exploration Tab×3 Right shows `level: warn`, Down Right shows `duration: 5s`; it stays in toasts                                                                               |
| Settings navigation/copy/discard without write nor cancellation                    | PASS       | P shell/catalog, stale input and `settings_cycle_arrows_only_persist_displayed_values_on_enter`; the exploration discards before Apply                                                                                |
| Joint Apply, one write, disk full error and retry                          | PASS       | P `settings_failed_apply_preserves_copy_and_error_and_can_retry`, `settings_cycle_arrows_only_persist_displayed_values_on_enter`; the saved values are recovered when reopening                                        |
| Pending Apply, Esc/replacement, success/error receipt with a new Settings/Approval | PASS       | P `late_settings_receipt_preserves_new_working_copy_and_focus`, `replaced_settings_writes_remain_serialized_in_submission_order`, `settings_started_write_finishes_after_shutdown_without_reopening_or_notifying` |
| External/unknown TOML, defaults and shell without live application                      | PASS       | C `config::tests::ui_defaults_fill_absent_sections_and_partial_nested_tables`, four `ui_settings::tests`; P Settings shell and projected config                                                                  |
| Settings excluded from line-mode, catalog and Add provider without I/O                 | PASS       | S tui `non_tty_default_fails_clearly_and_explicit_line_recovery_works` sends `/settings`; P catalog/Add provider                                                                                                 |
| Functional login/providers/model/logout without restart, OVL-05                    | PASS       | P providers_ui and S providers_flow 6/6: save/select/update/revoke, Secure failed without fallback, cancellation/OAuth; no remote provider                                                                             |
| Next turn selection, current turn pin and revocation                     | PASS       | S providers_turns/providers_integration; local HTTP demonstrates that the selection does not change the admitted turn                                                                                                           |
| Initial watch, late/duplicate A, retired close and new source               | PASS       | P `watch_attachment_initial_read_coalesces_concurrent_updates_without_gap`, `connection_attachment_rejects_stale_sources_revisions_and_retired_close`; S staged commit/teardown/faults                            |
| Bar always visible; no false Connected availability                        | PASS       | P status_bar and the PTY overlay matrix; the session shows fixture/alpha and then Bar-Login Ephemeral, Busy/Approval/Idle without Connected                                                                                 |
| Normal/20x8 resize, min/1x1 and zero-size                                         | PASS       | P PTY matrix and `approval_resize_below_minimum_disarms_until_a_fresh_draw`; lib `zero_area_draw_does_not_arm_approval_then_viable_draw_can_arm`; exploration Login 100x30→20x8→100x30                              |
| Complete scrollable preview and per-cell Unicode                               | PASS       | P Approval preview/PgUp/PgDn and status_bar clipping; Unicode width is not inferred only from the textual snapshot                                                                                                       |
| Normal restore, cancellation, drop/unload and new generation                     | PASS       | P PTY termios/raw/bracketed-paste/inline; the exploration compares the complete `stty -g` before/after and shows TERMINAL_RESTORED                                                                                        |
| Keyboard exploration and focus after Escape                                          | PASS       | `stream-login.json` sends the input too early and does not open a form; after an observable composer, `stream-login-retry.json` opens Login; the previous input does not cross the lease                                          |
| WebM export                                                                | FAIL       | `recording.json`: `ok:false`, `EXPORT_ERROR`; instrument failure, not a functional scenario nor a permission concession                                                                                              |
| Accessibility with a screen reader                                            | NOT TESTED | No AT session nor new conformance contract; the tested keyboard does not certify AT                                                                                                                                   |
| Full locale internationalization                                       | NOT TESTED | There is no new translation requirement; Unicode/clipping were executed                                                                                                                                           |
| Browser as a product surface                                          | NOT TESTED | Not applicable: terminal target, not a web dashboard                                                                                                                                                                   |
| Real external provider / OS secure store                                      | NOT TESTED | Without credentials/permission; no external calls nor writes were attempted                                                                                                                                        |
| Two inherited opt-ins                                                            | NOT TESTED | Pre-rename binary not supplied and OS backend not authorized; not included in the selected runners                                                                                                              |

The host and services tests are focused acceptance probes. The exploration
adds new flows of its own; QA does not limit itself to renaming the Apply/Verify reports.
Neither human user acceptance nor acceptance of the harness itself is claimed.

## 6. Visual evidence, instrument incidents and untested scope

- PNG: the exact paths in `settings-screenshot.json`, `login-screenshot.json`,
  `small-screenshot.json` and `approval-screenshot.json` of Q. The last one shows
  the state after a denied paste, not a pending Approval.
- Valid recording: `asciicast.json` references
  `tty/sessions/01M5CCGBF89CVHTSHK6S4MG08T/artifacts/recording-199-asciicast.cast`.
  It captures 169 output events and two resizes; it is not presented as WebM.
- `doctor.json` announced WebM as available; the export ended with `EXPORT_ERROR`.
  The instrument FAIL is preserved. PNG/asciicast and PTY keep providing
  observable evidence; that envelope is not turned into a PASS.
- First launch: the DSR response from the instrument was missing, cursor position error;
  the terminal was restored and it was repeated with an explicit ESC[9;1R response.
  No automatic compatibility with every emulator is declared. The initial captures
  keep shell/previous error text outside the inline viewport.
- Two batches ended up `interrupted` by an external timeout: startup without DSR and a
  command sent during quarantine after Escape. They are not counted as PASS;
  the later executions are identified separately, without deleting the originals.
- The sentinel is synthetic only. The agent-tty `events.jsonl` telemetry
  contains input entries; it is not a product log nor proof of the instrument redaction.
  Snapshots/PNG/asciicast are evidence of clean output.
- Broad CI, audit, fmt/clippy and the dashboard E2E are not repeated in QA; it corresponds
  to the current technical handoff. No new full CI is declared.
- To extend operational acceptance: provide test credentials and explicit
  write permission in Keychain/Secret Service; run the opt-ins and the
  remote flow in an isolated environment. For the legacy lock: supply
  `CORTEX_AGENT_LEGACY_BIN`. For AT/locales: define the target, the contract and the session.

## 7. Findings and risks

| ID    | Severity | State                       | Description / follow-up                                                                                         |
|-------|-----------|------------------------------|-------------------------------------------------------------------------------------------------------------------|
| QA-01 | P3        | OPEN, non-blocking          | WebM failed despite doctor available; use the valid PNG/asciicast and diagnose the exporter before demanding video   |
| QA-02 | P3        | OPEN, non-blocking          | agent-tty did not answer the initial DSR automatically; the reproduction requires the documented explicit response      |
| QA-03 | P2        | NOT TESTED, operational risk | Real external provider, OS secure store and two opt-ins not executed; it does not certify production nor legacy interoperability |

No new functional defects nor open CRITICAL/P0/P1 findings were identified
in the executions. The historical V-01 is not reopened: Right was observed
working in the session and the three host regressions pass again.
The cause of the historical failure of the challenge PTY helper is not established here;
the complete case passed in the 39 own PTY, without filters nor assertion changes.

## 8. Verdict, Archive policy and handoff

**PASS WITH WARNINGS. QA gate: authorized with warnings.**

The relevant specified acceptance was executed: OVL-05-A explicitly requires
a **controlled** service and OVL-05-B a **controlled** failure of the secure
storage. Both were tested with real local operations and controlled doubles.
No new requirement demands real external credentials, a pre-rename lock,
a screen reader or complete translations. Therefore the previous NOT TESTED items
are not pending contractual acceptance nor do they become an external PASS.
The normal barrier for relevant NOT TESTED remains intact; no
documentation/configuration exception is invented for this product change.

The export failure is supplementary tooling: it does not invalidate product
scenarios with PTY, snapshot, PNG and valid asciicast evidence. It stays visible
as a warning, neither hidden nor interpreted as approved video.

Archive must check both reports, the current state and the absence of new
CRITICAL/P0/P1/functional failures before acting. This report authorizes the QA
gate of the tested candidate, not deployment nor remote availability. The orchestrator
can advance to `sdd-archive` preserving these warnings. If the code changes,
the affected evidence must be reevaluated. `state.yaml` is not changed here.

Initial and final staging SHA-256:
`180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
Without add/stage/unstage/reset/checkout/commit/push/merge/PR, without data reset.
Only new repository file in this phase: this QA report.
