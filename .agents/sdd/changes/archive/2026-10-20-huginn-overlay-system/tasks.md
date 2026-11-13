# Tasks: Huginn overlay system

## Execution and review contract

Apply closure 2026-10-20: 21/21 tasks complete, including 6.2–6.3.
Final `just ci-local` PASS 9/9 (431 s); details, earlier failures and limits are in
`apply-validation.md` and `apply-progress.md`. Independent Verify/QA remain
pending; no earlier checkpoint replaces this new evidence.

Worktree state validated on 2026-10-19 against `HEAD=442e34b`: 1.1–1.3, 2.1–2.6 and 3.1–3.3 complete as implementation, with their own evidence in `apply-progress.md`. HEAD without these changes and historical checks do not accredit this implementation. Earlier staging preserved, including `event.rs`, `overlay.rs` and artifacts. Contract 2.1 allows bare Help/Settings and requires real workflows for the other surfaces; 2.2 connects openings, payloads, continuations, render and input to `UiState.overlays`, with no parallel active slots. 2.3–2.6 harden and cover those contracts; focused Clippy all-targets/test-driver passes after a small refactor of event.rs and lint cleanup in the change tests. Verify/QA and full CI not executed; no claim of product acceptance or PR readiness. The proposal,
four delta specs and design prevail over incompatible exploration suggestions. Only the
orchestrator modifies `state.yaml`; this document authorizes no commits, branches,
publication, merge or archive.

### Review load forecast

- Budget: 400 modified lines, additions plus deletions.
- Estimated load: high; approximately 2500–5500 lines, including consumers,
  tests, fixtures and documentation. Planning estimate, not a measured diff.
- Strategy: `size-exception`, one P0+P1 delivery in one PR, expressly authorized
  by the mission and the proposal. Rationale: cross-cutting migration with joint
  rollback of contracts and consumers, without imposing another delivery decision.
- Stacking would be advisable by size; it is **not adopted** against the authorized
  single delivery. The following packages are review units inside the PR,
  not new PRs and not mandatory commits.

Delivery decision pending: No; the authorization for a single P0+P1 PR stands.
The user authorization merged 2.2 into one mission, now implemented and
technically tested; it is not product acceptance and does not automatically start dependent tasks. The estimate remains
planning, not a measured diff.

Reviewable packages after rebaselining:

- A: events/categories/dedup (1.1–1.3), complete in the worktree with focused evidence; it does not imply changes already integrated in HEAD.
- B1: plugin contract (2.1), complete in the worktree with tests for invocation, metadata, continuation and rejection without replacement.
- B2: host authority, adapters, lifecycle and input/render (2.2), complete in the
  worktree with its own host and PTY tests; pending orchestrator review,
  without replacing later verify/QA.
- B3: hardening of workflow/Approval/input and providers (2.3–2.6), complete
  as implementation and focused suites; it does not replace verify/QA.
- C–F: Settings, connection/bar and legacy removal; they depend on the shared authority.

No separate PRs or commits were created; the packages are review points inside
the authorized single delivery.

Reviewable packages, dependencies and indicative effort:

- A: events/categories/dedup (1.1–1.3), 2–3 sessions.
- B: contracts, integrated authority, adapters and overlay flows (2.1–2.6), depends on A, original estimate 5–7 sessions; review after rebaselining.
- C: Settings with asynchronous persistence (3.1–3.3), depends on B, 2–3 sessions.
- D: observable selection and lifecycle (4.1–4.4), depends on A; integration with B/C,
  3–5 sessions. New contracts and all their implementers change together.
- E: real bar/viewport (5.1–5.2), depends on B/C/D, 2–3 sessions.
- F: legacy removal and verifiable closure (6.1–6.3), depends on A–E, 1–2 sessions.

Each session covers one logical unit; if it exceeds the session, subdivide while keeping
tests and behavior together, without changing delivery. Safe pause points
are task ends with adapted consumers and focused verification green;
RED is a local working state, never a deliberately non-compilable deliverable
checkpoint. Temporary strangler-fig adapters have a single owner per
transition: never double input/render, emission or write. Delivery rollback:
revert the whole PR, with no destructive migrations and no data reset.

### Testing mode and evidence

The inherited configuration declares `testing.strict_tdd: false`; there is no explicit
activation of strict TDD in this mission. No global RED is imposed for every new
type. **Every incorrect behavior requires a regression that fails before the fix**.
In each task: prepare the test/fixture, record the regression RED, implement,
check GREEN and refactor while checking again. Land those tests
with the task, never defer them to a generic final block.

For every RED record the exact command, filter, case, exit code and sanitized
failing assertion; a compilation failure caused by a missing API does not prove the
regression. If it requires a new seam, introduce it compatibly and demonstrate the
behavioral failure before the fix. GREEN records the repetition of the same command and its
output; no omitted case is marked approved. Barriers/channels and fake clock,
no sleeps to demonstrate order, dedup or concurrency; timeouts only as a watchdog.

Runners verified by reading manifests, targets and `justfile`, **not executed**.
In tasks, `P`, `A`, `R`, `U`, `X`, `H`, `C` abbreviate respectively the packages
`huginn-presentation`, `huginn-application`, `huginn-runtime`, `huginn-plugin-api`,
`huginn-reference-extension`, `huginn`, `huginn-core`. `T(P; state,workflows)`
means `cargo test -p huginn-presentation --test state --test workflows`;
`L(P)` means `cargo test -p huginn-presentation --lib`. Always expand the
aliases in reports. Add an existing filter for RED and repeat the
full target for GREEN.

PTY means `cargo test -p huginn-presentation --features test-driver --test pty`;
parser means the same command with `--test input_parser`. Both require Unix;
`pty` contains the tests, `tui-fixture` is the auxiliary binary, not their runner.
`--all-features` **does** enable `test-driver`; check the number of tests executed,
do not confuse fixture compilation or zero tests with PTY evidence.

Abbreviated paths: `P/`, `A/`, `R/`, `U/`, `X/`, `C/` are
`crates/huginn/{presentation,application,runtime,plugin-api,reference-extension,core}/`;
`H/` is `apps/huginn/`. They are concrete paths relative to the workspace.

## Phase 1 — Categories, conversation and scoped errors

- [x] **1.1 Separate explicit surfaces** — `P/src/{event,lib,state}.rs`,
  `H/src/main.rs`, `P/tests/support/fixture.rs`, `P/tests/{state,extensions}.rs`.
  Dependency: none. EVT-01/02. Replace the `notice` port with object-safe methods
  for five categories; adapt every implementer/caller of the port in
  this unit. Rename `committed` to `conversation` with temporary internal
  compatibility only where the migration needs it. Tests: same text in different
  categories, Session/Workspace outside conversation, full tool error,
  streaming, idempotent reprojection and legitimate identical messages.
  Acceptance/verification: only Conversation feeds scrollback; `T(P; state,extensions)`.
  Risk: losing renderers or introducing classification by text; forbid both.
  Apply evidence 2026-10-19: the Interaction `notice` port was replaced with object-safe methods for five categories; host/app/fixture callers migrated and `committed` renamed to `conversation`. Session/Workspace are projected as Status; ToolFinished keeps its payload intact in conversation/stream; engine errors are projected as diagnostics without heuristic text. Tests `state`, `extensions`, `workflows`, `providers_ui`, `apps/huginn providers_flow` pass. RED/GREEN was observed in the history of a notice contaminating conversation; reprojection and legitimate repetition are tested. Full result of the focused suites is below in apply-progress.

- [x] **1.2 Implement retention and dedup** — `P/src/{event,state}.rs`, unit
  tests of `event.rs` and `P/tests/state.rs`. Depends on 1.1. EVT-03/04.
  Test hash plus equality per operation, forced collision, t=0/1.999/2s,
  re-anchoring only on visible emission and A/B interleaving. Test FIFO 256/257,
  dedup 256 entries/256KiB, exactly 4096 bytes and a larger message not cached
  and not truncated for comparison; expired purge and eviction of the oldest emission.
  Acceptance/verification: saturation never hides a new error, no secret
  retained; `L(P)` and `T(P; state)`, fake monotonic clock without sleeps.
  Risk: dedup by hash alone or window extended by suppression.
  Reconciliation (continuation 2026-10-19): functional criteria EVT-03/04 of 1.2 covered by unit and UI producer tests; `T(P; state)` ran and passes (17), but the task explicitly required completing 1.3 before this integrated acceptance; **1.2 is [x]** after separating its functional criteria from the 1.3 integration criteria; required acceptance suites executed in this worktree (27 lib and 17 state). Scope propagation and authoritative recovery are criteria of 1.3 itself, not criteria added to 1.2. The deduplicator keeps exact equality encrypted with AES-256-GCM with a random key per instance, counter nonce and accounted encrypted size (message + 28 bytes); it keeps hash plus equality against collisions, the 256-entry/256 KiB limit, window and cleanup. The oversized message (4097 bytes) passes without caching or truncation, exactly 4096 bytes are compared, there is a test for the absence of the secret in retained state, and saturation/eviction. RED observed in this continuation:
  `cargo test -p huginn-presentation --lib event::tests::dedup_nonce_exhaustion_emits_without_panicking_or_caching` failed because of a panic when the counter was exhausted; GREEN after changing exhaustion to a visible emission without caching, clearing entries to prevent nonce reuse. Additional RED: `cargo test -p huginn-presentation --lib event::tests::dedup_enforces_entry_byte_and_oversize_bounds_without_truncation` showed that the FIFO test wrongly assumed the second-oldest entry was still retained after reinserting the oldest, which evicts another entry again; the test was corrected to check what is required: oldest evicted and reinsertion visible. Most recent exact checks: `cargo test -p huginn-presentation --lib` (26 passed, before the last FIFO assertion; the focused bounds test passed later), `cargo test -p huginn-presentation --lib event::tests::dedup_enforces_entry_byte_and_oversize_bounds_without_truncation` (1 passed),
  `cargo test -p huginn-presentation --lib event::tests::dedup_nonce_exhaustion_emits_without_panicking_or_caching` (1 passed), `cargo test -p huginn-presentation --test state` (17 passed), `cargo test -p huginn-application --test contracts` (4 passed). Not all workspace suites were re-run, nor the full `L(P)`. `cargo fmt --all -- --check` and `git diff --check` pending at the close of this continuation. Propagation and verification of 1.3 remain a prerequisite; do not claim 1.2 acceptance before that gate.
- [x] **1.3 Complete scopes and authoritative preservation** — `P/src/{lib,state}.rs`,
  `H/src/main.rs`, `P/tests/support/fixture.rs`, `A/src/loop_engine.rs`,
  `A/tests/contracts.rs`, `R/tests/{coding_workflow,stream_persistence,sessions_persistence}.rs`.
  Depends on 1.2. EVT-01–04. `begin` returns OperationId; app/fixture fallback and
  Event::Error share the turn id. Continuations inherit theirs; new
  invocations and independent tasks do not adopt the turn. Finish after
  all producers/results; filter once before choosing the UI destination.
  Test engine error plus fallback without a double surface and operation cleanup;
  a session with more than 256 events fully recoverable, without changing `record` or Session.events.
  Acceptance/verification: `T(A; contracts)`, `T(R; coding_workflow,stream_persistence,sessions_persistence)`
  and `T(P; state)`. Risk: releasing scope before the fallback or capping persistence.
  Apply evidence 2026-10-19: the engine-error/fallback flow in `main.rs` goes through the same `ScopedEventSink` before `finish_operation`; `engine_and_host_fallback_errors_share_one_operation_diagnostic` verifies a single surface. The runtime uses a borrowed sink (`&dyn EventSink`) across loop, turn and sequential tool execution awaits; it does not clone it or hand it to `'static` tasks, so no independent UI producer can implicitly inherit scope in the current scope. Test `operation_scope_survives_tool_continuation_and_is_distinct_per_run` captures `emit_for` with the real `OperationId` of each emission: tool, continuation response and turn end keep the same ID; a second `run` call records another ID. `explicit_event_operation_scope_survives_interleaved_producers` verifies dedup per operation in the production projection; an emission after finish is rejected. Session persistence keeps 300 events; `record`/`Session.events` are not capped.
  Required focused suites executed: application contracts 4, runtime coding_workflow 28, stream_persistence 4, sessions_persistence 2, presentation state 18, all pass. Therefore 1.3 is complete.

## Phase 2 — One overlay owner, effects and focus

- [x] **2.1 Add complete plugin contracts** — `U/src/v1.rs`,
  `U/src/v1/workflow.rs`, `U/tests/{contract,workflows}.rs`, `X/tests/smoke.rs`,
  `P/src/state.rs` and `P/tests/workflows.rs`. Depends on 1.1. OVL-02/05.
  Add OverlayKind without Approval, `CommandResult::Overlay`, optional metadata and
  exhaustive validators in Command::run and Workflow::execute; update both
  dispatch paths and smoke together. Bare Help/Settings allowed; the others require
  a real workflow, builder only for Provider/Login/ModelPicker/Confirm. Keep Notice
  sanitized and Dialog compatible, and reject dialog lookup from an async workflow.
  Test allowlist, rejection without replacement and exact registration binding.
  Acceptance/verification: `T(U; contract,workflows)`, `T(X; smoke)`, `T(P; workflows)`.
  Risk: losing payload/callback or turning Confirm into a host approval. Evidence 2026-10-19: exhaustive validators and the builder are already present and characterized without a fictitious RED; new tests cover the six surfaces by invocation, four workflow types, continuation metadata (including None), definition/callback/fields, revocation and re-registration without using a substitute, sanitized/scoped Notice and rejection of async Dialog. Behavioral RED in presentation by Overlay category instead of Notification and async message without explicit rejection; GREEN after fixing both dispatch paths. Rejections preserve the current payload, selection and workflow_focus, and the conversation. Suites PASS: plugin-api contract 11/workflows 9, reference-extension smoke 1, presentation workflows 6/extensions 2. Real opening and single authority remain 2.2 acceptance, not a circular precondition of this contract.

- [x] **2.2 — Vertical migration of overlay authority (coordinated mission; do not mark
  complete until the whole checklist and host-level acceptance are satisfied)** —
  `P/src/{overlay,lib,state,workflow,contributions,approval,settings,terminal,input}.rs`,
  unit tests of `overlay.rs`, `P/tests/{state,workflows,extensions,input_parser,pty}.rs`,
  `P/tests/support/{fixture,pty}.rs`. Depends on 2.1; OVL-01–05, EVT-01, SET-12.
  Sub-steps a/b/c are internal dependency-ordered phases of this single task, not
  separately authorizable or acceptable tasks. Do not stop or declare success after
  creating the authority: both consumers and payloads must migrate in the same cut.

  Internal sub-checklist — do not mark a partial step as a complete task:
    - [x] **a. Authority and transitions:** create `Overlay`, `ActiveOverlay`, origin,
      operation, ticket, revision and lease; `UiState` single active owner. Open/close
      every surface for real with the exact payload; stale revision/lease and
      rejection without altering focus/copy. Safe snapshots, without exposing secrets.
    - [x] **b. Adapters and lifecycle:** migrate Dialog/Handle, HostWorkflow/Pending,
      host-only Approval with oneshot/epoch and preemption, SettingsDialog and widgets as
      payloads; preserve callbacks, continuation, exact registration, workflow lifecycle
      and receipts. Cancellation/revocation/abort and Drop outside the mutex. Settings keeps
      public DTOs/imports and semantics; no parallel slot and no accidental persistence.
    - [x] **c. Render/input and host-level acceptance:** snapshot, render and input consume
      the same authority; no duplicated active ownership. Approval host-only, one-shot,
      preempts and rejects ordinary openings. Keep stale revision/lease, paste
      ownership and consumed close. No toast layer; `conversation` keeps its semantics.
      Effects/callbacks/drop outside the mutex. Adapt fixture/PTY and every consumer.

  RED/GREEN tests inside the same mission for valid/rejected replacement, exact
  payload, stale ticket/continuation, secrets in snapshot, reentrant callback/Drop,
  local cancellation, Approval/preemption, workflow lifecycle/receipt, Settings without
  premature write, and render/input source equivalence. Acceptance requires
  together `L(P)`, `T(P; state,workflows,extensions)`, parser and PTY with tests
  executed, plus host-level acceptance: every surface opens, updates and
  closes through the shared authority; payloads are preserved; no alternative
  writers, renderers or input readers remain active. A green sub-step does not accept 2.2.
  Risk: incomplete migration, double authority or lost functionality.

  Apply evidence 2026-10-19: `UiState.overlays: OverlayMachine<Payload>` is the
  single owner; parallel slots and leases removed. Real Help, Settings with copy and
  serialized receipts, Dialog with the exact handle, workflows with secrets/callbacks/
  continuations and host-only Approval keep their payloads. Tickets/leases are
  revalidated before effects; retired resources, cancel/abort, callbacks, save and
  renderers are processed outside the UI mutex. Behavioral RED observed for
  Help, replacement by Settings, aborted waiter, renderer under mutex, exhaustion
  on close and operation assignment from a stale ticket; GREEN of the same
  command for each case. Final suites without case filters:
  `cargo test -p huginn-presentation --features test-driver -- --test-threads=1`
  (132 tests: 31 lib, 24 state, 12 workflows, 2 extensions, 9 parser, 35 PTY,
  19 additional), and `cargo test -p huginn --test providers_flow` (6), exit 0.
  Format and diff check pass. Optional Clippy fails exclusively in two lints
  of the earlier `event.rs`, documented in progress; CI/verify/QA not executed.

- [x] **2.3 Harden workflow/dialog and residual exhaustive results** —
  `P/src/{contributions,workflow,state,terminal}.rs`,
  `P/tests/{workflows,extensions,completion}.rs`. Depends on the full acceptance of 2.2; OVL-01/02/04/05,
  EVT-01. Do not migrate ownership again or create another router: cover `prepare_submission`,
  captured data, self-executable empty fields, Zeroizing secrets up to 4096 bytes,
  `FormInput.secrets`, constant mask and exhaustive routing of results/continuations.
  RED/GREEN to revoke and re-register the same name and for a late completion. Verify
  `T(P; workflows,extensions,completion)`. Risk: callback bound to a substitute registration.
  Evidence 2026-10-19: revocation/re-registration and late result regressions from 2.2
  are reused; new characterization checks UTF-8/4096 bytes, constant mask,
  wipe/sanitize, separate FormInput, immutable capture and single submission. Behavior
  already implemented: no artificial RED and no second router. Full suites 13/2/9 PASS.

- [x] **2.4 Harden host-only, fail-closed Approval** —
  `P/src/{approval,overlay,state,terminal}.rs`, `P/tests/{state,pty}.rs` and
  `P/tests/support/fixture.rs`. Depends on the full acceptance of 2.2 and 2.3; OVL-03/04, SET-12.
  Keep a single sender, serialized requests, epoch/challenge/armed and resolution
  exactly once; do not rebuild or relocate Approval. RED/GREEN for stale lease/epoch,
  late callback, priority cancellation, Esc/Ctrl+C, closed channel, shutdown,
  driver drop and acceptance only with fresh input after draw. Verify `T(P; state)` and
  PTY with synchronous restoration. Risk: any improper acceptance blocks.
  Evidence 2026-10-19: deterministic characterization through polling of real futures
  demonstrates serialization, an earlier lease being inoperative, priority cancellation with
  the answer already ready, and shutdown of an active/queued waiter. Tests for abort,
  epoch/challenge, preemption and PTY arming/fresh input/paste/restoration are kept. Without
  duplicating Approval or inventing RED for existing guarantees. State 26 and serial PTY 35 PASS.

- [x] **2.5 Harden input/paste without a second ownership** —
  `P/src/{input,terminal,overlay}.rs`, `P/tests/{input_parser,pty}.rs` and fixture.
  Depends on the full acceptance of 2.2 and 2.4; OVL-04. Keep terminal generation, draining until a quiet
  tick, paste owner until the delimiter, local cancellation by Ctrl+C and rejection/
  zeroization of stale paste. RED/GREEN for a split secret sentinel and
  unknown/textual and non-textual keys consumed on all surfaces; never
  deliver fragments to the editor/completion or accept Approval. Verify parser and
  PTY. Risk: leak when focus changes or arming with earlier input.
  Evidence 2026-10-19: real RED regression for an unknown CSI suffix longer than 64 bytes
  released as text; GREEN after keeping the quarantine until the final byte, even with
  a new focus. The existing parser/ownership is kept; reused tests cover a
  split sentinel, local Ctrl+C, zeroization and a new generation. Typed
  surfaces were extended with unknown/non-textual/release events consumed.
  Parser 10, workflows 13, serial PTY 35 PASS.

- [x] **2.6 Preserve functional Provider flows on the integrated authority** —
  `P/src/{providers,overlay,terminal}.rs`, `P/tests/providers_ui.rs` and
  `H/tests/providers_flow.rs`. Mandatorily depends on 2.2 accepted and on 2.4–2.5;
  OVL-05, EVT-01. Label loading and each continuation of login/connection_form,
  providers, model_form and logout Confirm without replacing payloads or the real workflow.
  RED/GREEN for host opening, recover/list/status/save/select_model/revoke,
  invalid endpoint/model, explicit Secure/Ephemeral choice, failure without fallback,
  unsupported OAuth and recoverable cancellation. Verify `T(P; providers_ui)` and
  `T(H; providers_flow)`; zero placeholders and results outside conversation.
  Risk: functional Provider before having the integrated authority or losing the continuation.
  Evidence 2026-10-19: real RED Workflow != Login when opening the existing flow;
  GREEN by labeling loading and Login/Provider/ModelPicker/Confirm continuations without
  replacing callbacks or data. Real save/select/update/revoke tests extended
  with labels, Confirm from Providers/recoverable cancellation and invalid
  endpoint/model before writing credentials. Reused secure failure without fallback,
  explicit Ephemeral, OAuth and persistence/revocation. Providers UI 7 and app flow 6 PASS.

## Phase 3 — Settings, copy and receipts

- [x] **3.1 Integrate Settings without changing its shell** —
  `P/src/{settings,overlay,state,terminal,contributions}.rs`,
  `P/tests/{completion,plugins,state}.rs`, unit tests of `settings.rs`.
  Depends on 2.6 and uses exclusively the authority accepted in 2.2. SET-01–06/08/12–16. Keep public DTOs/imports in settings.rs,
  five tabs, rows/order/markers, navigation and copy without persisting on move;
  `Settings` is a payload of the shared authority, never a `SettingsDialog` slot.
  Esc consumes the key, does not cancel the turn; before Apply it does not write. `/settings` and
  whitespace do not send a prompt; model-independent catalog; Add provider
  keeps the close/Notification warning without touching providers and without saving.
  Test authorized replacement and rejection while keeping copy/lease, without restoration.
  Acceptance/verification: `L(P)`, `T(P; completion,plugins,state)`.
  Risk: confusing selection with saved value or activating onboarding/live theme.
  Evidence 2026-10-19: shell integrated in 2.2 preserved; new host
  characterization of the catalog with/without model, whitespace, navigation across five tabs, selection
  different from the marker, discarded copy and Add provider without persistence or Provider
  opening. The Settings→Dialog replacement and rejection/preemption from 2.2/2.4 are reused.
  Lib 31, completion 9, plugins 2 and state 27 PASS at this task checkpoint.

- [x] **3.2 Coordinate Apply outside the lock** — `P/src/{settings,overlay,state}.rs`,
  `P/tests/state.rs` and unit tests of `settings.rs`. Depends on 3.1. SET-07/08/12,
  OVL-03/04. A blocking worker invokes the injected closure; a separate coordinator
  serializes saves and receipts keep only the required copy/result.
  Earlier RED regressions: reentrant closure does not block input, disk full keeps
  copy/error only in the body and repeated Enter does not duplicate. Barriers for the order of
  two saves, failure/retry, Esc/replacement/Approval after writing starts,
  success/error receipt with another Settings open, shutdown and late completion.
  Acceptance/verification: `L(P)`, `T(P; state)`; success updates the snapshot, closes and
  notifies only the current overlay; a late one never reopens or alters a new copy/Approval;
  a late failure does not publish failed values. Risk: promised rollback or double write.
  Evidence 2026-10-19: real coordinator already integrated in 2.2, without duplicating it. Tests
  of existing barriers cover reentrant closure, single Enter, error/retry,
  order between two saves and receipt with another Settings/Approval. Esc is extended with
  an active turn during a write and shutdown is added with a late success/error without
  reopening/warning or a second write. Lib 31 and state 28 PASS; no fictitious RED.

- [x] **3.3 Preserve persistence/config and line mode** —
  `H/src/{config,ui_settings,main,terminal}.rs`, `H/tests/{config_compat,cli,tui}.rs`,
  `H/config.example.toml`; keep/restore the helper unit tests.
  Depends on 3.2. SET-09/10/11/15 and 07. Test complete/partial/absent TOML,
  internal unknowns, comments/order and bytes outside ui, joint Apply;
  line mode without catalog/Settings overlay. Do not move UiConfig or modify the schema.
  Acceptance/verification: `T(H; config_compat,cli,tui)` and `cargo test -p huginn --bin huginn`.
  Risk: an old Settings-modal test not executed is taken as evidence;
  obtain own evidence and do not activate live preference consumers.
  Evidence 2026-10-19: RED regression Config::load with ui absent loaded empty
  strings/false indicators instead of the documented defaults; GREEN after unifying
  UiConfig::defaults/Default and the defaults of nested DTOs, without moving types/schema.
  Absent/partial/partially nested tested through Config::load; helper extended with
  full round-trip, nested unknowns and stable external bytes. Line mode
  receives /settings without overlay or TUI sequences. Bin 15, config_compat 5, cli 7 and
  serial tui 7 PASS, without borrowed evidence from settings-modal or live consumers.

## Phase 4 — Observable confirmed selection and lifecycle

- [x] **4.1 Publish a versioned snapshot from the router** — `C/src/{lib,connections,providers}.rs`,
  `A/src/{providers,connections}.rs`, `A/tests/{providers_contracts,connections}.rs`,
  `A/examples/providers_headless.rs`. Depends on 1.3. BAR-02/03.
  Add ConnectionChanged and a safe serializable snapshot, source UUID per
  activation, increasing revision, phase and optional selection; watch with subscription
  and send_replace under the same mutex. Extend both ports/implementers/wrappers,
  StageSelection and exact publish_selection, keep direct Select Unmanaged and
  Legacy distinguished. Test absence/candidate/confirmation and serde without secrets.
  Acceptance/verification: `T(A; providers_contracts,connections)` and
  `cargo check -p huginn-application --examples`. Risk: announcing Connected by selection.

- [x] **4.2 Generalize publication guards to all routes** —
  `A/src/connections.rs`, `A/tests/connections.rs::PublicationFault`,
  `A/tests/providers_turns.rs`, `R/tests/providers_integration.rs`.
  Depends on 4.1. BAR-02/03, OVL-05. StageSelection installs the candidate without announcing
  Selected; confirm only after persistence/publication and exact comparison
  under a serialized operation. ModelPublication covers select_locked, model change,
  restore/recover and explicit resolve; error/drop/cancel clears fail-closed.
  The wrapper forwards subscribe/publish and intercepts StageSelection in addition to Select.
  Test with barriers for failure before/after publishing, Err/Pending after an
  effect, drop/cancel on each route and invalidated comparison; MetadataMutation and
  invalidate_all do not take an async mutex from Drop. Acceptance/verification:
  `T(A; connections,providers_turns)`, `T(R; providers_integration)`; never a fictitious B.
  High risk: inoperative fault injection or guard disarmed before confirmation.

- [x] **4.3 Close observability in all teardowns** —
  `A/src/{providers,connections}.rs`, `A/tests/{connections,providers_contracts}.rs`,
  `C/tests/{kernel_lifecycle,registration_ownership}.rs`.
  Depends on 4.2. BAR-02/03. RegistrationLease, revocation/Replace of the selected one,
  cancellation/unload adapter, ManagerLease and CoreLease publish absence with
  Revoked/Unavailable before teardown. Check the owner: a retired manager does not erase
  a new generation; revoking another connection does not erase the selected one.
  Test each hook and reactivation of a new source with barriers, including teardown
  concurrent with publication. Acceptance/verification: `T(A; connections,providers_contracts)`
  and `T(C; kernel_lifecycle,registration_ownership)`. Risk: publication by a stale owner.

- [x] **4.4 Connect watch without races and migrate Event consumers** —
  `H/src/{main,terminal}.rs`, `P/src/{lib,state,event}.rs`, `P/tests/state.rs`,
  `P/tests/support/fixture.rs`, `A/tests/{contracts,providers_turns}.rs`,
  `R/tests/{coding_workflow,providers_integration,stream_persistence}.rs`.
  Depends on 3.3 and 4.3. BAR-02/03, EVT-02. Inject the receiver from the manager or router;
  each open clones it and uses borrow_and_update before the first draw, then changed.
  Attachment fixes the source and accepts only greater revisions; a closed channel produces
  Unavailable only if the attachment is current. Test concurrent startup,
  A→B and late A, duplicate B, reconnection/different source and retired task close.
  Output::emit treats explicit ConnectionChanged: not conversation, safe public
  JSON if it receives the event. Test additive serde/old session without backfill and sinks
  without a UI cap. Acceptance/verification: `T(P; state)`, `T(A; contracts,providers_turns)`,
  `T(R; coding_workflow,providers_integration,stream_persistence)`, `T(H; providers_flow)`.
  Risk: independent snapshot before subscribing or an old fallback erasing B.

## Phase 5 — Persistent bar in a real inline terminal

Checkpoint updated 2026-10-20 after auditing tests/code. The PTY bar is checked
with a selection in the composer without an overlay and in Provider, plus a 100x30 and 20x8 matrix
for Help, Settings, Provider, Login, ModelPicker and Confirm. Approval is verified at
20x8 with challenge/field; TestBackend 0x0 tests not-armed and a viable draw with a current
identity. The additional case `approval_resize_below_minimum_disarms_until_a_fresh_draw`
tests PTY: captures the challenge, resizes to 1x1, sends the valid code and checks that it is not
approved; after restoring 8x20 the same code is accepted only after a redraw. Drop checks
synchronous restoration of raw mode, ICANON and ECHO; cancellation/normal exit checks
ICANON/ECHO, bracketed-paste disable and absence of alternate screen. Serial PTY case
37/37 before this update; run the serial suite again for consolidated evidence. No scenario
gap remains for the 5.1/5.2 criteria; confirm the new suite result and fmt/diff-check
before marking.

Phase 4 implementation evidence (2026-10-19): safe snapshot with a UUID per
activation, monotonic revision and watch published/subscribed under the router mutex.
StageSelection allows no admission and announces no selection; publish_selection requires the exact
active candidate. Direct Select keeps Unmanaged mode; Legacy seed
distinguished. Publication guards cover select, model, restore/recover and
explicit resolve. Real fault injection before/after StageSelection (Err or
Pending/drop), Err after commit and invalidated candidate cover all those
routes. Revocation/Replace, registration, own cancellation, manager/core teardown
publish absence; an old lease does not erase a new generation. Each open clones the watch,
reads borrow_and_update and serves changed; attachment/source/revision block
retired updates and close. Additive Event outside conversation and an old
session without backfill tested. Real RED and full commands in apply-progress.
Focused suites: core 5, application 55, runtime 40, presentation state 31 and app
providers_flow 6 PASS; examples/check and focused Clippy all-targets PASS. This does not
equal verify/QA, full CI or PR-ready.

- [x] **5.1 Reserve the bar before rendering content** —
  `P/src/{status_bar,overlay,terminal,state}.rs`, unit tests of status_bar/terminal,
  `P/tests/state.rs`. Depends on 4.4. BAR-01/03, EVT-01.
  Safe typed snapshot: activity/usage/connection/warning/diagnostic; no services
  or callbacks under the render lock. Reserve the bottom row first for all
  overlays and the composer; long non-conversational panel, summary visible with Approval,
  lexical widgets in state/detail. Test activity, error and explicit absence;
  a "next turn" selection never implies Connected or remote discovery.
  Acceptance/verification: `L(P)`, `T(P; state)`; plugin renderers outside the lock.
  Risk: an early return hides the bar or a UI error degrades to an invisible log.
  Evidence: `status_bar::tests` covers activity, diagnostic/feedback, absence/no
  false Connected and Unicode clipping; PTY `overlays_keep_status_visible_at_minimum_inline_size`
  covers narrow overlays. status_bar suite 5/5 and serial PTY 37/37.

- [x] **5.2 Ensure clipping, resize and PTY restoration** —
  `P/src/{terminal,status_bar,overlay}.rs`, `P/tests/{pty,input_parser}.rs`,
  `P/tests/support/{fixture,pty}.rs`. Depends on 5.1. BAR-01, OVL-03/04,
  SET-02/05/13. Keep a real bounded Viewport::Inline (20x8: seven inline rows,
  one for the bar), without alternate screen; only conversation goes to insert_before.
  Test each Overlay and no overlay at normal width/20x8, Unicode by cells,
  clipping/zero-size and resize below the minimum: no panic and no positive acceptance until
  space is recovered/redrawn/armed. Keep field/controls, challenge and a full preview
  scrollable with PgUp/PgDn; feedback by OverlayId+epoch.
  Acceptance/verification: `L(P)`, parser and PTY; inspect termios/bracketed paste
  after Ctrl+C/drop/shutdown. Risk: an artificial buffer snapshot does not test the real viewport.
  Evidence: PTY matrix both sizes; Approval 20x8; TestBackend 0x0 not-armed + viable
  draw/identity; resize 1x1 → 8x20 with a valid code rejected during the unviable size
  and accepted after redraw; scrollable preview; composer cancel/exit checks ICANON,
  ECHO, bracketed-paste disable and no alternate screen. PTY Drop checks synchronous
  close and `is_raw_mode_enabled()==false`; the explicit termios assert is in the
  cancel/exit test. Focused suites: serial PTY 37/37, status_bar 5/5, state 32/32.

## Phase 6 — Removal and delivery gates

- [x] **6.1 Remove legacy adapters and slots** — `P/src/{lib,state,terminal,workflow,settings}.rs`,
  `P/tests/{state,workflows,extensions,providers_ui,completion}.rs`,
  `P/tests/support/fixture.rs`, `H/tests/providers_flow.rs` and `docs/huginn/README.md`.
  Depends on 5.2. OVL-01, EVT-01/02. Delete UiState/View slots modal/plugin_dialog/
  workflow/settings, workflow_focus/settings_focus_epoch, committed, generic notice,
  sovereign status/status_error; update the non-authoritative FIFO comment.
  Keep legitimate names of modules/DTOs/CommandResult::Notice; no blind global search
  justifies deleting them. Review with `rg -n` scoped to P/src and H/src:
  `\bnotice\s*\(`, `\bcommitted\b`, `workflow_focus|settings_focus_epoch|status_error`
  and references `(state|view)\.(modal|plugin_dialog|workflow|settings)\b`;
  inspect the full definition of UiState/View and justify every exception.
  Acceptance/verification: zero parallel slots and zero UI in conversation;
  `T(P; state,workflows,extensions,providers_ui,completion)`, `T(H; providers_flow)`.
  Risk: legacy hidden by an alias; manual inspection in addition to the negative search.
  Evidence 2026-10-20: inspection of `UiState` and `View` shows only
  `overlays`/`overlay_identity`, without parallel slots. Methods `modal()`,
  `workflow()`, `plugin_dialog()` and `settings()` are projections of the single
  payload, not sovereign storage. `notice()` in providers.rs returns the legitimate
  `CommandResult::Notice`. There is no `committed`, `workflow_focus`,
  `settings_focus_epoch` or `status_error`; the FIFO comment updated as a
  non-authoritative projection. Suites state 32, workflows 13, extensions 2,
  providers_ui 7, completion 9 and huginn/providers_flow 6: PASS, log
  `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-phase6-1-focused.log`.

- [x] **6.2 Run targeted QA with own evidence** — use and, if a case is missing,
  extend `P/tests/{pty,providers_ui,state}.rs`, `P/tests/support/fixture.rs` and
  `H/tests/{tui,providers_flow}.rs` inside the unit that delivers that coverage.
  Depends on 6.1. Walk the following matrix during streaming, all overlays,
  blocked saves, resize and reconnection; repeat UI commands after more than 2 s using
  a fake clock in integration. PTY verifies the absence of UI in scrollback, not only
  substrings. Secret sentinel absent from screen/captures, scrollback, JSON/logs;
  split paste, fresh challenge, revoke/re-register and restored close.
  Acceptance/verification: PTY and `T(H; tui,providers_flow)` with sanitized evidence
  for a future own qa-report. Risk: real backend unreachable; record the case
  not tested, do not invent success or use the Settings-modal archive as evidence.
  Evidence 2026-10-20: targeted technical acceptance of Apply, not an independent QA phase.
  New PTY with streaming retained by cancellation, six ordinary overlays and a
  secret sentinel checks the real reconstructed scrollback with a positive conversational
  control; it does not compare only transcript substrings. Harness RED:
  the bottom LF overwrote instead of scrolling; GREEN keeps the ejected rows.
  Productive RED: Help did not open during streaming because of the `!busy` guard; GREEN allows
  UI commands and keeps sending another prompt blocked. Integration with fake clock
  repeats Help/Providers/Login/Model/Settings/Logout after 2001 ms without altering
  conversation, tail or cancellation. The earlier matrix covers Approval, resize, split
  paste, revocation/re-registration and restoration; state covers writes with barriers
  and reconnection. Full presentation: 155 PASS (39 PTY, 10 parser, 32 state,
  13 workflows, 8 providers_ui and 53 remaining). App tui 8 + providers_flow 6 PASS.
  Full logs and REQ→test mapping in `apply-validation.md`; without an external
  provider or a real secure store, without borrowed evidence from Settings-modal.

- [x] **6.3 Consolidate validation and deliver the report without publishing** — check
  all runners of the units and the full `just ci-local` with the requirements of
  `docs/development.md` and `dev/README.md`: tooling/dependencies, Docker available,
  browsers and the `just test-e2e-build` image. Do not copy the manual and do not reset data.
  Depends on 6.2. Review the real diff/statistics, exhaustive Event consumers,
  CommandResult and ProviderAction, including examples/fixtures/wrappers, without wildcards
  that hide new variants. `just ci-ci` skips E2E: it does not replace ci-local.
  Acceptance: report to the orchestrator with task/REQ/scenario→test/command/evidence,
  RED/GREEN, gates executed/failed/not executed, risks and real size;
  distinguish COMPLIANT/FAILING/UNTESTED and keep blockers for their own verify/qa.
  Risk: a partial gate presented as full CI. No empty validation commits,
  publication or archive; verify-report and qa-report are missing until their phases.
  Evidence 2026-10-20: seven complete Huginn packages PASS with all-features;
  additional Clippy all-features/all-targets PASS; providers_headless example PASS.
  Final CI without altering runner/parallelism: `just ci-local` exit 0, 9/9, 431 s,
  Rust PASS (39 PTY included), Vitest 24 files/210 tests and E2E 90/90 in three
  browsers. `cargo audit --no-fetch` directly exit 0 without a mask. Full logs
  `opencode/huginn-apply6/ci-local-verified.log` under the approved temp; REQ→test mapping,
  earlier failures, missing `just doctor`, two opt-in ignores,
  environment limits and real size in `apply-validation.md`. Without Verify/QA,
  publication, archive, data reset, state or index modification.

## Full coverage and barrier matrix

The SET identifiers refer to REQ of ui-settings; its delta replaces only
07/08/12. Each association requires evidence at verify; it is not an anticipated verdict.

- **EVT-01 A/B** → 1.1, 1.3, 2.2, 2.3, 2.6, 3.1, 5.1, 6.1/2: same text per category,
  repeated openings/closings after 2 s without a prompt or UI history; visible diagnostic.
- **EVT-02 A/B** → 1.1/3, 4.4: legitimate repetition, streaming and tool error intact,
  FIFO 256/257 without affecting Session.events, persistence/replay or reopening the session.
- **EVT-03 A/B/C** → 1.2/3: fake t=0/1.999/2s, interleaved A/B, injected collision,
  shared fallback; no suppression extends the window or changes surface.
- **EVT-04 A** → 1.2/3: entry/byte/4096 limits, L+1, expiration and finish;
  visible saturation, without truncating the comparison or storing secrets.
- **OVL-01 A** → 2.2, 2.3, 3.1, 6.1: Help/focus/consumed close, final single authority.
- **OVL-02 A/B** → 2.1, 2.2, 2.3: allowlist/metadata and exhaustive matching,
  Confirm without authority, rejection keeps the overlay. contract/workflows and smoke tests.
- **OVL-03 A/B/C** → 2.2, 2.4, 3.2, 5.2: single sender, fresh input,
  cancellation/Drop/channel/shutdown and earlier answer; ordinary ones blocked and never restored.
- **OVL-04 A/B** → 2.2, 2.3–2.5, 3.2, 5.2/6.2: split paste,
  reentrancy including Drop of Pending, lease/epoch, revocation/re-registration, late task,
  sentinel and PTY.
- **OVL-05 A/B** → 2.2, 2.3, 2.6, 4.2/4: callbacks/payloads/secrets preserved
  in each continuation; real login/providers/model/logout; Secure without fallback,
  explicit Ephemeral, validation/OAuth and turn pinning/revocation.
- **BAR-01 A** → 5.1/2, 6.2: all overlays including Approval, inline viewport,
  Unicode, 20x8, resize/zero-size, clipping and conversational-only scrollback.
- **BAR-02 A/B/C** → 4.1–4: snapshot/subscribe barriers and late A/B; exact
  stage/confirm on all routes; faults before/after publishing and Pending/drop/cancel;
  adapter/registration/manager/core teardown, owner/source/revision and reconnection.
- **BAR-03 A/B** → 4.1–4, 5.1, 6.2: local/Legacy/Unmanaged selection without Connected,
  no earlier success promises health; absence/error/approval always observable.
- **SET-07 A/B/C/D** → 3.2/3: reentrant closure, write barrier, single Enter,
  disk full keeps copy, retry and late receipt with another Settings open;
  snapshot saved only on success, joint TOML without live theme.
- **SET-08 A/B** → 3.1/2: Esc during streaming before/after starting the write,
  bytes intact only before Apply, one write and receipt without rollback or reopening.
- **SET-12 A/B/C/D** → 2.2, 2.4, 3.1/2: valid/rejected replacement, Approval before/
  during Apply and success/error receipt, stale lease does not cross to the substitute.
- **SET-01** → 3.1 and completion/plugins: registration, description, whitespace, clean
  editor and zero prompts; **SET-02** → 3.1/5.2: header/body/footer and markers.
- **SET-03/04/05** → 3.1/5.2: tabs/navigation/selection, specific rows and pickers; **SET-06** → 3.1: independent copy and moves without I/O.
- **SET-09** → 3.3: round-trip, external bytes/comments/order; **SET-10** → 3.3:
  defaults without ui, partial and complete; **SET-11** → 3.3: internal unknowns preserved.
- **SET-13** → 3.1/2, 5.2: Esc does not cancel the turn; **SET-14** → 3.1: Add provider without
  mutation/providers/persistence; **SET-15** → 3.3: line-mode excluded; **SET-16** → 3.1: chat catalog with/without model.

## Blockers and required output

No blocking product gaps were identified for planning. Installed tooling is not assumed,
nor green CI or inherited evidence: behavioral failures, races,
leaks, missing coverage of a publication or teardown route and incomplete QA
are real acceptance blockers that must be communicated, not silenced.
The size exception is already authorized; no other delivery question is added.

Orchestrator next step: review Apply evidence and delegate Verify, then
QA; updating state is the orchestrator's task alone. The reports themselves must document all
untested cases and blockers before archive. Settings-modal archived without
verify/qa does not accredit any of those gates.
